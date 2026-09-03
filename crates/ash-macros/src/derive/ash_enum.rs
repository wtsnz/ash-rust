use crate::ast_helpers::snake_case;
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Error, Fields, Lit, Result};

pub fn expand_ash_enum(input: DeriveInput) -> Result<TokenStream> {
    let name = &input.ident;
    let Data::Enum(data) = &input.data else {
        return Err(Error::new_spanned(
            &input,
            "AshEnum can only be derived on enums",
        ));
    };

    let mut variant_strs = Vec::new();
    let mut as_str_arms = Vec::new();
    let mut parse_arms = Vec::new();

    for variant in &data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(Error::new_spanned(
                variant,
                "AshEnum only supports unit variants without fields",
            ));
        }

        let var_ident = &variant.ident;
        let mut string_rep = snake_case(&var_ident.to_string());

        for attr in &variant.attrs {
            if attr.path().is_ident("ash") {
                attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("string") || meta.path.is_ident("rename") {
                        let value = meta.value()?;
                        let lit: Lit = value.parse()?;
                        if let Lit::Str(s) = lit {
                            string_rep = s.value();
                            Ok(())
                        } else {
                            Err(meta.error("expected string literal"))
                        }
                    } else {
                        Err(meta.error("expected `string` or `rename`"))
                    }
                })?;
            }
        }

        variant_strs.push(string_rep.clone());
        as_str_arms.push(quote! {
            Self::#var_ident => #string_rep,
        });
        parse_arms.push(quote! {
            #string_rep => ::std::result::Result::Ok(Self::#var_ident),
        });
    }

    Ok(quote! {
        impl ::ash_core::AshEnum for #name {
            const VARIANTS: &'static [&'static str] = &[#(#variant_strs),*];

            fn as_str(&self) -> &'static str {
                match self {
                    #(#as_str_arms)*
                }
            }

            fn parse(s: &str) -> ::ash_core::Result<Self> {
                match s {
                    #(#parse_arms)*
                    other => ::std::result::Result::Err(::ash_core::Error::Invalid(
                        ::std::format!("unknown variant `{}` for enum {}", other, stringify!(#name))
                    )),
                }
            }
        }

        impl ::ash_core::AshType for #name {
            const ATTR_TYPE: ::ash_core::AttrType = ::ash_core::AttrType::Atom {
                one_of: <Self as ::ash_core::AshEnum>::VARIANTS,
            };

            fn to_value(&self) -> ::ash_core::Value {
                ::ash_core::Value::String(::ash_core::AshEnum::as_str(self).to_string())
            }

            fn from_value(value: &::ash_core::Value) -> ::ash_core::Result<Self> {
                match value {
                    ::ash_core::Value::String(s) => <Self as ::ash_core::AshEnum>::parse(s),
                    other => ::std::result::Result::Err(::ash_core::Error::Invalid(
                        ::std::format!("expected string value for enum {}, got {:?}", stringify!(#name), other)
                    )),
                }
            }
        }

        impl ::std::fmt::Display for #name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}", ::ash_core::AshEnum::as_str(self))
            }
        }

        impl ::std::str::FromStr for #name {
            type Err = ::ash_core::Error;
            fn from_str(s: &str) -> ::ash_core::Result<Self> {
                <Self as ::ash_core::AshEnum>::parse(s)
            }
        }

        impl<'a> ::std::convert::TryFrom<&'a str> for #name {
            type Error = ::ash_core::Error;
            fn try_from(s: &'a str) -> ::ash_core::Result<Self> {
                <Self as ::ash_core::AshEnum>::parse(s)
            }
        }

        impl ::std::convert::From<#name> for ::ash_core::Value {
            fn from(val: #name) -> Self {
                ::ash_core::AshType::to_value(&val)
            }
        }

        impl ::std::convert::From<&#name> for ::ash_core::Value {
            fn from(val: &#name) -> Self {
                ::ash_core::AshType::to_value(val)
            }
        }

        impl ::ash_core::IntoOption<#name> for #name {
            fn into_option(self) -> ::std::option::Option<#name> {
                ::std::option::Option::Some(self)
            }
        }

        impl<'a> ::ash_core::IntoOption<#name> for &'a str {
            fn into_option(self) -> ::std::option::Option<#name> {
                <#name as ::ash_core::AshEnum>::parse(self).ok()
            }
        }
    })
}
