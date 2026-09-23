use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::ast_helpers::{option_inner, pascal_case};
use crate::define::ast::{ActionKind, ActionSpec, ChangeSpec, ResourceDefinition};

fn docs_for_input_name<'a>(
    def: &'a ResourceDefinition,
    act: &'a ActionSpec,
    name: &syn::Ident,
) -> &'a [syn::Attribute] {
    if let Some(attr) = def.attributes.iter().find(|a| a.ident == *name) {
        return &attr.outer_attrs;
    }
    if let Some(arg) = act.arguments.iter().find(|a| a.name == *name) {
        return &arg.outer_attrs;
    }
    &[]
}

fn arg_docs_for_name<'a>(act: &'a ActionSpec, name: &syn::Ident) -> &'a [syn::Attribute] {
    act.arguments
        .iter()
        .find(|a| a.name == *name)
        .map(|a| a.outer_attrs.as_slice())
        .unwrap_or(&[])
}

fn tuple_ty(parts: &[TokenStream]) -> TokenStream {
    match parts.len() {
        0 => quote! { () },
        1 => {
            let p = &parts[0];
            quote! { (#p,) }
        }
        _ => quote! { (#(#parts),*) },
    }
}

fn is_required_accept(
    def: &ResourceDefinition,
    act: &ActionSpec,
    name: &syn::Ident,
    ty: &syn::Type,
) -> bool {
    if option_inner(ty).is_some() {
        return false;
    }
    if matches!(act.kind, ActionKind::Update | ActionKind::Destroy) {
        return false;
    }
    if let Some(attr) = def.attributes.iter().find(|a| a.ident == *name)
        && (attr.default.is_some() || attr.default_fn.is_some() || attr.pk || attr.generated) {
            return false;
        }
    if act.changes.iter().any(|chg| match chg {
        ChangeSpec::Set { field, .. }
        | ChangeSpec::SetNew { field, .. }
        | ChangeSpec::SetFromArg { field, .. }
        | ChangeSpec::RelateActor { field } => field == name,
        _ => false,
    }) {
        return false;
    }
    true
}

fn required_input_names<'a>(
    def: &'a ResourceDefinition,
    act: &'a ActionSpec,
) -> Vec<&'a syn::Ident> {
    let mut names = Vec::new();
    for acc in &act.accept {
        if is_required_accept(def, act, &acc.name, &acc.ty) {
            names.push(&acc.name);
        }
    }
    for arg in &act.arguments {
        if option_inner(&arg.ty).is_none() {
            names.push(&arg.name);
        }
    }
    names
}

fn typestate_types(n: usize) -> (TokenStream, TokenStream, TokenStream) {
    if n == 0 {
        return (quote! {}, quote! {}, quote! {});
    }
    let unsets = vec![quote! { ::ash_core::InputUnset }; n];
    let sets = vec![quote! { ::ash_core::InputSet }; n];
    let unset_ty = tuple_ty(&unsets);
    let ready_ty = tuple_ty(&sets);
    let struct_params = quote! { , S = #unset_ty };
    (struct_params, unset_ty, ready_ty)
}

/// Per-required-field traits so rustc names the missing setter instead of `InputUnset`.
fn typestate_ready_impls(
    builder_name: &syn::Ident,
    resource: &syn::Ident,
    act_pascal: &str,
    required: &[&syn::Ident],
) -> (TokenStream, TokenStream, TokenStream, TokenStream) {
    if required.is_empty() {
        let impl_ready = quote! { impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> };
        let into_future = quote! {
            impl<'a, D: ::ash_core::DataLayer> ::std::future::IntoFuture for #builder_name<'a, D>
        };
        let into_changeset = quote! {
            impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoChangeset<#resource> for #builder_name<'a, D>
        };
        return (quote! {}, impl_ready, into_future, into_changeset);
    }

    let ts: Vec<syn::Ident> = (0..required.len())
        .map(|i| format_ident!("TS{i}"))
        .collect();
    let mut traits = Vec::new();
    let mut trait_defs = Vec::new();
    for name in required {
        let trait_name = format_ident!("__Ash{resource}{act_pascal}Need_{name}");
        let msg = format!("missing `.{name}(...)` on this action builder");
        let label = format!("required input `{name}` is not set");
        trait_defs.push(quote! {
            #[diagnostic::on_unimplemented(
                message = #msg,
                label = #label,
                note = "set it before `.await` or `.call()`"
            )]
            pub trait #trait_name {}
            impl #trait_name for ::ash_core::InputSet {}
        });
        traits.push(trait_name);
    }
    let ts_tys: Vec<TokenStream> = ts.iter().map(|t| quote! { #t }).collect();
    let tuple = tuple_ty(&ts_tys);
    let impl_ready = quote! {
        impl<'a, D: ::ash_core::DataLayer, #(#ts),*> #builder_name<'a, D, #tuple>
        where
            #(#ts: #traits + ::std::marker::Send + 'static,)*
    };
    let into_future = quote! {
        impl<'a, D: ::ash_core::DataLayer, #(#ts),*> ::std::future::IntoFuture for #builder_name<'a, D, #tuple>
        where
            #(#ts: #traits + ::std::marker::Send + 'static,)*
    };
    let into_changeset = quote! {
        impl<'a, D: ::ash_core::DataLayer, #(#ts),*> ::ash_core::IntoChangeset<#resource> for #builder_name<'a, D, #tuple>
        where
            #(#ts: #traits + ::std::marker::Send + 'static,)*
    };
    (
        quote! { #(#trait_defs)* },
        impl_ready,
        into_future,
        into_changeset,
    )
}

fn into_fieldmap_ref_impl(builder_name: &syn::Ident, has_state: bool) -> TokenStream {
    if has_state {
        quote! {
            impl<'a, D: ::ash_core::DataLayer, S> ::ash_core::IntoFieldMap for &'a #builder_name<'a, D, S> {
                fn into_field_map(self) -> ::ash_core::FieldMap {
                    self.into_fields()
                }
            }
        }
    } else {
        quote! {
            impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for &'a #builder_name<'a, D> {
                fn into_field_map(self) -> ::ash_core::FieldMap {
                    self.into_fields()
                }
            }
        }
    }
}

fn required_setter_impls(
    builder_name: &syn::Ident,
    required: &[(syn::Ident, syn::Type, bool, TokenStream)],
    extra_moves: &[TokenStream],
    all_input_names: &[syn::Ident],
) -> Vec<TokenStream> {
    let n = required.len();
    if n == 0 {
        return Vec::new();
    }
    let type_params: Vec<syn::Ident> = (0..n).map(|i| format_ident!("TS{i}")).collect();
    let mut impls = Vec::new();
    for (i, (name, ty, is_option, docs)) in required.iter().enumerate() {
        let mut from_parts = Vec::new();
        let mut to_parts = Vec::new();
        let mut generics = Vec::new();
        for (j, tp) in type_params.iter().enumerate() {
            if j == i {
                from_parts.push(quote! { ::ash_core::InputUnset });
                to_parts.push(quote! { ::ash_core::InputSet });
            } else {
                from_parts.push(quote! { #tp });
                to_parts.push(quote! { #tp });
                generics.push(tp.clone());
            }
        }
        let from_ty = tuple_ty(&from_parts);
        let to_ty = tuple_ty(&to_parts);
        let generic_list = if generics.is_empty() {
            quote! {}
        } else {
            quote! { , #(#generics),* }
        };
        let input_moves: Vec<TokenStream> = all_input_names
            .iter()
            .map(|fname| {
                if fname == name {
                    quote! { #name: ::std::option::Option::Some(value) }
                } else {
                    quote! { #fname: self.#fname }
                }
            })
            .collect();
        let setter_sig = if *is_option {
            let inner = option_inner(ty).unwrap_or(ty);
            quote! {
                #docs
                pub fn #name(self, value: impl ::ash_core::IntoOption<#inner>) -> #builder_name<'a, D, #to_ty> {
                    let value = value.into_option();
                    #builder_name {
                        #(#extra_moves,)*
                        #(#input_moves,)*
                        _state: ::std::marker::PhantomData,
                    }
                }
            }
        } else {
            quote! {
                #docs
                pub fn #name(self, value: impl ::std::convert::Into<#ty>) -> #builder_name<'a, D, #to_ty> {
                    let value = value.into();
                    #builder_name {
                        #(#extra_moves,)*
                        #(#input_moves,)*
                        _state: ::std::marker::PhantomData,
                    }
                }
            }
        };
        impls.push(quote! {
            impl<'a, D: ::ash_core::DataLayer #generic_list> #builder_name<'a, D, #from_ty> {
                #setter_sig
            }
        });
    }
    impls
}

pub struct ActionCodegen {
    pub builders: Vec<TokenStream>,
    pub resource_methods: Vec<TokenStream>,
    pub trait_block: TokenStream,
}

pub fn expand_action_builders(def: &ResourceDefinition, has_primary_read: bool) -> ActionCodegen {
    let resource = &def.resource;
    let actions = &def.actions;
    let mut builders = Vec::new();
    let mut resource_methods = Vec::new();
    let mut trait_methods = Vec::new();
    let mut trait_impls = Vec::new();

    if has_primary_read || actions.iter().any(|a| a.kind == ActionKind::Read) {
        resource_methods.push(quote! {
            pub fn query<D: ::ash_core::DataLayer>(ctx: &::ash_core::Context<D>) -> ::ash_core::Query<'_, Self, D> {
                ::ash_core::query(ctx)
            }

            pub async fn get<D: ::ash_core::DataLayer>(ctx: &::ash_core::Context<D>, id: ::uuid::Uuid) -> ::ash_core::Result<Self> {
                ::ash_core::get(ctx, id).await
            }
        });
    }

    if let Some(create_act) = actions
        .iter()
        .find(|a| a.kind == ActionKind::Create && a.primary)
        .or_else(|| actions.iter().find(|a| a.kind == ActionKind::Create))
    {
        let act_name_str = create_act.name.to_string();
        resource_methods.push(quote! {
            pub async fn bulk_create<D: ::ash_core::DataLayer, I, F>(
                ctx: &::ash_core::Context<D>,
                inputs: I,
            ) -> ::ash_core::Result<::ash_core::BulkResult<Self>>
            where
                I: ::std::iter::IntoIterator<Item = F>,
                F: ::ash_core::IntoFieldMap,
            {
                ::ash_core::bulk_create(ctx, #act_name_str, inputs, ::ash_core::BulkCreateOptions::default()).await
            }

            pub async fn bulk_create_with_opts<D: ::ash_core::DataLayer, I, F>(
                ctx: &::ash_core::Context<D>,
                action: &str,
                inputs: I,
                opts: ::ash_core::BulkCreateOptions,
            ) -> ::ash_core::Result<::ash_core::BulkResult<Self>>
            where
                I: ::std::iter::IntoIterator<Item = F>,
                F: ::ash_core::IntoFieldMap,
            {
                ::ash_core::bulk_create(ctx, action, inputs, opts).await
            }
        });
    }

    if let Some(destroy_act) = actions
        .iter()
        .find(|a| a.kind == ActionKind::Destroy && a.primary)
        .or_else(|| actions.iter().find(|a| a.kind == ActionKind::Destroy))
    {
        let act_name_str = destroy_act.name.to_string();
        resource_methods.push(quote! {
            pub async fn bulk_destroy<D: ::ash_core::DataLayer>(
                ctx: &::ash_core::Context<D>,
                ids: &[::uuid::Uuid],
            ) -> ::ash_core::Result<::ash_core::BulkResult<Self>> {
                ::ash_core::bulk_destroy(ctx, #act_name_str, ids, ::ash_core::BulkDestroyOptions::default()).await
            }

            pub async fn bulk_destroy_with_opts<D: ::ash_core::DataLayer>(
                ctx: &::ash_core::Context<D>,
                action: &str,
                ids: &[::uuid::Uuid],
                opts: ::ash_core::BulkDestroyOptions,
            ) -> ::ash_core::Result<::ash_core::BulkResult<Self>> {
                ::ash_core::bulk_destroy(ctx, action, ids, opts).await
            }
        });
    }

    let actions_trait_ident = format_ident!("{}Actions", resource);

    let mut rel_methods = Vec::new();
    for rel in &def.relationships {
        let rel_ident = &rel.ident;
        let rel_str = rel_ident.to_string();
        let manage_method_name = format_ident!("manage_{}", rel_ident);
        rel_methods.push(quote! {
            pub fn #manage_method_name<I, F>(self, inputs: I, rel_type: ::ash_core::ManagedRelType) -> Self
            where
                I: ::std::iter::IntoIterator<Item = F>,
                F: ::ash_core::IntoFieldMap,
            {
                self.manage_relationship(#rel_str, inputs, rel_type)
            }
        });
    }

    for act in actions {
        let act_name = &act.name;
        let act_name_str = act_name.to_string();
        let act_pascal = pascal_case(&act_name_str);
        let builder_name = format_ident!("{}{}Action", resource, act_pascal);
        let outer_attrs = &act.outer_attrs;

        match act.kind {
            ActionKind::Create => {
                let mut field_members = Vec::new();
                let mut field_inits = Vec::new();
                let mut optional_setters = Vec::new();
                let mut into_fields_inserts = Vec::new();
                let mut required_info = Vec::new();
                let mut all_input_names = Vec::new();
                let required_names = required_input_names(def, act);
                let (struct_params, _, _) = typestate_types(required_names.len());
                let has_state = !required_names.is_empty();
                let phantom_field = if has_state {
                    quote! { _state: ::std::marker::PhantomData<S>, }
                } else {
                    quote! {}
                };
                let phantom_init = if has_state {
                    quote! { _state: ::std::marker::PhantomData, }
                } else {
                    quote! {}
                };

                let all_inputs: Vec<(&syn::Ident, &syn::Type)> = act
                    .accept
                    .iter()
                    .map(|a| (&a.name, &a.ty))
                    .chain(act.arguments.iter().map(|a| (&a.name, &a.ty)))
                    .collect();

                for (name, ty) in all_inputs {
                    let s = name.to_string();
                    let docs = docs_for_input_name(def, act, name);
                    all_input_names.push(name.clone());
                    field_members.push(quote! { pub #name: ::std::option::Option<#ty> });
                    field_inits.push(quote! { #name: ::std::option::Option::None });
                    let is_req = required_names.contains(&name);
                    if is_req {
                        let docs_ts = quote! { #(#docs)* };
                        required_info.push((
                            name.clone(),
                            ty.clone(),
                            option_inner(ty).is_some(),
                            docs_ts,
                        ));
                    } else if let Some(inner) = option_inner(ty) {
                        optional_setters.push(quote! {
                            #(#docs)*
                            pub fn #name(mut self, value: impl ::ash_core::IntoOption<#inner>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into_option());
                                self
                            }
                        });
                    } else {
                        optional_setters.push(quote! {
                            #(#docs)*
                            pub fn #name(mut self, value: impl ::std::convert::Into<#ty>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into());
                                self
                            }
                        });
                    }
                    if let Some(_inner) = option_inner(ty) {
                        into_fields_inserts.push(quote! {
                            if let ::std::option::Option::Some(opt_val) = &self.#name {
                                match opt_val {
                                    ::std::option::Option::Some(val) => {
                                        map.insert(::std::string::String::from(#s), ::ash_core::Value::from(val.clone()));
                                    }
                                    ::std::option::Option::None => {
                                        map.insert(::std::string::String::from(#s), ::ash_core::Value::Null);
                                    }
                                }
                            }
                        });
                    } else {
                        into_fields_inserts.push(quote! {
                            if let ::std::option::Option::Some(val) = &self.#name {
                                map.insert(::std::string::String::from(#s), ::ash_core::Value::from(val.clone()));
                            }
                        });
                    }
                }

                let extra_moves_manual = vec![quote! { ctx: self.ctx }];
                let required_impls_manual = required_setter_impls(
                    &builder_name,
                    &required_info,
                    &extra_moves_manual,
                    &all_input_names,
                );
                let extra_moves_create = vec![
                    quote! { ctx: self.ctx },
                    quote! { tenant_override: self.tenant_override },
                    quote! { upsert_spec: self.upsert_spec },
                    quote! { before_actions: self.before_actions },
                    quote! { after_actions: self.after_actions },
                    quote! { after_transactions: self.after_transactions },
                    quote! { managed_relationships: self.managed_relationships },
                ];
                let required_impls_create = required_setter_impls(
                    &builder_name,
                    &required_info,
                    &extra_moves_create,
                    &all_input_names,
                );
                let impl_generic = if has_state {
                    quote! { impl<'a, D: ::ash_core::DataLayer, S> #builder_name<'a, D, S> }
                } else {
                    quote! { impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> }
                };
                let (need_traits, impl_ready, into_future_impl, into_changeset_impl) =
                    typestate_ready_impls(&builder_name, resource, &act_pascal, &required_names);
                let impl_new = quote! { impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> };
                let into_fieldmap_owned = if has_state {
                    quote! {
                        impl<'a, D: ::ash_core::DataLayer, S> ::ash_core::IntoFieldMap for #builder_name<'a, D, S> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }
                    }
                } else {
                    quote! {
                        impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for #builder_name<'a, D> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }
                    }
                };
                let into_fieldmap_ref = into_fieldmap_ref_impl(&builder_name, has_state);

                if act.persist_manual {
                    builders.push(quote! {
                        #need_traits

                        #(#outer_attrs)*
                        #[must_use = "this action is not executed unless you `.await` or `.call()` it"]
                        pub struct #builder_name<'a, D #struct_params> {
                            ctx: &'a ::ash_core::Context<D>,
                            #(#field_members,)*
                            #phantom_field
                        }

                        #impl_new {
                            pub fn new(ctx: &'a ::ash_core::Context<D>) -> Self {
                                Self {
                                    ctx,
                                    #(#field_inits,)*
                                    #phantom_init
                                }
                            }
                        }

                        #impl_generic {
                            #(#optional_setters)*

                            pub fn into_fields(&self) -> ::ash_core::FieldMap {
                                let mut map = ::ash_core::FieldMap::new();
                                #(#into_fields_inserts)*
                                map
                            }
                        }

                        #(#required_impls_manual)*

                        #impl_ready {
                            pub async fn persist<F, Fut>(self, f: F) -> ::ash_core::Result<#resource>
                            where
                                F: ::std::ops::FnOnce(&::ash_core::Context<D>, #resource) -> Fut,
                                Fut: ::std::future::Future<Output = ::ash_core::Result<#resource>>,
                            {
                                let fields = self.into_fields();
                                ::ash_core::manual_create(self.ctx, #act_name_str, fields, f).await
                            }
                        }

                        #into_fieldmap_owned

                        #into_fieldmap_ref
                    });
                } else {
                    builders.push(quote! {
                        #need_traits

                        #(#outer_attrs)*
                        #[must_use = "this action is not executed unless you `.await` or `.call()` it"]
                        pub struct #builder_name<'a, D #struct_params> {
                            ctx: &'a ::ash_core::Context<D>,
                            tenant_override: ::std::option::Option<::std::string::String>,
                            upsert_spec: ::std::option::Option<(&'static str, ::std::vec::Vec<::std::string::String>)>,
                            before_actions: ::std::vec::Vec<::ash_core::BeforeActionHook<#resource>>,
                            after_actions: ::std::vec::Vec<::ash_core::AfterActionHook<#resource>>,
                            after_transactions: ::std::vec::Vec<::ash_core::AfterTransactionHook<#resource>>,
                            managed_relationships: ::std::vec::Vec<::ash_core::ManagedRelationshipSpec>,
                            #(#field_members,)*
                            #phantom_field
                        }

                        #impl_new {
                            pub fn new(ctx: &'a ::ash_core::Context<D>) -> Self {
                                Self {
                                    ctx,
                                    tenant_override: ::std::option::Option::None,
                                    upsert_spec: ::std::option::Option::None,
                                    before_actions: ::std::vec::Vec::new(),
                                    after_actions: ::std::vec::Vec::new(),
                                    after_transactions: ::std::vec::Vec::new(),
                                    managed_relationships: ::std::vec::Vec::new(),
                                    #(#field_inits,)*
                                    #phantom_init
                                }
                            }
                        }

                        #impl_generic {
                            /// Explicitly override the tenant on this action builder.
                            pub fn tenant(mut self, tenant: impl ::std::convert::Into<::std::string::String>) -> Self {
                                self.tenant_override = ::std::option::Option::Some(tenant.into());
                                self
                            }

                            /// Clear any tenant override on this action builder.
                            pub fn without_tenant(mut self) -> Self {
                                self.tenant_override = ::std::option::Option::None;
                                self
                            }

                            pub fn upsert(mut self, identity: &'static str, update_fields: &[&str]) -> Self {
                                self.upsert_spec = ::std::option::Option::Some((
                                    identity,
                                    update_fields.iter().map(|s| ::std::string::String::from(*s)).collect(),
                                ));
                                self
                            }

                            pub fn upsert_on(self, identity: &'static str, update_fields: &[&str]) -> Self {
                                self.upsert(identity, update_fields)
                            }

                            pub fn before_action<F>(mut self, hook: F) -> Self
                            where
                                F: ::std::ops::FnOnce(&mut ::ash_core::Changeset<#resource>) -> ::ash_core::Result<()> + ::std::marker::Send + 'static,
                            {
                                self.before_actions.push(::std::boxed::Box::new(hook));
                                self
                            }

                            pub fn after_action<F>(mut self, hook: F) -> Self
                            where
                                F: ::std::ops::FnOnce(&mut #resource) -> ::ash_core::Result<()> + ::std::marker::Send + 'static,
                            {
                                self.after_actions.push(::std::boxed::Box::new(hook));
                                self
                            }

                            pub fn after_transaction<F>(mut self, hook: F) -> Self
                            where
                                F: ::std::ops::FnOnce(::std::result::Result<&#resource, &::ash_core::Error>) + ::std::marker::Send + 'static,
                            {
                                self.after_transactions.push(::std::boxed::Box::new(hook));
                                self
                            }

                            pub fn manage_relationship<I, F>(
                                mut self,
                                relationship: &'static str,
                                inputs: I,
                                rel_type: ::ash_core::ManagedRelType,
                            ) -> Self
                            where
                                I: ::std::iter::IntoIterator<Item = F>,
                                F: ::ash_core::IntoFieldMap,
                            {
                                let field_maps: ::std::vec::Vec<::ash_core::FieldMap> = inputs.into_iter().map(|f| f.into_field_map()).collect();
                                self.managed_relationships.push(::ash_core::ManagedRelationshipSpec {
                                    relationship,
                                    rel_type,
                                    inputs: field_maps,
                                });
                                self
                            }

                            pub fn manage_relationship_one(
                                self,
                                relationship: &'static str,
                                input: impl ::ash_core::IntoFieldMap,
                                rel_type: ::ash_core::ManagedRelType,
                            ) -> Self {
                                self.manage_relationship(relationship, ::std::vec![input.into_field_map()], rel_type)
                            }

                            #(#rel_methods)*

                            #(#optional_setters)*

                            pub fn into_fields(&self) -> ::ash_core::FieldMap {
                                let mut map = ::ash_core::FieldMap::new();
                                #(#into_fields_inserts)*
                                map
                            }
                        }

                        #(#required_impls_create)*

                        #impl_ready {
                            pub fn changeset(self) -> ::ash_core::Result<::ash_core::Changeset<#resource>> {
                                let ctx_owned;
                                let ctx = if let ::std::option::Option::Some(ref t) = self.tenant_override {
                                    ctx_owned = self.ctx.with_tenant(t.clone());
                                    &ctx_owned
                                } else {
                                    self.ctx
                                };
                                let fields = self.into_fields();
                                let upsert_spec = self.upsert_spec;
                                let before_actions = self.before_actions;
                                let after_actions = self.after_actions;
                                let after_transactions = self.after_transactions;
                                let managed_relationships = self.managed_relationships;
                                let mut cs = ::ash_core::Changeset::<#resource>::for_create(ctx, #act_name_str, fields)?;
                                if let ::std::option::Option::Some((ident, ref u_fields)) = upsert_spec {
                                    let field_strs: ::std::vec::Vec<&str> = u_fields.iter().map(|s| s.as_str()).collect();
                                    cs = cs.with_upsert(ident, &field_strs);
                                }
                                for hook in before_actions {
                                    cs = cs.before_action(hook);
                                }
                                for hook in after_actions {
                                    cs = cs.after_action(hook);
                                }
                                for hook in after_transactions {
                                    cs = cs.after_transaction(hook);
                                }
                                for managed in managed_relationships {
                                    cs = cs.manage_relationship(managed.relationship, managed.inputs, managed.rel_type);
                                }
                                Ok(cs)
                            }

                            pub fn build(self) -> ::ash_core::Result<#resource> {
                                let fields = self.into_fields();
                                ::ash_core::Changeset::<#resource>::apply_embedded(#act_name_str, fields)
                            }

                            pub async fn call(self) -> ::ash_core::Result<#resource> {
                                let ctx_owned;
                                let ctx = if let ::std::option::Option::Some(ref t) = self.tenant_override {
                                    ctx_owned = self.ctx.with_tenant(t.clone());
                                    &ctx_owned
                                } else {
                                    self.ctx
                                };
                                self.changeset()?.commit(ctx).await
                            }
                        }

                        #into_fieldmap_owned

                        #into_fieldmap_ref

                        #into_future_impl {
                            type Output = ::ash_core::Result<#resource>;
                            type IntoFuture = ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = Self::Output> + ::std::marker::Send + 'a>>;

                            fn into_future(self) -> Self::IntoFuture {
                                ::std::boxed::Box::pin(self.call())
                            }
                        }

                        #into_changeset_impl {
                            fn into_changeset(self) -> ::ash_core::Result<::ash_core::Changeset<#resource>> {
                                self.changeset()
                            }
                        }
                    });
                }

                let build_act_name = format_ident!("build_{}", act_name);
                resource_methods.push(quote! {
                    #(#outer_attrs)*
                    pub fn #act_name<'a, D: ::ash_core::DataLayer>(ctx: &'a ::ash_core::Context<D>) -> #builder_name<'a, D> {
                        #builder_name::new(ctx)
                    }

                    #(#outer_attrs)*
                    pub fn #build_act_name() -> #builder_name<'static, ::ash_memory::Memory> {
                        static DUMMY: ::std::sync::OnceLock<::ash_core::Context<::ash_memory::Memory>> = ::std::sync::OnceLock::new();
                        let ctx = DUMMY.get_or_init(|| ::ash_core::Context::new(::ash_memory::Memory::new()));
                        #builder_name::new(ctx)
                    }
                });
            }

            ActionKind::Update => {
                let target_enum = format_ident!("{}{}Target", resource, act_pascal);

                let mut field_members = Vec::new();
                let mut field_inits = Vec::new();
                let mut optional_setters = Vec::new();
                let mut into_fields_inserts = Vec::new();
                let mut required_info = Vec::new();
                let mut all_input_names = Vec::new();
                let required_names = required_input_names(def, act);
                let (struct_params, _, _) = typestate_types(required_names.len());
                let has_state = !required_names.is_empty();
                let phantom_field = if has_state {
                    quote! { _state: ::std::marker::PhantomData<S>, }
                } else {
                    quote! {}
                };
                let phantom_init = if has_state {
                    quote! { _state: ::std::marker::PhantomData, }
                } else {
                    quote! {}
                };
                let impl_generic = if has_state {
                    quote! { impl<'a, D: ::ash_core::DataLayer, S> #builder_name<'a, D, S> }
                } else {
                    quote! { impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> }
                };
                let (need_traits, impl_ready, into_future_impl, into_changeset_impl) =
                    typestate_ready_impls(&builder_name, resource, &act_pascal, &required_names);
                let impl_new = quote! { impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> };
                let into_fieldmap_owned = if has_state {
                    quote! {
                        impl<'a, D: ::ash_core::DataLayer, S> ::ash_core::IntoFieldMap for #builder_name<'a, D, S> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }
                    }
                } else {
                    quote! {
                        impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for #builder_name<'a, D> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }
                    }
                };
                let into_fieldmap_ref = into_fieldmap_ref_impl(&builder_name, has_state);

                let all_inputs: Vec<(&syn::Ident, &syn::Type)> = act
                    .accept
                    .iter()
                    .map(|a| (&a.name, &a.ty))
                    .chain(act.arguments.iter().map(|a| (&a.name, &a.ty)))
                    .collect();

                for (name, ty) in all_inputs {
                    let s = name.to_string();
                    let docs = docs_for_input_name(def, act, name);
                    all_input_names.push(name.clone());
                    field_members.push(quote! { pub #name: ::std::option::Option<#ty> });
                    field_inits.push(quote! { #name: ::std::option::Option::None });
                    let is_req = required_names.contains(&name);
                    if is_req {
                        let docs_ts = quote! { #(#docs)* };
                        required_info.push((
                            name.clone(),
                            ty.clone(),
                            option_inner(ty).is_some(),
                            docs_ts,
                        ));
                    } else if let Some(inner) = option_inner(ty) {
                        optional_setters.push(quote! {
                            #(#docs)*
                            pub fn #name(mut self, value: impl ::ash_core::IntoOption<#inner>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into_option());
                                self
                            }
                        });
                    } else {
                        optional_setters.push(quote! {
                            #(#docs)*
                            pub fn #name(mut self, value: impl ::std::convert::Into<#ty>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into());
                                self
                            }
                        });
                    }
                    if option_inner(ty).is_some() {
                        into_fields_inserts.push(quote! {
                            if let ::std::option::Option::Some(opt_val) = &self.#name {
                                match opt_val {
                                    ::std::option::Option::Some(val) => {
                                        map.insert(::std::string::String::from(#s), ::ash_core::Value::from(val.clone()));
                                    }
                                    ::std::option::Option::None => {
                                        map.insert(::std::string::String::from(#s), ::ash_core::Value::Null);
                                    }
                                }
                            }
                        });
                    } else {
                        into_fields_inserts.push(quote! {
                            if let ::std::option::Option::Some(val) = &self.#name {
                                map.insert(::std::string::String::from(#s), ::ash_core::Value::from(val.clone()));
                            }
                        });
                    }
                }

                let extra_moves_update = vec![
                    quote! { ctx: self.ctx },
                    quote! { target: self.target },
                    quote! { tenant_override: self.tenant_override },
                    quote! { before_actions: self.before_actions },
                    quote! { after_actions: self.after_actions },
                    quote! { after_transactions: self.after_transactions },
                    quote! { managed_relationships: self.managed_relationships },
                ];
                let required_impls_update = required_setter_impls(
                    &builder_name,
                    &required_info,
                    &extra_moves_update,
                    &all_input_names,
                );

                builders.push(quote! {
                    #need_traits

                    pub enum #target_enum {
                        Id(::uuid::Uuid),
                        Existing(#resource),
                    }

                    #(#outer_attrs)*
                    #[must_use = "this action is not executed unless you `.await` or `.call()` it"]
                    pub struct #builder_name<'a, D #struct_params> {
                        ctx: &'a ::ash_core::Context<D>,
                        target: #target_enum,
                        tenant_override: ::std::option::Option<::std::string::String>,
                        before_actions: ::std::vec::Vec<::ash_core::BeforeActionHook<#resource>>,
                        after_actions: ::std::vec::Vec<::ash_core::AfterActionHook<#resource>>,
                        after_transactions: ::std::vec::Vec<::ash_core::AfterTransactionHook<#resource>>,
                        managed_relationships: ::std::vec::Vec<::ash_core::ManagedRelationshipSpec>,
                        #(#field_members,)*
                        #phantom_field
                    }

                    #impl_new {
                        pub fn for_id(ctx: &'a ::ash_core::Context<D>, id: ::uuid::Uuid) -> Self {
                            Self {
                                ctx,
                                target: #target_enum::Id(id),
                                tenant_override: ::std::option::Option::None,
                                before_actions: ::std::vec::Vec::new(),
                                after_actions: ::std::vec::Vec::new(),
                                after_transactions: ::std::vec::Vec::new(),
                                managed_relationships: ::std::vec::Vec::new(),
                                #(#field_inits,)*
                                #phantom_init
                            }
                        }

                        pub fn for_existing(ctx: &'a ::ash_core::Context<D>, existing: #resource) -> Self {
                            Self {
                                ctx,
                                target: #target_enum::Existing(existing),
                                tenant_override: ::std::option::Option::None,
                                before_actions: ::std::vec::Vec::new(),
                                after_actions: ::std::vec::Vec::new(),
                                after_transactions: ::std::vec::Vec::new(),
                                managed_relationships: ::std::vec::Vec::new(),
                                #(#field_inits,)*
                                #phantom_init
                            }
                        }
                    }

                    #impl_generic {

                        /// Explicitly override the tenant on this action builder.
                        pub fn tenant(mut self, tenant: impl ::std::convert::Into<::std::string::String>) -> Self {
                            self.tenant_override = ::std::option::Option::Some(tenant.into());
                            self
                        }

                        /// Clear any tenant override on this action builder.
                        pub fn without_tenant(mut self) -> Self {
                            self.tenant_override = ::std::option::Option::None;
                            self
                        }

                        pub fn before_action<F>(mut self, hook: F) -> Self
                        where
                            F: ::std::ops::FnOnce(&mut ::ash_core::Changeset<#resource>) -> ::ash_core::Result<()> + ::std::marker::Send + 'static,
                        {
                            self.before_actions.push(::std::boxed::Box::new(hook));
                            self
                        }

                        pub fn after_action<F>(mut self, hook: F) -> Self
                        where
                            F: ::std::ops::FnOnce(&mut #resource) -> ::ash_core::Result<()> + ::std::marker::Send + 'static,
                        {
                            self.after_actions.push(::std::boxed::Box::new(hook));
                            self
                        }

                        pub fn after_transaction<F>(mut self, hook: F) -> Self
                        where
                            F: ::std::ops::FnOnce(::std::result::Result<&#resource, &::ash_core::Error>) + ::std::marker::Send + 'static,
                        {
                            self.after_transactions.push(::std::boxed::Box::new(hook));
                            self
                        }

                        pub fn manage_relationship<I, F>(
                            mut self,
                            relationship: &'static str,
                            inputs: I,
                            rel_type: ::ash_core::ManagedRelType,
                        ) -> Self
                        where
                            I: ::std::iter::IntoIterator<Item = F>,
                            F: ::ash_core::IntoFieldMap,
                        {
                            let field_maps: ::std::vec::Vec<::ash_core::FieldMap> = inputs.into_iter().map(|f| f.into_field_map()).collect();
                            self.managed_relationships.push(::ash_core::ManagedRelationshipSpec {
                                relationship,
                                rel_type,
                                inputs: field_maps,
                            });
                            self
                        }

                        pub fn manage_relationship_one(
                            self,
                            relationship: &'static str,
                            input: impl ::ash_core::IntoFieldMap,
                            rel_type: ::ash_core::ManagedRelType,
                        ) -> Self {
                            self.manage_relationship(relationship, ::std::vec![input.into_field_map()], rel_type)
                        }

                        #(#rel_methods)*

                        #(#optional_setters)*

                        pub fn into_fields(&self) -> ::ash_core::FieldMap {
                            let mut map = ::ash_core::FieldMap::new();
                            #(#into_fields_inserts)*
                            map
                        }
                    }

                    #(#required_impls_update)*

                    #impl_ready {
                        pub fn changeset(self) -> ::ash_core::Result<::ash_core::Changeset<#resource>> {
                            let ctx_owned;
                            let ctx = if let ::std::option::Option::Some(ref t) = self.tenant_override {
                                ctx_owned = self.ctx.with_tenant(t.clone());
                                &ctx_owned
                            } else {
                                self.ctx
                            };
                            let fields = self.into_fields();
                            let before_actions = self.before_actions;
                            let after_actions = self.after_actions;
                            let after_transactions = self.after_transactions;
                            let managed_relationships = self.managed_relationships;
                            let tenant_override = self.tenant_override;
                            match self.target {
                                #target_enum::Existing(record) => {
                                    let mut cs = ::ash_core::Changeset::<#resource>::for_update_on(ctx, #act_name_str, record, fields)?;
                                    if let ::std::option::Option::Some(t) = tenant_override {
                                        cs = cs.with_tenant(t);
                                    }
                                    for hook in before_actions {
                                        cs = cs.before_action(hook);
                                    }
                                    for hook in after_actions {
                                        cs = cs.after_action(hook);
                                    }
                                    for hook in after_transactions {
                                        cs = cs.after_transaction(hook);
                                    }
                                    for managed in managed_relationships {
                                        cs = cs.manage_relationship(managed.relationship, managed.inputs, managed.rel_type);
                                    }
                                    Ok(cs)
                                }
                                #target_enum::Id(_) => {
                                    Err(::ash_core::Error::Invalid(
                                        "changeset for update requires an existing record; call on an instance instead".into()
                                    ))
                                }
                            }
                        }

                        pub fn build(self) -> ::ash_core::Result<#resource> {
                            let fields = self.into_fields();
                            match self.target {
                                #target_enum::Existing(record) => {
                                    ::ash_core::Changeset::<#resource>::apply_embedded_update(#act_name_str, record, fields)
                                }
                                #target_enum::Id(_) => Err(::ash_core::Error::Invalid(
                                    "build for update requires an existing record; call on an instance instead".into(),
                                )),
                            }
                        }

                        pub async fn call(self) -> ::ash_core::Result<#resource> {
                            let ctx_owned;
                            let ctx = if let ::std::option::Option::Some(ref t) = self.tenant_override {
                                ctx_owned = self.ctx.with_tenant(t.clone());
                                &ctx_owned
                            } else {
                                self.ctx
                            };
                            let fields = self.into_fields();
                            let before_actions = self.before_actions;
                            let after_actions = self.after_actions;
                            let after_transactions = self.after_transactions;
                            let managed_relationships = self.managed_relationships;
                            let tenant_override = self.tenant_override;
                            let existing = match self.target {
                                #target_enum::Id(id) => ::ash_core::get::<#resource, D>(ctx, id).await?,
                                #target_enum::Existing(record) => record,
                            };
                            let mut cs = ::ash_core::Changeset::<#resource>::for_update_on(ctx, #act_name_str, existing, fields)?;
                            if let ::std::option::Option::Some(t) = tenant_override {
                                cs = cs.with_tenant(t);
                            }
                            for hook in before_actions {
                                cs = cs.before_action(hook);
                            }
                            for hook in after_actions {
                                cs = cs.after_action(hook);
                            }
                            for hook in after_transactions {
                                cs = cs.after_transaction(hook);
                            }
                            for managed in managed_relationships {
                                cs = cs.manage_relationship(managed.relationship, managed.inputs, managed.rel_type);
                            }
                            cs.commit(ctx).await
                        }
                    }

                    #into_fieldmap_owned

                    #into_fieldmap_ref

                    #into_future_impl {
                        type Output = ::ash_core::Result<#resource>;
                        type IntoFuture = ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = Self::Output> + ::std::marker::Send + 'a>>;

                        fn into_future(self) -> Self::IntoFuture {
                            ::std::boxed::Box::pin(self.call())
                        }
                    }

                    #into_changeset_impl {
                        fn into_changeset(self) -> ::ash_core::Result<::ash_core::Changeset<#resource>> {
                            self.changeset()
                        }
                    }
                });

                let act_on_name = format_ident!("{}_on", act_name);
                let build_act_name = format_ident!("build_{}", act_name);

                resource_methods.push(quote! {
                    #(#outer_attrs)*
                    pub fn #act_name<'a, D: ::ash_core::DataLayer>(
                        ctx: &'a ::ash_core::Context<D>,
                        target: impl ::std::convert::Into<::ash_core::ActionTarget<Self>>,
                    ) -> #builder_name<'a, D> {
                        match target.into() {
                            ::ash_core::ActionTarget::Id(id) => #builder_name::for_id(ctx, id),
                            ::ash_core::ActionTarget::Record(rec) => #builder_name::for_existing(ctx, rec),
                        }
                    }

                    #(#outer_attrs)*
                    pub fn #act_on_name<'a, D: ::ash_core::DataLayer>(&self, ctx: &'a ::ash_core::Context<D>) -> #builder_name<'a, D> {
                        #builder_name::for_existing(ctx, self.clone())
                    }

                    #(#outer_attrs)*
                    pub fn #build_act_name(&self) -> #builder_name<'static, ::ash_memory::Memory> {
                        static DUMMY: ::std::sync::OnceLock<::ash_core::Context<::ash_memory::Memory>> = ::std::sync::OnceLock::new();
                        let ctx = DUMMY.get_or_init(|| ::ash_core::Context::new(::ash_memory::Memory::new()));
                        #builder_name::for_existing(ctx, self.clone())
                    }
                });

                trait_methods.push(quote! {
                    #(#outer_attrs)*
                    fn #act_name<'a, D: ::ash_core::DataLayer>(&self, ctx: &'a ::ash_core::Context<D>) -> #builder_name<'a, D>;
                });

                trait_impls.push(quote! {
                    fn #act_name<'a, D: ::ash_core::DataLayer>(&self, ctx: &'a ::ash_core::Context<D>) -> #builder_name<'a, D> {
                        self.#act_on_name(ctx)
                    }
                });
            }

            ActionKind::Destroy => {
                let act_on_name = format_ident!("{}_on", act_name);
                resource_methods.push(quote! {
                    #(#outer_attrs)*
                    pub async fn #act_name<D: ::ash_core::DataLayer>(
                        ctx: &::ash_core::Context<D>,
                        target: impl ::std::convert::Into<::ash_core::ActionTarget<Self>>,
                    ) -> ::ash_core::Result<()> {
                        match target.into() {
                            ::ash_core::ActionTarget::Id(id) => ::ash_core::destroy::<#resource, D>(ctx, #act_name_str, id).await,
                            ::ash_core::ActionTarget::Record(rec) => ::ash_core::destroy_existing::<#resource, D>(ctx, #act_name_str, rec).await,
                        }
                    }

                    #(#outer_attrs)*
                    pub async fn #act_on_name<D: ::ash_core::DataLayer>(&self, ctx: &::ash_core::Context<D>) -> ::ash_core::Result<()> {
                        ::ash_core::destroy_existing::<#resource, D>(ctx, #act_name_str, self.clone()).await
                    }
                });

                trait_methods.push(quote! {
                    #(#outer_attrs)*
                    fn #act_name<D: ::ash_core::DataLayer>(&self, ctx: &::ash_core::Context<D>) -> impl ::std::future::Future<Output = ::ash_core::Result<()>> + Send;
                });

                trait_impls.push(quote! {
                    fn #act_name<D: ::ash_core::DataLayer>(&self, ctx: &::ash_core::Context<D>) -> impl ::std::future::Future<Output = ::ash_core::Result<()>> + Send {
                        self.#act_on_name(ctx)
                    }
                });
            }

            ActionKind::Generic => {
                let input_struct_name = format_ident!("{}{}Input", resource, act_pascal);
                let returns_ty = act.returns.clone().unwrap_or_else(|| syn::parse_quote!(()));

                let mut field_members = Vec::new();
                let mut field_inits = Vec::new();
                let mut optional_setters = Vec::new();
                let mut input_struct_fields = Vec::new();
                let mut input_extracts = Vec::new();
                let mut required_info = Vec::new();
                let mut all_input_names = Vec::new();
                let required_names = required_input_names(def, act);
                let (struct_params, _, _) = typestate_types(required_names.len());
                let has_state = !required_names.is_empty();
                let phantom_field = if has_state {
                    quote! { _state: ::std::marker::PhantomData<S>, }
                } else {
                    quote! {}
                };
                let phantom_init = if has_state {
                    quote! { _state: ::std::marker::PhantomData, }
                } else {
                    quote! {}
                };
                let impl_generic = if has_state {
                    quote! { impl<'a, D: ::ash_core::DataLayer, S> #builder_name<'a, D, S> }
                } else {
                    quote! { impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> }
                };
                let (need_traits, impl_ready, into_future_impl, _) =
                    typestate_ready_impls(&builder_name, resource, &act_pascal, &required_names);
                let impl_new = quote! { impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> };

                let all_inputs: Vec<(&syn::Ident, &syn::Type)> = act
                    .accept
                    .iter()
                    .map(|a| (&a.name, &a.ty))
                    .chain(act.arguments.iter().map(|a| (&a.name, &a.ty)))
                    .collect();

                for (name, ty) in all_inputs {
                    let s = name.to_string();
                    let docs = docs_for_input_name(def, act, name);
                    let arg_docs = arg_docs_for_name(act, name);
                    all_input_names.push(name.clone());
                    input_struct_fields.push(quote! { #(#arg_docs)* pub #name: #ty });
                    field_members.push(quote! { pub #name: ::std::option::Option<#ty> });
                    field_inits.push(quote! { #name: ::std::option::Option::None });
                    let is_req = required_names.contains(&name);

                    if is_req {
                        let docs_ts = quote! { #(#docs)* };
                        required_info.push((
                            name.clone(),
                            ty.clone(),
                            option_inner(ty).is_some(),
                            docs_ts,
                        ));
                    } else if let Some(inner) = option_inner(ty) {
                        optional_setters.push(quote! {
                            #(#docs)*
                            pub fn #name(mut self, value: impl ::ash_core::IntoOption<#inner>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into_option());
                                self
                            }
                        });
                    } else {
                        optional_setters.push(quote! {
                            #(#docs)*
                            pub fn #name(mut self, value: impl ::std::convert::Into<#ty>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into());
                                self
                            }
                        });
                    }

                    if option_inner(ty).is_some() {
                        input_extracts.push(quote! {
                            #name: self.#name.unwrap_or(::std::option::Option::None)
                        });
                    } else {
                        input_extracts.push(quote! {
                            #name: self.#name.ok_or_else(|| ::ash_core::Error::Missing { field: #s.into() })?
                        });
                    }
                }

                let extra_moves = vec![quote! { ctx: self.ctx }];
                let required_impls = required_setter_impls(
                    &builder_name,
                    &required_info,
                    &extra_moves,
                    &all_input_names,
                );

                let run_impl = if let Some(expr) = &act.run_expr {
                    quote! {
                        let fut = Self::__run_action(#expr, input);
                        ::ash_core::run::<#resource, D, #returns_ty, _, _>(ctx, #act_name_str, move || fut).await
                    }
                } else {
                    quote! {
                        Err(::ash_core::Error::Invalid("generic action has no run closure or runner; provide one via `.run(...)`".into()))
                    }
                };

                builders.push(quote! {
                    #need_traits

                    pub struct #input_struct_name<'a, D> {
                        pub ctx: &'a ::ash_core::Context<D>,
                        #(#input_struct_fields,)*
                    }

                    impl<'a, D: ::ash_core::DataLayer> #input_struct_name<'a, D> {
                        pub fn actor(&self) -> ::std::option::Option<&::ash_core::Actor> {
                            self.ctx.actor.as_ref()
                        }

                        pub fn tenant(&self) -> ::std::option::Option<&str> {
                            self.ctx.tenant()
                        }

                        pub fn metadata(&self) -> &::ash_core::FieldMap {
                            self.ctx.metadata()
                        }
                    }

                    #(#outer_attrs)*
                    #[must_use = "this action is not executed unless you `.await` or `.call()` it"]
                    pub struct #builder_name<'a, D #struct_params> {
                        ctx: &'a ::ash_core::Context<D>,
                        #(#field_members,)*
                        #phantom_field
                    }

                    #impl_new {
                        pub fn new(ctx: &'a ::ash_core::Context<D>) -> Self {
                            Self {
                                ctx,
                                #(#field_inits,)*
                                #phantom_init
                            }
                        }
                    }

                    #impl_generic {
                        #(#optional_setters)*

                        fn __run_action<F, Fut>(f: F, input: #input_struct_name<'a, D>) -> Fut
                        where
                            F: ::std::ops::FnOnce(#input_struct_name<'a, D>) -> Fut,
                            Fut: ::std::future::Future<Output = ::ash_core::Result<#returns_ty>> + ::std::marker::Send,
                        {
                            f(input)
                        }
                    }

                    #(#required_impls)*

                    #impl_ready {
                        pub async fn run<F, Fut>(self, runner: F) -> ::ash_core::Result<#returns_ty>
                        where
                            F: ::std::ops::FnOnce(#input_struct_name<'a, D>) -> Fut,
                            Fut: ::std::future::Future<Output = ::ash_core::Result<#returns_ty>> + ::std::marker::Send,
                        {
                            let ctx = self.ctx;
                            let input = #input_struct_name {
                                ctx,
                                #(#input_extracts,)*
                            };
                            let fut = runner(input);
                            ::ash_core::run::<#resource, D, #returns_ty, _, _>(ctx, #act_name_str, move || fut).await
                        }

                        pub async fn call(self) -> ::ash_core::Result<#returns_ty> {
                            let ctx = self.ctx;
                            let input = #input_struct_name {
                                ctx,
                                #(#input_extracts,)*
                            };
                            #run_impl
                        }
                    }

                    #into_future_impl {
                        type Output = ::ash_core::Result<#returns_ty>;
                        type IntoFuture = ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = Self::Output> + ::std::marker::Send + 'a>>;

                        fn into_future(self) -> Self::IntoFuture {
                            ::std::boxed::Box::pin(self.call())
                        }
                    }
                });

                resource_methods.push(quote! {
                    #(#outer_attrs)*
                    pub fn #act_name<'a, D: ::ash_core::DataLayer>(ctx: &'a ::ash_core::Context<D>) -> #builder_name<'a, D> {
                        #builder_name::new(ctx)
                    }
                });
            }

            ActionKind::Read => {
                if !act.primary && act_name != "read" {
                    let custom_query_fn = format_ident!("{}_query", act_name);
                    resource_methods.push(quote! {
                        pub fn #custom_query_fn<D: ::ash_core::DataLayer>(ctx: &::ash_core::Context<D>) -> ::ash_core::Query<'_, Self, D> {
                            ::ash_core::query(ctx).action(#act_name_str)
                        }

                        pub fn #act_name<D: ::ash_core::DataLayer>(ctx: &::ash_core::Context<D>) -> ::ash_core::Query<'_, Self, D> {
                            ::ash_core::query(ctx).action(#act_name_str)
                        }
                    });
                }
            }
        }
    }

    let trait_block = if !trait_methods.is_empty() {
        quote! {
            pub trait #actions_trait_ident {
                #(#trait_methods)*
            }

            impl #actions_trait_ident for #resource {
                #(#trait_impls)*
            }
        }
    } else {
        quote! {}
    };

    ActionCodegen {
        builders,
        resource_methods,
        trait_block,
    }
}
