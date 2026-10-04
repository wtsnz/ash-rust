//! `#[derive(AshTypedMap)]` and `#[derive(AshUnion)]`: a struct as a map of declared
//! fields (Ash's `:map` with `fields` constraints), an enum of one-field variants as a
//! union of typed members (Ash's `Ash.Type.Union`).

use crate::ast_helpers::{option_inner, snake_case};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Data, DeriveInput, Error, Fields, Lit, Result};

/// The name `#[ash(rename = "...")]` gives, if any.
fn renamed(attrs: &[Attribute]) -> Result<Option<String>> {
    let mut name = None;
    for attr in attrs.iter().filter(|attr| attr.path().is_ident("ash")) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                match meta.value()?.parse::<Lit>()? {
                    Lit::Str(s) => {
                        name = Some(s.value());
                        Ok(())
                    }
                    _ => Err(meta.error("expected a string")),
                }
            } else {
                Err(meta.error("expected `rename`"))
            }
        })?;
    }
    Ok(name)
}

/// Conversions from the type to a `Value`, as every Ash type has them.
fn value_conversions(name: &syn::Ident) -> TokenStream {
    quote! {
        impl ::std::convert::From<#name> for ::ash_core::Value {
            fn from(value: #name) -> Self {
                ::ash_core::AshType::to_value(&value)
            }
        }

        impl ::std::convert::From<&#name> for ::ash_core::Value {
            fn from(value: &#name) -> Self {
                ::ash_core::AshType::to_value(value)
            }
        }
    }
}

pub fn expand_typed_map(input: DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(Error::new_spanned(&input, "AshTypedMap can only be derived on structs"));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(Error::new_spanned(&input, "AshTypedMap needs named fields"));
    };
    let mut defs = Vec::new();
    let mut to_value = Vec::new();
    let mut from_value = Vec::new();
    for field in &fields.named {
        let ident = field.ident.as_ref().expect("named");
        let ty = &field.ty;
        let key = renamed(&field.attrs)?.unwrap_or_else(|| ident.to_string());
        let allow_nil = option_inner(ty).is_some();
        defs.push(quote! {
            ::ash_core::MapField { name: #key, ty: <#ty as ::ash_core::AshType>::ATTR_TYPE, allow_nil: #allow_nil }
        });
        to_value.push(quote! {
            map.insert(::std::string::String::from(#key), ::ash_core::AshType::to_value(&self.#ident));
        });
        from_value.push(quote! {
            #ident: <#ty as ::ash_core::AshType>::from_value(map.get(#key).unwrap_or(&::ash_core::Value::Null))?,
        });
    }
    let conversions = value_conversions(name);
    Ok(quote! {
        impl ::ash_core::AshType for #name {
            const ATTR_TYPE: ::ash_core::AttrType = ::ash_core::AttrType::TypedMap(&[#(#defs),*]);

            fn to_value(&self) -> ::ash_core::Value {
                let mut map = ::ash_core::FieldMap::new();
                #(#to_value)*
                ::ash_core::Value::Map(map)
            }

            fn from_value(value: &::ash_core::Value) -> ::ash_core::Result<Self> {
                match value {
                    ::ash_core::Value::Map(map) => ::std::result::Result::Ok(Self { #(#from_value)* }),
                    other => ::std::result::Result::Err(::ash_core::Error::Invalid(
                        ::std::format!("expected a map for {}, got {other}", stringify!(#name))
                    )),
                }
            }
        }

        #conversions
    })
}

pub fn expand_union(input: DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(&input, "AshUnion can only be derived on enums"));
    };
    let mut members = Vec::new();
    let mut to_value = Vec::new();
    let mut from_value = Vec::new();
    for variant in &data.variants {
        let ident = &variant.ident;
        let Fields::Unnamed(fields) = &variant.fields else {
            return Err(Error::new_spanned(variant, "an AshUnion member holds one value: `Member(Type)`"));
        };
        if fields.unnamed.len() != 1 {
            return Err(Error::new_spanned(variant, "an AshUnion member holds one value: `Member(Type)`"));
        }
        let ty = &fields.unnamed[0].ty;
        let member = renamed(&variant.attrs)?.unwrap_or_else(|| snake_case(&ident.to_string()));
        members.push(quote! {
            ::ash_core::UnionMember { name: #member, ty: <#ty as ::ash_core::AshType>::ATTR_TYPE }
        });
        to_value.push(quote! {
            Self::#ident(value) => ::ash_core::union_value(#member, ::ash_core::AshType::to_value(value)),
        });
        from_value.push(quote! {
            ::std::option::Option::Some((#member, value)) => ::std::result::Result::Ok(Self::#ident(<#ty as ::ash_core::AshType>::from_value(value)?)),
        });
    }
    let conversions = value_conversions(name);
    Ok(quote! {
        impl ::ash_core::AshType for #name {
            const ATTR_TYPE: ::ash_core::AttrType = ::ash_core::AttrType::Union(&[#(#members),*]);

            fn to_value(&self) -> ::ash_core::Value {
                match self {
                    #(#to_value)*
                }
            }

            fn from_value(value: &::ash_core::Value) -> ::ash_core::Result<Self> {
                match value.union_member() {
                    #(#from_value)*
                    _ => ::std::result::Result::Err(::ash_core::Error::Invalid(
                        ::std::format!("expected a member of {}, got {value}", stringify!(#name))
                    )),
                }
            }
        }

        #conversions
    })
}
