use ash_macro_support::{Action, ResourceTokens};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Error, Ident, LitStr, Result, Token, bracketed};

struct StateMachineBlock {
    state_attribute: String,
    initial: String,
    transitions: Vec<TransitionBlock>,
}

struct TransitionBlock {
    action: String,
    from: Vec<String>,
    to: String,
}

impl Parse for StateMachineBlock {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut state_attribute = "state".to_string();
        let mut initial = String::new();
        let mut transitions = Vec::new();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "state_attribute" => {
                    if input.peek(Token![:]) {
                        let _: Token![:] = input.parse()?;
                    }
                    let attr_ident: Ident = input.parse()?;
                    state_attribute = attr_ident.to_string();
                    if input.peek(Token![;]) {
                        let _: Token![;] = input.parse()?;
                    }
                }
                "initial" => {
                    if input.peek(Token![:]) {
                        let _: Token![:] = input.parse()?;
                    }
                    let lit: LitStr = input.parse()?;
                    initial = lit.value();
                    if input.peek(Token![;]) {
                        let _: Token![;] = input.parse()?;
                    }
                }
                "transition" => {
                    let action: Ident = input.parse()?;
                    let _: Token![,] = input.parse()?;

                    // optional "from:"
                    if input.peek(Ident) {
                        let from_ident: Ident = input.parse()?;
                        if from_ident != "from" {
                            return Err(Error::new_spanned(from_ident, "expected `from`"));
                        }
                        let _: Token![:] = input.parse()?;
                    }

                    let from_content;
                    bracketed!(from_content in input);
                    let mut from = Vec::new();
                    while !from_content.is_empty() {
                        let from_lit: LitStr = from_content.parse()?;
                        from.push(from_lit.value());
                        if from_content.peek(Token![,]) {
                            let _: Token![,] = from_content.parse()?;
                        }
                    }

                    let _: Token![,] = input.parse()?;

                    // optional "to:"
                    if input.peek(Ident) {
                        let to_ident: Ident = input.parse()?;
                        if to_ident != "to" {
                            return Err(Error::new_spanned(to_ident, "expected `to`"));
                        }
                        let _: Token![:] = input.parse()?;
                    }

                    let to_lit: LitStr = input.parse()?;
                    let to = to_lit.value();

                    if input.peek(Token![;]) {
                        let _: Token![;] = input.parse()?;
                    }

                    transitions.push(TransitionBlock {
                        action: action.to_string(),
                        from,
                        to,
                    });
                }
                other => {
                    return Err(Error::new_spanned(
                        key,
                        format!(
                            "unknown state_machine directive `{other}`, expected `state_attribute`, `initial`, or `transition`"
                        ),
                    ));
                }
            }
        }

        if initial.is_empty() {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "`initial` state is required in state_machine definition",
            ));
        }

        Ok(Self {
            state_attribute,
            initial,
            transitions,
        })
    }
}

/// Pattern 2: Transformative Macro Decorator (`#[state_machine] resource! { ... }`)
#[proc_macro_attribute]
pub fn state_machine(_attr: TokenStream, item: TokenStream) -> TokenStream {
    match ResourceTokens::from_item(item.into()).and_then(expand_state_machine_transformer) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_state_machine_transformer(mut resource: ResourceTokens) -> Result<TokenStream2> {
    let sm = match resource.take_section("state_machine") {
        Some(section) => syn::parse2::<StateMachineBlock>(section.tokens)?,
        None => {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "#[state_machine] requires a `state_machine { ... }` block inside the resource",
            ));
        }
    };

    let resource_ident = resource.name.clone();
    let state_attr_str = &sm.state_attribute;
    let state_attr_ident = format_ident!("{}", sm.state_attribute);
    let initial_str = &sm.initial;

    // 1. Transform `attributes`: ensure state_attribute exists
    if resource.section("attributes").is_none() {
        resource.append_to_section("attributes", quote! { id: ::uuid::Uuid [pk]; });
    }
    if !resource.declares_attribute(&state_attr_ident) {
        resource.append_to_section("attributes", quote! { #state_attr_ident: String; });
    }

    // 2. Transform `actions`:
    //    - In primary create action: inject default state change
    //    - For each transition: inject transition validation + change
    let mut actions = resource.actions()?;

    let default_state_change = quote! {
        change custom(&::ash_state_machine::DefaultStateChange::new(
            #state_attr_str,
            #initial_str,
        ));
    };

    let mut had_create = false;
    for act in &mut actions {
        if act.is("create") {
            had_create = true;
            act.append(default_state_change.clone());
        }
    }
    if !had_create {
        actions.push(Action::new(
            "create",
            format_ident!("create"),
            quote! {
                primary;
                #default_state_change
            },
        ));
    }

    for t in &sm.transitions {
        let t_from = &t.from;
        let t_to = &t.to;
        let t_action_str = &t.action;

        let transition_tokens = quote! {
            validate custom(&::ash_state_machine::TransitionValidation::new(
                #state_attr_str,
                &[#(#t_from),*],
                #t_to,
                #t_action_str,
            ));
            change custom(&::ash_state_machine::TransitionChange::new(
                #state_attr_str,
                #t_to,
            ));
        };

        match actions
            .iter_mut()
            .find(|a| a.is("update") && a.name == t.action)
        {
            Some(existing) => existing.append(transition_tokens),
            None => actions.push(Action::new(
                "update",
                format_ident!("{}", t.action),
                transition_tokens,
            )),
        }
    }
    resource.set_actions(&actions);

    // 3. Transform `extensions`: inject StateMachineDef constant into resource definition extensions
    let transitions_tokens: Vec<TokenStream2> = sm
        .transitions
        .iter()
        .map(|t| {
            let a_str = &t.action;
            let from_lits = &t.from;
            let to_str = &t.to;
            quote! {
                ::ash_state_machine::TransitionDef::new(#a_str, &[#(#from_lits),*], #to_str)
            }
        })
        .collect();
    resource.add_extension(quote! {
        &::ash_state_machine::StateMachineDef::new(
            #state_attr_str,
            &[#initial_str],
            Some(#initial_str),
            &[#(#transitions_tokens),*],
        )
    })?;

    // 4. Generate HasStateMachine implementation and can_<action> helper methods
    let can_helper_methods = sm.transitions.iter().map(|t| {
        let can_method = format_ident!("can_{}", t.action);
        let a_str = &t.action;
        quote! {
            pub fn #can_method(&self) -> bool {
                ::ash_state_machine::HasStateMachine::can_transition(self, #a_str)
            }
        }
    });

    let resource_macro = resource.to_resource_macro();
    Ok(quote! {
        #resource_macro

        impl ::ash_state_machine::HasStateMachine for #resource_ident {
            const STATE_MACHINE: ::ash_state_machine::StateMachineDef = ::ash_state_machine::StateMachineDef::new(
                #state_attr_str,
                &[#initial_str],
                Some(#initial_str),
                &[#(#transitions_tokens),*],
            );

            fn current_state(&self) -> &str {
                &self.#state_attr_ident
            }
        }

        impl #resource_ident {
            pub fn current_state(&self) -> &str {
                &self.#state_attr_ident
            }

            pub fn possible_next_states(&self) -> ::std::vec::Vec<&'static str> {
                ::ash_state_machine::HasStateMachine::possible_next_states(self)
            }

            #(#can_helper_methods)*
        }
    })
}
