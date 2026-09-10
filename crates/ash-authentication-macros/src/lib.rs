use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Error, Ident, ItemMacro, LitInt, LitStr, Result, Token};

struct PasswordStrategyConfig {
    identity_field: String,
    hashed_password_field: String,
    min_password_length: usize,
    require_confirmation: bool,
    register_action_name: String,
    sign_in_action_name: String,
}

impl Default for PasswordStrategyConfig {
    fn default() -> Self {
        Self {
            identity_field: "email".to_string(),
            hashed_password_field: "hashed_password".to_string(),
            min_password_length: 8,
            require_confirmation: true,
            register_action_name: "register_with_password".to_string(),
            sign_in_action_name: "sign_in_with_password".to_string(),
        }
    }
}

struct TokenStrategyConfig {
    token_lifetime_secs: u64,
}

impl Default for TokenStrategyConfig {
    fn default() -> Self {
        Self {
            token_lifetime_secs: 3600,
        }
    }
}

struct ApiKeyStrategyConfig {
    api_key_field: String,
    key_prefix: String,
}

impl Default for ApiKeyStrategyConfig {
    fn default() -> Self {
        Self {
            api_key_field: "api_key_hash".to_string(),
            key_prefix: "ash_".to_string(),
        }
    }
}

struct ConfirmationStrategyConfig {
    confirmed_field: String,
    prevent_unconfirmed_sign_in: bool,
    token_lifetime_secs: u64,
}

impl Default for ConfirmationStrategyConfig {
    fn default() -> Self {
        Self {
            confirmed_field: "confirmed_at".to_string(),
            prevent_unconfirmed_sign_in: true,
            token_lifetime_secs: 86400,
        }
    }
}

#[derive(Default)]
struct AuthenticationBlock {
    password: Option<PasswordStrategyConfig>,
    tokens: Option<TokenStrategyConfig>,
    api_key: Option<ApiKeyStrategyConfig>,
    confirmation: Option<ConfirmationStrategyConfig>,
}

