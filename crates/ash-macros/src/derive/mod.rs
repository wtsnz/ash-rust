pub mod ash_enum;
mod field;

use crate::ast_helpers::{lit_string, screaming_snake};
use field::{FieldSpec, Kind};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Attribute, Data, DeriveInput, Error, Fields, Ident, Meta, Result, Token};

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let table = struct_table(&input.attrs, name)?;
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(
            &input,
            "Resource can only be derived on structs",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(&input, "Resource requires named fields"));
    };

    let parsed = fields
        .named
        .iter()
        .map(FieldSpec::from_field)
        .collect::<Result<Vec<_>>>()?;

    let attr_defs = parsed.iter().filter_map(FieldSpec::attribute_def);
    let rel_defs = parsed.iter().filter_map(FieldSpec::relationship_def);
    let calc_defs = parsed.iter().filter_map(FieldSpec::calculation_def);
    let to_inserts = parsed.iter().filter_map(FieldSpec::to_field_insert);
    let from_inits = parsed.iter().map(FieldSpec::init_from_map);
    let attach_arms = parsed.iter().filter_map(FieldSpec::attach_arm);
    let field_consts = parsed.iter().map(|field| field.field_token(name));
    let pk = parsed
        .iter()
        .find(|f| matches!(f.kind, Kind::Pk))
        .ok_or_else(|| Error::new_spanned(name, "Resource needs a #[ash(pk)] Uuid field"))?;
    let pk_ident = &pk.ident;

    let def_name = format_ident!("{}_DEF", screaming_snake(&name.to_string()));
    let name_str = name.to_string();

    Ok(quote! {
        impl ::ash_core::Resource for #name {
            type Store = ::ash_core::DefaultStore;
            const DEF: ::ash_core::ResourceDef = ::ash_core::ResourceDef {
                name: #name_str,
                table: #table,
                attributes: &[#(#attr_defs),*],
                relationships: &[#(#rel_defs),*],
                actions: Self::ACTIONS,
                policies: Self::POLICIES,
                field_policies: &[],
                calculations: &[#(#calc_defs),*],
                aggregates: &[],
                extensions: &[],
                notifiers: &[],
                identities: &[],
                indexes: &[],
                checks: &[],
                statements: &[],
                embedded: false,
                data_layer: ::ash_core::DataLayerKind::Memory,
                timestamps: None,
                store_type_id: ::ash_core::default_store_type_id,
                store_name: "DefaultStore",
                multitenancy: None,
            };

            fn id(&self) -> ::uuid::Uuid {
                self.#pk_ident
            }

            fn to_fields(&self) -> ::ash_core::FieldMap {
                let mut map = ::ash_core::FieldMap::new();
                #(#to_inserts)*
                map
            }

            fn from_fields(fields: &::ash_core::FieldMap) -> ::ash_core::Result<Self> {
                Ok(Self {
                    #(#from_inits),*
                })
            }

            fn attach(
                &mut self,
                name: &str,
                related: ::std::vec::Vec<::ash_core::FieldMap>,
            ) -> ::ash_core::Result<()> {
                match name {
                    #(#attach_arms)*
                    other => Err(::ash_core::Error::Invalid(::std::format!(
                        "unknown relationship `{other}` on {}",
                        stringify!(#name)
                    ))),
                }
            }
        }

        pub const #def_name: ::ash_core::ResourceDef = <#name as ::ash_core::Resource>::DEF;

        #[allow(non_upper_case_globals)]
        pub mod fields {
            #[allow(unused_imports)]
            use super::*;
            #(#field_consts)*
        }
    })
}

fn struct_table(attrs: &[Attribute], name: &Ident) -> Result<String> {
    for attr in attrs.iter().filter(|a| a.path().is_ident("ash")) {
        for meta in attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)? {
            if let Meta::NameValue(nv) = meta
                && nv.path.is_ident("table")
            {
                return lit_string(&nv.value);
            }
        }
    }
    Ok(name.to_string())
}
