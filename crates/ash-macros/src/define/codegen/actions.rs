use super::policies::lit_to_const_value;
use crate::ast_helpers::{is_bool, is_i64, is_string, is_uuid, option_inner, pascal_case};
use crate::define::ast::{
    ActionKind, ChangeSpec, PreparationSpec, ResourceDefinition, ValidationSpec,
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Result;

fn filter_expr_to_tokens(expr: &syn::Expr, resource: &syn::Ident) -> TokenStream {
    match expr {
        syn::Expr::Binary(b) => {
            let left = filter_expr_to_tokens(&b.left, resource);
            let right = filter_expr_to_tokens(&b.right, resource);
            match b.op {
                syn::BinOp::Eq(_) => quote! { (#left).eq(#right) },
                syn::BinOp::Ne(_) => quote! { (#left).ne(#right) },
                syn::BinOp::Lt(_) => quote! { (#left).lt(#right) },
                syn::BinOp::Le(_) => quote! { (#left).lte(#right) },
                syn::BinOp::Gt(_) => quote! { (#left).gt(#right) },
                syn::BinOp::Ge(_) => quote! { (#left).gte(#right) },
                syn::BinOp::And(_) | syn::BinOp::BitAnd(_) => quote! { (#left) & (#right) },
                syn::BinOp::Or(_) | syn::BinOp::BitOr(_) => quote! { (#left) | (#right) },
                _ => quote! { #expr },
            }
        }
        syn::Expr::MethodCall(m) => {
            let receiver = filter_expr_to_tokens(&m.receiver, resource);
            let method = &m.method;
            let args = &m.args;
            quote! { (#receiver).#method(#args) }
        }
        syn::Expr::Path(p) if p.path.get_ident().is_some() => {
            let id = p.path.get_ident().unwrap();
            if id == "Self" {
                quote! { #resource }
            } else {
                quote! { #resource::#id }
            }
        }
        syn::Expr::Path(p) if p.path.segments.len() == 2 && p.path.segments[0].ident == "Self" => {
            let id = &p.path.segments[1].ident;
            quote! { #resource::#id }
        }
        syn::Expr::Paren(p) => {
            let inner = filter_expr_to_tokens(&p.expr, resource);
            quote! { (#inner) }
        }
        _ => quote! { #expr },
    }
}

pub fn expand_action_defs(def: &ResourceDefinition) -> Result<(Vec<TokenStream>, bool)> {
    let mut action_defs = Vec::new();
    let mut has_primary_read = false;
    let actions = &def.actions;
    let resource = &def.resource;

    for act in actions {
        let name_str = act.name.to_string();
        let method = act.kind.method_ident();
        let mut builder_chain = quote! { ::ash_core::ActionDef::#method(#name_str) };

        if !act.accept.is_empty() {
            let accept_strs: Vec<String> = act.accept.iter().map(|a| a.name.to_string()).collect();
            builder_chain = quote! { #builder_chain.accept(&[#(#accept_strs),*]) };
        }

        if !act.arguments.is_empty() {
            let arg_tokens: Vec<_> = act
                .arguments
                .iter()
                .map(|arg| {
                    let name_str = arg.name.to_string();
                    let allow_nil = arg.allow_nil;
                    let inner = option_inner(&arg.ty).unwrap_or(&arg.ty);
                    let ty_tokens = if is_uuid(inner) {
                        quote! { ::ash_core::AttrType::Uuid }
                    } else if is_string(inner) {
                        quote! { ::ash_core::AttrType::String }
                    } else if is_i64(inner) {
                        quote! { ::ash_core::AttrType::Integer }
                    } else if is_bool(inner) {
                        quote! { ::ash_core::AttrType::Boolean }
                    } else {
                        quote! { ::ash_core::AttrType::String }
                    };
                    quote! {
                        ::ash_core::ArgumentDef {
                            name: #name_str,
                            ty: #ty_tokens,
                            allow_nil: #allow_nil,
                        }
                    }
                })
                .collect();
            builder_chain = quote! { #builder_chain.arguments(&[#(#arg_tokens),*]) };
        }

        if !act.changes.is_empty() {
            let change_tokens = act
                .changes
                .iter()
                .map(|ch| match ch {
                    ChangeSpec::Set { field, value } => {
                        let field_str = field.to_string();
                        let const_val = lit_to_const_value(value)?;
                        Ok(
                            quote! { ::ash_core::Change::SetAttribute { field: #field_str, value: #const_val } },
                        )
                    }
                    ChangeSpec::SetNew { field, value } => {
                        let field_str = field.to_string();
                        let const_val = lit_to_const_value(value)?;
                        Ok(
                            quote! { ::ash_core::Change::SetNewAttribute { field: #field_str, value: #const_val } },
                        )
                    }
                    ChangeSpec::RelateActor { field } => {
                        let field_str = field.to_string();
                        Ok(quote! { ::ash_core::Change::RelateActor { field: #field_str } })
                    }
                    ChangeSpec::SetFromArg { field, argument } => {
                        let field_str = field.to_string();
                        let arg_str = argument.to_string();
                        Ok(quote! { ::ash_core::Change::SetFromArgument { field: #field_str, argument: #arg_str } })
                    }
                    ChangeSpec::ManageRelationship { relationship, rel_type } => {
                        let rel_str = relationship.to_string();
                        let type_str = rel_type.to_string().to_lowercase();
                        let type_tokens = match type_str.as_str() {
                            "create" => quote! { ::ash_core::ManagedRelType::Create },
                            "append" => quote! { ::ash_core::ManagedRelType::Append },
                            _ => quote! { ::ash_core::ManagedRelType::DirectControl },
                        };
                        Ok(quote! {
                            ::ash_core::Change::ManageRelationship {
                                relationship: #rel_str,
                                rel_type: #type_tokens,
                            }
                        })
                    }
                    ChangeSpec::BeforeAction(expr) => {
                        Ok(quote! { ::ash_core::Change::BeforeAction(#expr) })
                    }
                    ChangeSpec::AfterAction(expr) => {
                        Ok(quote! { ::ash_core::Change::AfterAction(#expr) })
                    }
                    ChangeSpec::AfterTransaction(expr) => {
                        Ok(quote! { ::ash_core::Change::AfterTransaction(#expr) })
                    }
                    ChangeSpec::Custom(expr) => {
                        Ok(quote! { ::ash_core::Change::Custom(#expr) })
                    }
                    ChangeSpec::Func(expr) => {
                        Ok(quote! { ::ash_core::Change::Func(#expr) })
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            builder_chain = quote! {
                {
                    const CHANGES: &'static [::ash_core::Change] = &[#(#change_tokens),*];
                    #builder_chain.changes(CHANGES)
                }
            };
        }

        if !act.validations.is_empty() {
            let validation_tokens = act.validations.iter().map(|v| match v {
                ValidationSpec::Present { field } => {
                    let field_str = field.to_string();
                    quote! { ::ash_core::Validation::Present { field: #field_str } }
                }
                ValidationSpec::StringLength { field, min, max } => {
                    let field_str = field.to_string();
                    let min_tokens = match min {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    let max_tokens = match max {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    quote! {
                        ::ash_core::Validation::StringLength {
                            field: #field_str,
                            min: #min_tokens,
                            max: #max_tokens,
                        }
                    }
                }
                ValidationSpec::OneOf { field, allowed } => {
                    let field_str = field.to_string();
                    quote! {
                        ::ash_core::Validation::OneOf {
                            field: #field_str,
                            allowed: &[#(#allowed),*],
                        }
                    }
                }
                ValidationSpec::Numericality { field, min, max } => {
                    let field_str = field.to_string();
                    let min_tokens = match min {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    let max_tokens = match max {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    quote! {
                        ::ash_core::Validation::Numericality {
                            field: #field_str,
                            min: #min_tokens,
                            max: #max_tokens,
                        }
                    }
                }
                ValidationSpec::Custom(expr) => {
                    quote! { ::ash_core::Validation::Custom(#expr) }
                }
                ValidationSpec::Func(expr) => {
                    quote! { ::ash_core::Validation::Func(#expr) }
                }
            });
            builder_chain = quote! {
                {
                    const VALIDATIONS: &'static [::ash_core::Validation] = &[#(#validation_tokens),*];
                    #builder_chain.validations(VALIDATIONS)
                }
            };
        }

        if !act.preparations.is_empty() {
            let mut prep_helpers = Vec::new();
            let mut prep_tokens = Vec::new();
            for (idx, prep) in act.preparations.iter().enumerate() {
                match prep {
                    PreparationSpec::Filter { expr } => {
                        let prep_fn_name = format_ident!("__prep_{}_{}_filter", act.name, idx);
                        let filter_tokens = filter_expr_to_tokens(expr, resource);
                        prep_helpers.push(quote! {
                            fn #prep_fn_name() -> ::ash_core::Filter {
                                #filter_tokens
                            }
                        });
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Filter(#prep_fn_name)
                        });
                    }
                    PreparationSpec::Sort { field, descending } => {
                        let f_str = field.to_string();
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Sort { field: #f_str, descending: #descending }
                        });
                    }
                    PreparationSpec::Limit(limit) => {
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Limit(#limit)
                        });
                    }
                    PreparationSpec::Offset(offset) => {
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Offset(#offset)
                        });
                    }
                }
            }
            builder_chain = quote! {
                {
                    #(#prep_helpers)*
                    const PREPARATIONS: &'static [::ash_core::PreparationDef] = &[#(#prep_tokens),*];
                    #builder_chain.preparations(PREPARATIONS)
                }
            };
        }

        if act.primary {
            has_primary_read = true;
            builder_chain = quote! { #builder_chain.primary() };
        }

        if act.persist_manual {
            builder_chain = quote! { #builder_chain.manual() };
        }

        action_defs.push(builder_chain);
    }

    Ok((action_defs, has_primary_read))
}

pub struct ActionCodegen {
    pub builders: Vec<TokenStream>,
    pub resource_methods: Vec<TokenStream>,
    pub trait_block: TokenStream,
}

pub fn expand_action_builders(
    def: &crate::define::ast::ResourceDefinition,
    has_primary_read: bool,
) -> ActionCodegen {
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

    if let Some(create_act) = actions.iter().find(|a| a.kind == ActionKind::Create && a.primary).or_else(|| actions.iter().find(|a| a.kind == ActionKind::Create)) {
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

    if let Some(destroy_act) = actions.iter().find(|a| a.kind == ActionKind::Destroy && a.primary).or_else(|| actions.iter().find(|a| a.kind == ActionKind::Destroy)) {
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

        match act.kind {
            ActionKind::Create => {
                let mut field_members = Vec::new();
                let mut field_inits = Vec::new();
                let mut field_setters = Vec::new();
                let mut into_fields_inserts = Vec::new();

                let all_inputs: Vec<(&syn::Ident, &syn::Type)> = act
                    .accept
                    .iter()
                    .map(|a| (&a.name, &a.ty))
                    .chain(act.arguments.iter().map(|a| (&a.name, &a.ty)))
                    .collect();

                for (name, ty) in all_inputs {
                    let s = name.to_string();
                    field_members.push(quote! { pub #name: ::std::option::Option<#ty> });
                    field_inits.push(quote! { #name: ::std::option::Option::None });
                    if let Some(inner) = option_inner(ty) {
                        field_setters.push(quote! {
                            pub fn #name(mut self, value: impl ::ash_core::IntoOption<#inner>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into_option());
                                self
                            }
                        });
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
                        field_setters.push(quote! {
                            pub fn #name(mut self, value: impl ::std::convert::Into<#ty>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into());
                                self
                            }
                        });
                        into_fields_inserts.push(quote! {
                            if let ::std::option::Option::Some(val) = &self.#name {
                                map.insert(::std::string::String::from(#s), ::ash_core::Value::from(val.clone()));
                            }
                        });
                    }
                }

                if act.persist_manual {
                    builders.push(quote! {
                        pub struct #builder_name<'a, D> {
                            ctx: &'a ::ash_core::Context<D>,
                            #(#field_members,)*
                        }

                        impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> {
                            pub fn new(ctx: &'a ::ash_core::Context<D>) -> Self {
                                Self {
                                    ctx,
                                    #(#field_inits,)*
                                }
                            }

                            #(#field_setters)*

                            pub fn into_fields(&self) -> ::ash_core::FieldMap {
                                let mut map = ::ash_core::FieldMap::new();
                                #(#into_fields_inserts)*
                                map
                            }

                            pub async fn persist<F, Fut>(self, f: F) -> ::ash_core::Result<#resource>
                            where
                                F: ::std::ops::FnOnce(&::ash_core::Context<D>, #resource) -> Fut,
                                Fut: ::std::future::Future<Output = ::ash_core::Result<#resource>>,
                            {
                                let fields = self.into_fields();
                                ::ash_core::manual_create(self.ctx, #act_name_str, fields, f).await
                            }
                        }

                        impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for #builder_name<'a, D> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }

                        impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for &'a #builder_name<'a, D> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }
                    });
                } else {
                    builders.push(quote! {
                        pub struct #builder_name<'a, D> {
                            ctx: &'a ::ash_core::Context<D>,
                            tenant_override: ::std::option::Option<::std::string::String>,
                            upsert_spec: ::std::option::Option<(&'static str, ::std::vec::Vec<::std::string::String>)>,
                            before_actions: ::std::vec::Vec<::ash_core::BeforeActionHook<#resource>>,
                            after_actions: ::std::vec::Vec<::ash_core::AfterActionHook<#resource>>,
                            after_transactions: ::std::vec::Vec<::ash_core::AfterTransactionHook<#resource>>,
                            managed_relationships: ::std::vec::Vec<::ash_core::ManagedRelationshipSpec>,
                            #(#field_members,)*
                        }

                        impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> {
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
                                }
                            }

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

                            #(#field_setters)*

                            pub fn into_fields(&self) -> ::ash_core::FieldMap {
                                let mut map = ::ash_core::FieldMap::new();
                                #(#into_fields_inserts)*
                                map
                            }

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

                        impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for #builder_name<'a, D> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }

                        impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for &'a #builder_name<'a, D> {
                            fn into_field_map(self) -> ::ash_core::FieldMap {
                                self.into_fields()
                            }
                        }

                        impl<'a, D: ::ash_core::DataLayer> ::std::future::IntoFuture for #builder_name<'a, D> {
                            type Output = ::ash_core::Result<#resource>;
                            type IntoFuture = ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = Self::Output> + ::std::marker::Send + 'a>>;

                            fn into_future(self) -> Self::IntoFuture {
                                ::std::boxed::Box::pin(self.call())
                            }
                        }

                        impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoChangeset<#resource> for #builder_name<'a, D> {
                            fn into_changeset(self) -> ::ash_core::Result<::ash_core::Changeset<#resource>> {
                                self.changeset()
                            }
                        }
                    });
                }

                let build_act_name = format_ident!("build_{}", act_name);
                resource_methods.push(quote! {
                    pub fn #act_name<'a, D: ::ash_core::DataLayer>(ctx: &'a ::ash_core::Context<D>) -> #builder_name<'a, D> {
                        #builder_name::new(ctx)
                    }

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
                let mut field_setters = Vec::new();
                let mut into_fields_inserts = Vec::new();

                let all_inputs: Vec<(&syn::Ident, &syn::Type)> = act
                    .accept
                    .iter()
                    .map(|a| (&a.name, &a.ty))
                    .chain(act.arguments.iter().map(|a| (&a.name, &a.ty)))
                    .collect();

                for (name, ty) in all_inputs {
                    let s = name.to_string();
                    field_members.push(quote! { pub #name: ::std::option::Option<#ty> });
                    field_inits.push(quote! { #name: ::std::option::Option::None });
                    if let Some(inner) = option_inner(ty) {
                        field_setters.push(quote! {
                            pub fn #name(mut self, value: impl ::ash_core::IntoOption<#inner>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into_option());
                                self
                            }
                        });
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
                        field_setters.push(quote! {
                            pub fn #name(mut self, value: impl ::std::convert::Into<#ty>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into());
                                self
                            }
                        });
                        into_fields_inserts.push(quote! {
                            if let ::std::option::Option::Some(val) = &self.#name {
                                map.insert(::std::string::String::from(#s), ::ash_core::Value::from(val.clone()));
                            }
                        });
                    }
                }

                builders.push(quote! {
                    pub enum #target_enum {
                        Id(::uuid::Uuid),
                        Existing(#resource),
                    }

                    pub struct #builder_name<'a, D> {
                        ctx: &'a ::ash_core::Context<D>,
                        target: #target_enum,
                        tenant_override: ::std::option::Option<::std::string::String>,
                        before_actions: ::std::vec::Vec<::ash_core::BeforeActionHook<#resource>>,
                        after_actions: ::std::vec::Vec<::ash_core::AfterActionHook<#resource>>,
                        after_transactions: ::std::vec::Vec<::ash_core::AfterTransactionHook<#resource>>,
                        managed_relationships: ::std::vec::Vec<::ash_core::ManagedRelationshipSpec>,
                        #(#field_members,)*
                    }

                    impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> {
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
                            }
                        }

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

                        #(#field_setters)*

                        pub fn into_fields(&self) -> ::ash_core::FieldMap {
                            let mut map = ::ash_core::FieldMap::new();
                            #(#into_fields_inserts)*
                            map
                        }

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

                    impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for #builder_name<'a, D> {
                        fn into_field_map(self) -> ::ash_core::FieldMap {
                            self.into_fields()
                        }
                    }

                    impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoFieldMap for &'a #builder_name<'a, D> {
                        fn into_field_map(self) -> ::ash_core::FieldMap {
                            self.into_fields()
                        }
                    }

                    impl<'a, D: ::ash_core::DataLayer> ::std::future::IntoFuture for #builder_name<'a, D> {
                        type Output = ::ash_core::Result<#resource>;
                        type IntoFuture = ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = Self::Output> + ::std::marker::Send + 'a>>;

                        fn into_future(self) -> Self::IntoFuture {
                            ::std::boxed::Box::pin(self.call())
                        }
                    }

                    impl<'a, D: ::ash_core::DataLayer> ::ash_core::IntoChangeset<#resource> for #builder_name<'a, D> {
                        fn into_changeset(self) -> ::ash_core::Result<::ash_core::Changeset<#resource>> {
                            self.changeset()
                        }
                    }
                });

                let act_on_name = format_ident!("{}_on", act_name);
                let build_act_name = format_ident!("build_{}", act_name);

                resource_methods.push(quote! {
                    pub fn #act_name<'a, D: ::ash_core::DataLayer>(ctx: &'a ::ash_core::Context<D>, id: ::uuid::Uuid) -> #builder_name<'a, D> {
                        #builder_name::for_id(ctx, id)
                    }

                    pub fn #act_on_name<'a, D: ::ash_core::DataLayer>(&self, ctx: &'a ::ash_core::Context<D>) -> #builder_name<'a, D> {
                        #builder_name::for_existing(ctx, self.clone())
                    }

                    pub fn #build_act_name(&self) -> #builder_name<'static, ::ash_memory::Memory> {
                        static DUMMY: ::std::sync::OnceLock<::ash_core::Context<::ash_memory::Memory>> = ::std::sync::OnceLock::new();
                        let ctx = DUMMY.get_or_init(|| ::ash_core::Context::new(::ash_memory::Memory::new()));
                        #builder_name::for_existing(ctx, self.clone())
                    }
                });

                trait_methods.push(quote! {
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
                    pub async fn #act_name<D: ::ash_core::DataLayer>(ctx: &::ash_core::Context<D>, id: ::uuid::Uuid) -> ::ash_core::Result<()> {
                        ::ash_core::destroy::<#resource, D>(ctx, #act_name_str, id).await
                    }

                    pub async fn #act_on_name<D: ::ash_core::DataLayer>(&self, ctx: &::ash_core::Context<D>) -> ::ash_core::Result<()> {
                        ::ash_core::destroy_existing::<#resource, D>(ctx, #act_name_str, self.clone()).await
                    }
                });

                trait_methods.push(quote! {
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
                let returns_ty = act
                    .returns
                    .clone()
                    .unwrap_or_else(|| syn::parse_quote!(()));

                let mut field_members = Vec::new();
                let mut field_inits = Vec::new();
                let mut field_setters = Vec::new();
                let mut input_struct_fields = Vec::new();
                let mut input_extracts = Vec::new();

                let all_inputs: Vec<(&syn::Ident, &syn::Type)> = act
                    .accept
                    .iter()
                    .map(|a| (&a.name, &a.ty))
                    .chain(act.arguments.iter().map(|a| (&a.name, &a.ty)))
                    .collect();

                for (name, ty) in all_inputs {
                    let s = name.to_string();
                    input_struct_fields.push(quote! { pub #name: #ty });
                    field_members.push(quote! { pub #name: ::std::option::Option<#ty> });
                    field_inits.push(quote! { #name: ::std::option::Option::None });

                    if let Some(inner) = option_inner(ty) {
                        field_setters.push(quote! {
                            pub fn #name(mut self, value: impl ::ash_core::IntoOption<#inner>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into_option());
                                self
                            }
                        });
                        input_extracts.push(quote! {
                            #name: self.#name.unwrap_or(::std::option::Option::None)
                        });
                    } else {
                        field_setters.push(quote! {
                            pub fn #name(mut self, value: impl ::std::convert::Into<#ty>) -> Self {
                                self.#name = ::std::option::Option::Some(value.into());
                                self
                            }
                        });
                        input_extracts.push(quote! {
                            #name: self.#name.ok_or_else(|| ::ash_core::Error::Missing { field: #s.into() })?
                        });
                    }
                }

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

                    pub struct #builder_name<'a, D> {
                        ctx: &'a ::ash_core::Context<D>,
                        #(#field_members,)*
                    }

                    impl<'a, D: ::ash_core::DataLayer> #builder_name<'a, D> {
                        pub fn new(ctx: &'a ::ash_core::Context<D>) -> Self {
                            Self {
                                ctx,
                                #(#field_inits,)*
                            }
                        }

                        #(#field_setters)*

                        fn __run_action<F, Fut>(f: F, input: #input_struct_name<'a, D>) -> Fut
                        where
                            F: ::std::ops::FnOnce(#input_struct_name<'a, D>) -> Fut,
                            Fut: ::std::future::Future<Output = ::ash_core::Result<#returns_ty>> + ::std::marker::Send,
                        {
                            f(input)
                        }

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

                    impl<'a, D: ::ash_core::DataLayer> ::std::future::IntoFuture for #builder_name<'a, D> {
                        type Output = ::ash_core::Result<#returns_ty>;
                        type IntoFuture = ::std::pin::Pin<::std::boxed::Box<dyn ::std::future::Future<Output = Self::Output> + ::std::marker::Send + 'a>>;

                        fn into_future(self) -> Self::IntoFuture {
                            ::std::boxed::Box::pin(self.call())
                        }
                    }
                });

                resource_methods.push(quote! {
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