impl Parse for AuthenticationBlock {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut block = AuthenticationBlock::default();

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            if key == "strategy" {
                let strat_kind: Ident = input.parse()?;
                let content;
                syn::braced!(content in input);

                match strat_kind.to_string().as_str() {
                    "password" => {
                        let mut cfg = PasswordStrategyConfig::default();
                        while !content.is_empty() {
                            let item_key: Ident = content.parse()?;
                            if content.peek(Token![:]) {
                                let _: Token![:] = content.parse()?;
                            }
                            match item_key.to_string().as_str() {
                                "identity_field" => {
                                    if content.peek(Ident) {
                                        let id: Ident = content.parse()?;
                                        cfg.identity_field = id.to_string();
                                    } else {
                                        let lit: LitStr = content.parse()?;
                                        cfg.identity_field = lit.value();
                                    }
                                }
                                "hashed_password_field" => {
                                    if content.peek(Ident) {
                                        let id: Ident = content.parse()?;
                                        cfg.hashed_password_field = id.to_string();
                                    } else {
                                        let lit: LitStr = content.parse()?;
                                        cfg.hashed_password_field = lit.value();
                                    }
                                }
                                "min_password_length" => {
                                    let lit: LitInt = content.parse()?;
                                    cfg.min_password_length = lit.base10_parse()?;
                                }
                                "require_confirmation" => {
                                    if content.peek(syn::LitBool) {
                                        let b: syn::LitBool = content.parse()?;
                                        cfg.require_confirmation = b.value();
                                    } else {
                                        let id: Ident = content.parse()?;
                                        cfg.require_confirmation = id == "true";
                                    }
                                }
                                "register_action_name" => {
                                    let id: Ident = content.parse()?;
                                    cfg.register_action_name = id.to_string();
                                }
                                "sign_in_action_name" => {
                                    let id: Ident = content.parse()?;
                                    cfg.sign_in_action_name = id.to_string();
                                }
                                _ => {
                                    // Skip unknown tokens in item
                                    let _ = content.parse::<proc_macro2::TokenTree>();
                                }
                            }
                            if content.peek(Token![;]) {
                                let _: Token![;] = content.parse()?;
                            }
                        }
                        block.password = Some(cfg);
                    }
                    "tokens" => {
                        let mut cfg = TokenStrategyConfig::default();
                        while !content.is_empty() {
                            let item_key: Ident = content.parse()?;
                            if content.peek(Token![:]) {
                                let _: Token![:] = content.parse()?;
                            }
                            if item_key == "token_lifetime_secs" {
                                let lit: LitInt = content.parse()?;
                                cfg.token_lifetime_secs = lit.base10_parse()?;
                            } else {
                                let _ = content.parse::<proc_macro2::TokenTree>();
                            }
                            if content.peek(Token![;]) {
                                let _: Token![;] = content.parse()?;
                            }
                        }
                        block.tokens = Some(cfg);
                    }
                    "api_key" => {
                        let mut cfg = ApiKeyStrategyConfig::default();
                        while !content.is_empty() {
                            let item_key: Ident = content.parse()?;
                            if content.peek(Token![:]) {
                                let _: Token![:] = content.parse()?;
                            }
                            if item_key == "api_key_field" {
                                if content.peek(Ident) {
                                    let id: Ident = content.parse()?;
                                    cfg.api_key_field = id.to_string();
                                } else {
                                    let lit: LitStr = content.parse()?;
                                    cfg.api_key_field = lit.value();
                                }
                            } else if item_key == "key_prefix" {
                                let lit: LitStr = content.parse()?;
                                cfg.key_prefix = lit.value();
                            } else {
                                let _ = content.parse::<proc_macro2::TokenTree>();
                            }
                            if content.peek(Token![;]) {
                                let _: Token![;] = content.parse()?;
                            }
                        }
                        block.api_key = Some(cfg);
                    }
                    "confirmation" => {
                        let mut cfg = ConfirmationStrategyConfig::default();
                        while !content.is_empty() {
                            let item_key: Ident = content.parse()?;
                            if content.peek(Token![:]) {
                                let _: Token![:] = content.parse()?;
                            }
                            if item_key == "confirmed_field" {
                                if content.peek(Ident) {
                                    let id: Ident = content.parse()?;
                                    cfg.confirmed_field = id.to_string();
                                } else {
                                    let lit: LitStr = content.parse()?;
                                    cfg.confirmed_field = lit.value();
                                }
                            } else if item_key == "prevent_unconfirmed_sign_in" {
                                if content.peek(syn::LitBool) {
                                    let b: syn::LitBool = content.parse()?;
                                    cfg.prevent_unconfirmed_sign_in = b.value();
                                } else {
                                    let id: Ident = content.parse()?;
                                    cfg.prevent_unconfirmed_sign_in = id == "true";
                                }
                            } else if item_key == "token_lifetime_secs" {
                                let lit: LitInt = content.parse()?;
                                cfg.token_lifetime_secs = lit.base10_parse()?;
                            } else {
                                let _ = content.parse::<proc_macro2::TokenTree>();
                            }
                            if content.peek(Token![;]) {
                                let _: Token![;] = content.parse()?;
                            }
                        }
                        block.confirmation = Some(cfg);
                    }
                    _ => {
                        // ignore unknown strategy
                    }
                }
            } else {
                let _ = input.parse::<proc_macro2::TokenTree>();
            }
        }

        Ok(block)
    }
}

struct RawSection {
    name: Ident,
    tokens: TokenStream2,
    has_brace: bool,
}

struct ResourceDslInput {
    outer_attrs: Vec<Attribute>,
    resource_ident: Ident,
    sections: Vec<RawSection>,
    auth_block: Option<AuthenticationBlock>,
}

