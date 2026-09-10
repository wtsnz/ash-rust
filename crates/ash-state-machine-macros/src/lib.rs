use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Error, Ident, ItemMacro, LitStr, Result, Token, bracketed};

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

struct ActionBlock {
    kind: Ident,
    name: Ident,
    body: TokenStream2,
}

impl Parse for ActionBlock {
    fn parse(input: ParseStream) -> Result<Self> {
        let kind: Ident = input.parse()?;
        let name: Ident = input.parse()?;
        let content;
        syn::braced!(content in input);
        let body: TokenStream2 = content.parse()?;
        Ok(Self { kind, name, body })
    }
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
                        format!("unknown state_machine directive `{other}`, expected `state_attribute`, `initial`, or `transition`"),
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

/// Generic section of resource DSL
struct RawSection {
    name: Ident,
    tokens: TokenStream2,
    has_brace: bool,
}

struct ResourceDslInput {
    outer_attrs: Vec<syn::Attribute>,
    resource_ident: Ident,
    sections: Vec<RawSection>,
    state_machine: Option<StateMachineBlock>,
}

impl Parse for ResourceDslInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let mut resource_ident = None;
        let mut sections = Vec::new();
        let mut state_machine = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;
            if ident == "resource" {
                let r: Ident = input.parse()?;
                resource_ident = Some(r);
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if ident == "state_machine" {
                let content;
                syn::braced!(content in input);
                let sm: StateMachineBlock = content.parse()?;
                state_machine = Some(sm);
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if input.peek(syn::token::Brace) {
                let content;
                syn::braced!(content in input);
                let inner: TokenStream2 = content.parse()?;
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                sections.push(RawSection {
                    name: ident,
                    tokens: inner,
                    has_brace: true,
                });
            } else {
                // e.g. table "orders"; or optimistic_lock :version;
                let mut tok_vec = Vec::new();
                while !input.is_empty() && !input.peek(Token![;]) {
                    tok_vec.push(input.parse::<proc_macro2::TokenTree>()?);
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                let tokens = tok_vec.into_iter().collect();
                sections.push(RawSection {
                    name: ident,
                    tokens,
                    has_brace: false,
                });
            }
        }

        let resource_ident = resource_ident.ok_or_else(|| {
            Error::new(proc_macro2::Span::call_site(), "missing `resource <Name>;`")
        })?;

        Ok(Self {
            outer_attrs,
            resource_ident,
            sections,
            state_machine,
        })
    }
}

/// Pattern 2: Transformative Macro Decorator (`#[state_machine] resource! { ... }`)
#[proc_macro_attribute]
pub fn state_machine(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_2: TokenStream2 = item.clone().into();

    // Check if item is wrapped in `resource! { ... }` or raw DSL
    let parsed_result: Result<ResourceDslInput> = syn::parse2::<ItemMacro>(item_2.clone())
        .and_then(|item_macro| syn::parse2::<ResourceDslInput>(item_macro.mac.tokens))
        .or_else(|_| syn::parse2::<ResourceDslInput>(item_2));

    let dsl = match parsed_result {
        Ok(d) => d,
        Err(e) => return e.to_compile_error().into(),
    };

    match expand_state_machine_transformer(dsl) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_state_machine_transformer(mut dsl: ResourceDslInput) -> Result<TokenStream2> {
    let sm = match dsl.state_machine {
        Some(sm) => sm,
        None => {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "#[state_machine] requires a `state_machine { ... }` block inside the resource",
            ));
        }
    };

    let resource_ident = &dsl.resource_ident;
    let state_attr_str = &sm.state_attribute;
    let state_attr_ident = format_ident!("{}", sm.state_attribute);
    let initial_str = &sm.initial;

    // 1. Transform `attributes`: ensure state_attribute exists
    let attr_section_opt = dsl.sections.iter_mut().find(|s| s.name == "attributes");
    if let Some(attr_sec) = attr_section_opt {
        let attr_tokens_str = attr_sec.tokens.to_string();
        if !attr_tokens_str.contains(state_attr_str.as_str()) {
            let existing_tokens = &attr_sec.tokens;
            attr_sec.tokens = quote! {
                #existing_tokens
                #state_attr_ident: String,
            };
        }
    } else {
        dsl.sections.push(RawSection {
            name: format_ident!("attributes"),
            tokens: quote! {
                id: ::uuid::Uuid [pk],
                #state_attr_ident: String,
            },
            has_brace: true,
        });
    }

    // 2. Transform `actions`:
    //    - In primary create action: inject default state change
    //    - For each transition: inject transition validation + change
    let actions_section_opt = dsl.sections.iter().find(|s| s.name == "actions");
    let mut actions: Vec<ActionBlock> = match actions_section_opt {
        Some(act_sec) => {
            syn::parse::Parser::parse2(
                |input: syn::parse::ParseStream| {
                    let mut list = Vec::new();
                    while !input.is_empty() {
                        list.push(input.parse::<ActionBlock>()?);
                    }
                    Ok(list)
                },
                act_sec.tokens.clone(),
            )
            .unwrap_or_default()
        }
        None => Vec::new(),
    };

