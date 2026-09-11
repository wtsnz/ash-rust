use super::ast::{CodeInterfaceTarget, DomainDefinition};
use crate::ast_helpers::{screaming_snake, snake_case};
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::Result;

fn pluralize(s: &str) -> String {
    if s.ends_with('s')
        || s.ends_with('x')
        || s.ends_with('z')
        || s.ends_with("ch")
        || s.ends_with("sh")
    {
        format!("{s}es")
    } else if s.ends_with('y')
        && !s.ends_with("ay")
        && !s.ends_with("ey")
        && !s.ends_with("oy")
        && !s.ends_with("uy")
    {
        format!("{}ies", &s[..s.len() - 1])
    } else {
        format!("{s}s")
    }
}

pub fn expand_domain(def: DomainDefinition) -> Result<TokenStream> {
    let domain_name = &def.domain_name;
    let outer_attrs = &def.outer_attrs;
    let def_const_name = format_ident!("{}_DEF", screaming_snake(&domain_name.to_string()));

    let resource_idents: Vec<_> = def.resources.iter().map(|r| &r.resource).collect();

    let mut resource_methods = Vec::new();

    for res in &def.resources {
        let res_ident = &res.resource;
        let res_snake = snake_case(&res_ident.to_string());
        let plural_ident = format_ident!("{}", pluralize(&res_snake));
        let getter_ident = format_ident!("get_{}", &res_snake);

        let has_explicit_getter = res.interfaces.iter().any(|ci| ci.fn_name == getter_ident);
        let has_explicit_plural = res.interfaces.iter().any(|ci| ci.fn_name == plural_ident);

        if !has_explicit_plural {
            resource_methods.push(quote! {
                pub fn #plural_ident(&self) -> ::ash_core::Query<'_, #res_ident, D> {
                    ::ash_core::query(&self.ctx)
                }
            });
        }

        if !has_explicit_getter {
            resource_methods.push(quote! {
                pub async fn #getter_ident(&self, id: ::uuid::Uuid) -> ::ash_core::Result<#res_ident> {
                    ::ash_core::get::<#res_ident, D>(&self.ctx, id).await
                }
            });
        }

        // Explicit code interfaces
        for ci in &res.interfaces {
            let fn_name = &ci.fn_name;
            let action_name = &ci.action_name;
            let action_str = action_name.to_string();
            let act_on_name = format_ident!("{}_on", action_name);

            let arg_names: Vec<_> = ci.args.iter().map(|a| &a.name).collect();
            let arg_tys: Vec<_> = ci.args.iter().map(|a| &a.ty).collect();

            if ci.get_by.is_some() {
                resource_methods.push(quote! {
                    pub async fn #fn_name(&self, id: ::uuid::Uuid) -> ::ash_core::Result<#res_ident> {
                        ::ash_core::get::<#res_ident, D>(&self.ctx, id).await
                    }
                });
            } else if action_str == "read" && arg_names.is_empty() {
                resource_methods.push(quote! {
                    pub fn #fn_name(&self) -> ::ash_core::Query<'_, #res_ident, D> {
                        ::ash_core::query(&self.ctx)
                    }
                });
            } else if action_str == "destroy" && arg_names.is_empty() {
                match ci.target {
                    CodeInterfaceTarget::Record => {
                        resource_methods.push(quote! {
                            pub async fn #fn_name(&self, record: &#res_ident) -> ::ash_core::Result<()> {
                                record.#act_on_name(&self.ctx).await
                            }
                        });
                    }
                    _ => {
                        resource_methods.push(quote! {
                            pub async fn #fn_name(&self, id: ::uuid::Uuid) -> ::ash_core::Result<()> {
                                ::ash_core::destroy::<#res_ident, D>(&self.ctx, stringify!(#action_name), id).await
                            }
                        });
                    }
                }
            } else {
                match ci.target {
                    CodeInterfaceTarget::Record => {
                        resource_methods.push(quote! {
                            pub async fn #fn_name(
                                &self,
                                record: &#res_ident,
                                #(#arg_names: impl ::std::convert::Into<#arg_tys>),*
                            ) -> ::ash_core::Result<#res_ident> {
                                record.#act_on_name(&self.ctx)#(.#arg_names(#arg_names.into()))*.await
                            }
                        });
                    }
                    CodeInterfaceTarget::Id => {
                        resource_methods.push(quote! {
                            pub async fn #fn_name(
                                &self,
                                id: ::uuid::Uuid,
                                #(#arg_names: impl ::std::convert::Into<#arg_tys>),*
                            ) -> ::ash_core::Result<#res_ident> {
                                #res_ident::#action_name(&self.ctx, id)#(.#arg_names(#arg_names.into()))*.await
                            }
                        });
                    }
                    CodeInterfaceTarget::Static => {
                        resource_methods.push(quote! {
                            pub async fn #fn_name(
                                &self,
                                #(#arg_names: impl ::std::convert::Into<#arg_tys>),*
                            ) -> ::ash_core::Result<#res_ident> {
                                #res_ident::#action_name(&self.ctx)#(.#arg_names(#arg_names.into()))*.await
                            }
                        });
                    }
                }
            }
        }
    }

    let mut interface_probes = Vec::new();
    for res in &def.resources {
        let res_ident = &res.resource;
        interface_probes.push(quote_spanned! { res_ident.span() =>
            __ash_assert_resource::<#res_ident>();
        });
        for ci in &res.interfaces {
            if ci.get_by.is_some() || ci.action_name == "read" {
                continue;
            }
            let action_name = &ci.action_name;
            let method = quote_spanned! { action_name.span() => #action_name };
            interface_probes.push(match ci.target {
                CodeInterfaceTarget::Static => quote! {
                    if false {
                        let _ = <#res_ident>::#method::<::ash_memory::Memory>;
                    }
                },
                CodeInterfaceTarget::Record | CodeInterfaceTarget::Id => quote! {
                    if false {
                        fn __ash_probe_action(
                            __ctx: &::ash_core::Context<::ash_memory::Memory>,
                            __id: ::uuid::Uuid,
                        ) {
                            let _ = <#res_ident>::#method(__ctx, __id);
                        }
                    }
                },
            });
        }
    }
    let probe_tokens = quote! {
        #[doc(hidden)]
        const _: () = {
            #[allow(
                dead_code,
                unused_variables,
                unused_imports,
                unreachable_code,
                clippy::all,
                clippy::pedantic
            )]
            fn __ash_domain_ide_probes() {
                #[allow(dead_code)]
                fn __ash_assert_resource<T: ::ash_core::Resource>() {}
                #(#interface_probes)*
            }
        };
    };

    Ok(quote! {
        #(#outer_attrs)*
        pub struct #domain_name<D = ()> {
            pub ctx: ::ash_core::Context<D>,
        }

        impl<D> ::std::clone::Clone for #domain_name<D> {
            fn clone(&self) -> Self {
                Self {
                    ctx: self.ctx.clone(),
                }
            }
        }

        impl<D: ::std::fmt::Debug> ::std::fmt::Debug for #domain_name<D> {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.debug_struct(stringify!(#domain_name))
                    .field("ctx", &self.ctx)
                    .finish()
            }
        }

        impl<D> #domain_name<D> {
            pub const DEF: ::ash_core::DomainDef = ::ash_core::DomainDef {
                name: stringify!(#domain_name),
                resources: &[#(&<#resource_idents as ::ash_core::Resource>::DEF),*],
            };

            pub fn def(&self) -> &'static ::ash_core::DomainDef {
                &Self::DEF
            }

            pub fn new(data_layer: D) -> Self {
                Self {
                    ctx: ::ash_core::Context::new(data_layer),
                }
            }

            pub fn from_context(ctx: ::ash_core::Context<D>) -> Self {
                Self { ctx }
            }

            pub fn from_arc(data: ::std::sync::Arc<D>) -> Self {
                Self {
                    ctx: ::ash_core::Context::from_arc(data),
                }
            }

            pub fn context(&self) -> &::ash_core::Context<D> {
                &self.ctx
            }

            pub fn into_context(self) -> ::ash_core::Context<D> {
                self.ctx
            }

            pub fn with_actor(&self, actor: ::ash_core::Actor) -> Self {
                Self {
                    ctx: self.ctx.with_actor(actor),
                }
            }

            pub fn as_actor(&self, actor: ::ash_core::Actor) -> Self {
                self.with_actor(actor)
            }

            pub fn without_actor(&self) -> Self {
                Self {
                    ctx: self.ctx.without_actor(),
                }
            }

            pub fn actor(&self) -> ::std::option::Option<&::ash_core::Actor> {
                self.ctx.actor.as_ref()
            }
        }

        impl<D> ::std::ops::Deref for #domain_name<D> {
            type Target = ::ash_core::Context<D>;
            fn deref(&self) -> &Self::Target {
                &self.ctx
            }
        }

        impl<D> ::std::convert::AsRef<::ash_core::Context<D>> for #domain_name<D> {
            fn as_ref(&self) -> &::ash_core::Context<D> {
                &self.ctx
            }
        }

        impl<D: 'static> ::ash_core::Domain for #domain_name<D> {
            const DEF: ::ash_core::DomainDef = Self::DEF;
        }

        pub const #def_const_name: ::ash_core::DomainDef = #domain_name::<()>::DEF;

        impl<D: ::ash_core::DataLayer> #domain_name<D> {
            pub fn query<R: ::ash_core::Resource>(&self) -> ::ash_core::Query<'_, R, D> {
                ::ash_core::query(&self.ctx)
            }

            pub async fn get<R: ::ash_core::Resource>(&self, id: ::uuid::Uuid) -> ::ash_core::Result<R> {
                ::ash_core::get::<R, D>(&self.ctx, id).await
            }

            pub async fn destroy<R: ::ash_core::Resource>(&self, action: &str, id: ::uuid::Uuid) -> ::ash_core::Result<()> {
                ::ash_core::destroy::<R, D>(&self.ctx, action, id).await
            }

            #(#resource_methods)*
        }

        #probe_tokens

        impl<D: ::ash_core::SchemaSupport> #domain_name<D> {
            pub async fn install(&self) -> ::ash_core::Result<()> {
                self.ctx.data.install_resources(Self::DEF.resources).await
            }
        }

        impl<D: ::ash_core::TransactionSupport + 'static> #domain_name<D> {
            pub async fn transaction<F, Fut, T>(&self, f: F) -> ::ash_core::Result<T>
            where
                F: FnOnce(Self) -> Fut + Send,
                Fut: ::std::future::Future<Output = ::ash_core::Result<T>> + Send,
                T: Send,
            {
                self.ctx
                    .transaction(|tx_ctx| async move {
                        f(Self::from_context(tx_ctx)).await
                    })
                    .await
            }

            pub async fn run_multi(
                &self,
                multi: ::ash_core::Multi<D>,
            ) -> ::ash_core::Result<::ash_core::MultiResult> {
                self.ctx.run_multi(multi).await
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::ast::DomainDefinition;
    use quote::quote;

    #[test]
    fn test_domain_emits_action_interface_probes() {
        let def = match syn::parse2::<DomainDefinition>(quote! {
            Helpdesk {
                resources {
                    Ticket {
                        define open_ticket action: open args: [subject: String];
                        define close_ticket action: close on: record;
                        define list_tickets action: read;
                        define get_ticket action: read get_by: id;
                    };
                }
            }
        }) {
            Ok(d) => d,
            Err(e) => panic!("parse failed: {e}"),
        };
        let out = expand_domain(def).expect("expand").to_string();
        assert!(out.contains("Ticket"), "missing resource: {out}");
        assert!(out.contains("open"), "missing open probe: {out}");
        assert!(out.contains("close"), "missing close probe: {out}");
        assert!(
            out.contains("__ash_domain_ide_probes"),
            "missing domain probe: {out}"
        );
        assert!(
            out.contains("__ash_assert_resource"),
            "missing resource probe: {out}"
        );
    }
}