impl Parse for ResourceDslInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let outer_attrs = input.call(Attribute::parse_outer)?;
        let mut resource_ident = None;
        let mut sections = Vec::new();
        let mut auth_block = None;

        while !input.is_empty() {
            let ident: Ident = input.parse()?;

            if ident == "resource" {
                let name: Ident = input.parse()?;
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                resource_ident = Some(name);
                continue;
            }

            if ident == "authentication" {
                let inner;
                syn::braced!(inner in input);
                let parsed_auth: AuthenticationBlock = inner.parse()?;
                auth_block = Some(parsed_auth);
                continue;
            }

            if input.peek(syn::token::Brace) {
                let inner;
                syn::braced!(inner in input);
                let inner_tokens: TokenStream2 = inner.parse()?;
                sections.push(RawSection {
                    name: ident,
                    tokens: inner_tokens,
                    has_brace: true,
                });
            } else {
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
            auth_block,
        })
    }
}

/// Pattern 2: Transformative Macro Decorator (`#[authentication] resource! { ... }`)
#[proc_macro_attribute]
pub fn authentication(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let item_2: TokenStream2 = item.clone().into();

    let (dsl_tokens, outer_attrs) = match syn::parse2::<ItemMacro>(item_2.clone()) {
        Ok(item_macro) => (item_macro.mac.tokens, item_macro.attrs),
        Err(_) => (item_2, Vec::new()),
    };

    let mut dsl = match syn::parse2::<ResourceDslInput>(dsl_tokens) {
        Ok(d) => d,
        Err(e) => return e.to_compile_error().into(),
    };
    if !outer_attrs.is_empty() {
        dsl.outer_attrs.extend(outer_attrs);
    }

    match expand_authentication_transformer(dsl) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand_authentication_transformer(mut dsl: ResourceDslInput) -> Result<TokenStream2> {
    let auth = dsl.auth_block.unwrap_or_else(|| AuthenticationBlock {
        password: Some(PasswordStrategyConfig::default()),
        tokens: Some(TokenStrategyConfig::default()),
        api_key: None,
        confirmation: None,
    });

    let resource_ident = &dsl.resource_ident;

    // 1. Injected fields into `attributes`
    let mut injected_attrs = quote! {};
    if let Some(pass) = &auth.password {
        let field_ident = format_ident!("{}", pass.hashed_password_field);
        injected_attrs = quote! {
            #injected_attrs
            #field_ident: Option<String>,
        };
    }
    if let Some(api) = &auth.api_key {
        let field_ident = format_ident!("{}", api.api_key_field);
        injected_attrs = quote! {
            #injected_attrs
            #field_ident: Option<String>,
        };
    }
    if let Some(conf) = &auth.confirmation {
        let field_ident = format_ident!("{}", conf.confirmed_field);
        injected_attrs = quote! {
            #injected_attrs
            #field_ident: Option<String>,
        };
    }

    if let Some(attr_sec) = dsl.sections.iter_mut().find(|s| s.name == "attributes") {
        let existing = &attr_sec.tokens;
        attr_sec.tokens = quote! {
            #existing
            #injected_attrs
        };
    } else {
        dsl.sections.push(RawSection {
            name: format_ident!("attributes"),
            tokens: quote! {
                id: ::uuid::Uuid [pk],
                #injected_attrs
            },
            has_brace: true,
        });
    }

    // 2. Injected actions into `actions`
    let mut injected_actions = quote! {};
    if let Some(pass) = &auth.password {
        let reg_ident = format_ident!("{}", pass.register_action_name);
        let id_ident = format_ident!("{}", pass.identity_field);
        let hashed_field_str = &pass.hashed_password_field;
        let min_len = pass.min_password_length;

        let conf_arg = if pass.require_confirmation {
            quote! {
                argument password_confirmation: String;
            }
        } else {
            quote! {}
        };

        let conf_str_opt = if pass.require_confirmation {
            quote! { Some("password_confirmation") }
        } else {
            quote! { None }
        };

        injected_actions = quote! {
            #injected_actions
            create #reg_ident {
                accept [#id_ident];
                argument password: String;
                #conf_arg
                change custom(&::ash_authentication::HashPasswordChange::new(
                    "password",
                    #conf_str_opt,
                    #hashed_field_str,
                ).with_min_length(#min_len));
            }
        };
    }

    if let Some(act_sec) = dsl.sections.iter_mut().find(|s| s.name == "actions") {
        let existing = &act_sec.tokens;
        act_sec.tokens = quote! {
            #existing
            #injected_actions
        };
    } else {
        dsl.sections.push(RawSection {
            name: format_ident!("actions"),
            tokens: quote! {
                read read { primary; }
                #injected_actions
            },
            has_brace: true,
        });
    }

    // 3. Inject AuthenticationDef into `extensions`
    let pass_tokens = if let Some(p) = &auth.password {
        let reg = &p.register_action_name;
        let sign = &p.sign_in_action_name;
        let id = &p.identity_field;
        let hash = &p.hashed_password_field;
        let min = p.min_password_length;
        let req_conf = p.require_confirmation;
        quote! {
            Some(::ash_authentication::PasswordStrategyDef {
                register_action_name: #reg,
                sign_in_action_name: #sign,
                identity_field: #id,
                hashed_password_field: #hash,
                min_password_length: #min,
                require_confirmation: #req_conf,
            })
        }
    } else {
        quote! { None }
    };
    let token_tokens = if let Some(t) = &auth.tokens {
        let lt = t.token_lifetime_secs;
        quote! {
            Some(::ash_authentication::TokenStrategyDef {
                token_lifetime_secs: #lt,
                track_revocations: true,
            })
        }
    } else {
        quote! { None }
    };
    let api_tokens = if let Some(a) = &auth.api_key {
        let f = &a.api_key_field;
        let p = &a.key_prefix;
        quote! {
            Some(::ash_authentication::ApiKeyStrategyDef {
                api_key_field: #f,
                key_prefix: #p,
            })
        }
    } else {
        quote! { None }
    };
    let conf_tokens = if let Some(c) = &auth.confirmation {
        let f = &c.confirmed_field;
        let p = c.prevent_unconfirmed_sign_in;
        let lt = c.token_lifetime_secs;
        quote! {
            Some(::ash_authentication::ConfirmationStrategyDef {
                confirmed_field: #f,
                prevent_unconfirmed_sign_in: #p,
                token_lifetime_secs: #lt,
            })
        }
    } else {
        quote! { None }
    };

    let auth_ext_expr = quote! {
        &::ash_authentication::AuthenticationDef {
            password: #pass_tokens,
            tokens: #token_tokens,
            api_key: #api_tokens,
            confirmation: #conf_tokens,
        }
    };

    if let Some(ext_sec) = dsl.sections.iter_mut().find(|s| s.name == "extensions") {
        let existing = &ext_sec.tokens;
        ext_sec.tokens = quote! {
            #existing, #auth_ext_expr
        };
    } else {
        dsl.sections.push(RawSection {
            name: format_ident!("extensions"),
            tokens: quote! {
                [#auth_ext_expr]
            },
            has_brace: false,
        });
    }

    // Reassemble sections into inner DSL
    let mut dsl_body = quote! {
        resource #resource_ident;
    };

    for sec in dsl.sections {
        let sec_name = &sec.name;
        let sec_tokens = &sec.tokens;
        if sec.has_brace {
            dsl_body = quote! {
                #dsl_body
                #sec_name {
                    #sec_tokens
                }
            };
        } else {
            dsl_body = quote! {
                #dsl_body
                #sec_name #sec_tokens;
            };
        }
    }

    let outer_attrs = &dsl.outer_attrs;

    let expanded = quote! {
        #(#outer_attrs)*
        ::ash_core::resource! {
            #dsl_body
        }

        impl #resource_ident {
            /// Initialize an `AuthStrategy` configured for this resource.
            pub fn auth_strategy() -> ::ash_authentication::AuthStrategy<Self> {
                ::ash_authentication::AuthStrategy::new(&<Self as ::ash_core::Resource>::DEF)
            }
        }
    };

    Ok(expanded)
}