    let default_state_change = quote! {
        change custom(&::ash_state_machine::DefaultStateChange::new(
            #state_attr_str,
            #initial_str,
        ));
    };

    let mut had_create = false;
    for act in &mut actions {
        if act.kind == "create" {
            had_create = true;
            let existing_body = &act.body;
            act.body = quote! {
                #existing_body
                #default_state_change
            };
        }
    }
    if !had_create {
        actions.push(ActionBlock {
            kind: format_ident!("create"),
            name: format_ident!("create"),
            body: quote! {
                primary;
                #default_state_change
            },
        });
    }

    for t in &sm.transitions {
        let t_from = &t.from;
        let t_to = &t.to;
        let t_action_str = &t.action;

        let val_tokens = quote! {
            validate custom(&::ash_state_machine::TransitionValidation::new(
                #state_attr_str,
                &[#(#t_from),*],
                #t_to,
                #t_action_str,
            ));
        };
        let change_tokens = quote! {
            change custom(&::ash_state_machine::TransitionChange::new(
                #state_attr_str,
                #t_to,
            ));
        };

        if let Some(existing_act) = actions
            .iter_mut()
            .find(|a| a.kind == "update" && a.name == t.action)
        {
            let existing_body = &existing_act.body;
            existing_act.body = quote! {
                #existing_body
                #val_tokens
                #change_tokens
            };
        } else {
            actions.push(ActionBlock {
                kind: format_ident!("update"),
                name: format_ident!("{}", t.action),
                body: quote! {
                    #val_tokens
                    #change_tokens
                },
            });
        }
    }

    let reconstructed_actions = actions.into_iter().map(|a| {
        let k = a.kind;
        let n = a.name;
        let b = a.body;
        quote! {
            #k #n {
                #b
            }
        }
    });

    if let Some(act_sec) = dsl.sections.iter_mut().find(|s| s.name == "actions") {
        act_sec.tokens = quote! {
            #(#reconstructed_actions)*
        };
    } else {
        dsl.sections.push(RawSection {
            name: format_ident!("actions"),
            tokens: quote! {
                #(#reconstructed_actions)*
            },
            has_brace: true,
        });
    }

    // 3. Transform `extensions`: inject StateMachineDef constant into resource definition extensions
    let sm_def_tokens = {
        let transitions_tokens = sm.transitions.iter().map(|t| {
            let a_str = &t.action;
            let from_lits = &t.from;
            let to_str = &t.to;
            quote! {
                ::ash_state_machine::TransitionDef::new(#a_str, &[#(#from_lits),*], #to_str)
            }
        });

        quote! {
            &::ash_state_machine::StateMachineDef::new(
                #state_attr_str,
                &[#initial_str],
                Some(#initial_str),
                &[#(#transitions_tokens),*],
            )
        }
    };

    let ext_section_opt = dsl.sections.iter_mut().find(|s| s.name == "extensions");
    if let Some(ext_sec) = ext_section_opt {
        let existing_ext = &ext_sec.tokens;
        ext_sec.tokens = quote! {
            #existing_ext
            #sm_def_tokens
        };
    } else {
        dsl.sections.push(RawSection {
            name: format_ident!("extensions"),
            tokens: quote! {
                #sm_def_tokens
            },
            has_brace: true,
        });
    }

    // 4. Reconstruct resource! { ... } tokens
    let mut reconstructed_sections = Vec::new();
    for sec in &dsl.sections {
        let sec_name = &sec.name;
        let sec_tokens = &sec.tokens;
        if sec.has_brace {
            reconstructed_sections.push(quote! {
                #sec_name {
                    #sec_tokens
                }
            });
        } else {
            reconstructed_sections.push(quote! {
                #sec_name #sec_tokens;
            });
        }
    }

    let outer_attrs = &dsl.outer_attrs;
    let resource_stmt = quote! { resource #resource_ident; };

    // 5. Generate HasStateMachine implementation and can_<action> helper methods
    let transitions_tokens = sm.transitions.iter().map(|t| {
        let a_str = &t.action;
        let from_lits = &t.from;
        let to_str = &t.to;
        quote! {
            ::ash_state_machine::TransitionDef::new(#a_str, &[#(#from_lits),*], #to_str)
        }
    });

    let can_helper_methods = sm.transitions.iter().map(|t| {
        let can_method = format_ident!("can_{}", t.action);
        let a_str = &t.action;
        quote! {
            pub fn #can_method(&self) -> bool {
                ::ash_state_machine::HasStateMachine::can_transition(self, #a_str)
            }
        }
    });

    Ok(quote! {
        #(#outer_attrs)*
        ::ash_core::resource! {
            #resource_stmt
            #(#reconstructed_sections)*
        }

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
