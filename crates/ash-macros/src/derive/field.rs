use crate::ast_helpers::{
    is_bool, is_integer, is_string, is_uuid, lit_string, option_inner, rel_inner, vec_inner,
};
use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{Attribute, Error, Field, Ident, Meta, Result, Token, Type};

pub struct FieldSpec {
    pub ident: Ident,
    pub ty: Type,
    pub kind: Kind,
}

pub enum Kind {
    Pk,
    Stored {
        optional: bool,
        atom: Option<Vec<String>>,
        is_enum: bool,
    },
    Calc {
        source: String,
    },
    BelongsTo {
        dest: Type,
        fk: String,
    },
    HasMany {
        dest: Type,
        fk: String,
    },
}

impl FieldSpec {
    pub fn from_field(field: &Field) -> Result<Self> {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| Error::new_spanned(field, "unnamed field"))?;
        let args = AshArgs::from_attrs(&field.attrs)?;
        let ty = field.ty.clone();

        let kind = if args.pk {
            Kind::Pk
        } else if let Some(source) = args.calc {
            Kind::Calc { source }
        } else if args.belongs_to {
            let dest = option_inner(rel_inner(&ty)?)
                .cloned()
                .ok_or_else(|| Error::new_spanned(&ty, "belongs_to expects Rel<Option<T>>"))?;
            let fk = args.fk.unwrap_or_else(|| format!("{ident}_id"));
            Kind::BelongsTo { dest, fk }
        } else if args.has_many {
            let dest = vec_inner(rel_inner(&ty)?)
                .cloned()
                .ok_or_else(|| Error::new_spanned(&ty, "has_many expects Rel<Vec<T>>"))?;
            let fk = args
                .fk
                .ok_or_else(|| Error::new_spanned(&ident, "has_many requires fk = \"…_id\""))?;
            Kind::HasMany { dest, fk }
        } else {
            Kind::Stored {
                optional: option_inner(&ty).is_some(),
                atom: args.atom,
                is_enum: args.is_enum,
            }
        };

        Ok(Self { ident, ty, kind })
    }

    pub fn attribute_def(&self) -> Option<TokenStream> {
        let name = self.ident.to_string();
        match &self.kind {
            Kind::Pk => Some(quote! { ::ash_core::AttributeDef::uuid_pk(#name) }),
            Kind::Stored {
                optional,
                atom,
                is_enum,
            } => {
                let ty = if *is_enum {
                    let inner = option_inner(&self.ty).unwrap_or(&self.ty);
                    quote! { <#inner as ::ash_core::AshType>::ATTR_TYPE }
                } else if let Some(atoms) = atom {
                    quote! { ::ash_core::AttrType::Atom { one_of: &[#(#atoms),*] } }
                } else if is_uuid(&self.ty) || option_inner(&self.ty).is_some_and(is_uuid) {
                    quote! { ::ash_core::AttrType::Uuid }
                } else if is_integer(&self.ty) || option_inner(&self.ty).is_some_and(is_integer) {
                    quote! { ::ash_core::AttrType::Integer }
                } else if is_bool(&self.ty) {
                    quote! { ::ash_core::AttrType::Boolean }
                } else {
                    quote! { ::ash_core::AttrType::String }
                };
                if *optional {
                    Some(quote! { ::ash_core::AttributeDef::optional(#name, #ty) })
                } else {
                    Some(quote! { ::ash_core::AttributeDef::required(#name, #ty) })
                }
            }
            Kind::Calc { .. } | Kind::BelongsTo { .. } | Kind::HasMany { .. } => None,
        }
    }

    pub fn relationship_def(&self) -> Option<TokenStream> {
        let name = self.ident.to_string();
        match &self.kind {
            Kind::BelongsTo { dest, fk } => Some(quote! {
                ::ash_core::RelationshipDef::belongs_to(
                    #name,
                    || &<#dest as ::ash_core::Resource>::DEF,
                    #fk,
                )
            }),
            Kind::HasMany { dest, fk } => Some(quote! {
                ::ash_core::RelationshipDef::has_many(
                    #name,
                    || &<#dest as ::ash_core::Resource>::DEF,
                    #fk,
                )
            }),
            _ => None,
        }
    }

    pub fn calculation_def(&self) -> Option<TokenStream> {
        let name = self.ident.to_string();
        match &self.kind {
            Kind::Calc { source } => Some(quote! {
                ::ash_core::CalculationDef::new(
                    #name,
                    ::ash_core::AttrType::Integer,
                    ::ash_core::Expr::StringLength(#source),
                )
            }),
            _ => None,
        }
    }

    pub fn to_field_insert(&self) -> Option<TokenStream> {
        let ident = &self.ident;
        let name = ident.to_string();
        match &self.kind {
            Kind::Stored {
                is_enum: true,
                optional: true,
                ..
            } => Some(quote! {
                if let ::std::option::Option::Some(ref val) = self.#ident {
                    map.insert(
                        ::std::string::String::from(#name),
                        ::ash_core::AshType::to_value(val),
                    );
                } else {
                    map.insert(
                        ::std::string::String::from(#name),
                        ::ash_core::Value::Null,
                    );
                }
            }),
            Kind::Stored {
                is_enum: true,
                optional: false,
                ..
            } => Some(quote! {
                map.insert(
                    ::std::string::String::from(#name),
                    ::ash_core::AshType::to_value(&self.#ident),
                );
            }),
            Kind::Stored { atom: Some(_), .. } => Some(quote! {
                map.insert(
                    ::std::string::String::from(#name),
                    ::ash_core::Value::from(self.#ident.as_str()),
                );
            }),
            Kind::Pk | Kind::Stored { .. } if is_string(&self.ty) => Some(quote! {
                map.insert(
                    ::std::string::String::from(#name),
                    ::ash_core::Value::from(self.#ident.clone()),
                );
            }),
            Kind::Pk | Kind::Stored { .. } => Some(quote! {
                map.insert(
                    ::std::string::String::from(#name),
                    ::ash_core::Value::from(self.#ident),
                );
            }),
            Kind::Calc { .. } | Kind::BelongsTo { .. } | Kind::HasMany { .. } => None,
        }
    }

    pub fn init_from_map(&self) -> TokenStream {
        let ident = &self.ident;
        let name = ident.to_string();
        let ty = &self.ty;
        match &self.kind {
            Kind::Pk => quote! {
                #ident: ::ash_core::required_uuid(fields, #name)?
            },
            Kind::Stored {
                is_enum: true,
                optional: true,
                ..
            } => {
                let inner = option_inner(&self.ty).unwrap_or(&self.ty);
                quote! {
                    #ident: match fields.get(#name) {
                        ::std::option::Option::Some(val) if !val.is_null() => {
                            ::std::option::Option::Some(<#inner as ::ash_core::AshType>::from_value(val)?)
                        }
                        _ => ::std::option::Option::None,
                    }
                }
            }
            Kind::Stored {
                is_enum: true,
                optional: false,
                ..
            } => {
                quote! {
                    #ident: match fields.get(#name) {
                        ::std::option::Option::Some(val) if !val.is_null() => {
                            <#ty as ::ash_core::AshType>::from_value(val)?
                        }
                        _ => return Err(::ash_core::Error::Missing { field: #name.into() }),
                    }
                }
            }
            Kind::Stored {
                optional: false,
                atom: None,
                ..
            } if is_uuid(&self.ty) => quote! {
                #ident: ::ash_core::required_uuid(fields, #name)?
            },
            Kind::Stored {
                optional: true,
                atom: None,
                ..
            } if option_inner(&self.ty).is_some_and(is_uuid) => {
                quote! { #ident: ::ash_core::optional_uuid(fields, #name)? }
            }
            Kind::Stored {
                optional: false,
                atom: None,
                ..
            } if is_string(&self.ty) => quote! {
                #ident: ::ash_core::required_string(fields, #name)?
            },
            Kind::Stored {
                optional: false,
                atom: None,
                ..
            } if is_integer(&self.ty) => {
                let ty = &self.ty;
                quote! {
                    #ident: {
                        let n = ::ash_core::optional_int(fields, #name)?
                            .ok_or_else(|| ::ash_core::Error::Missing { field: #name.into() })?;
                        <#ty as ::std::convert::TryFrom<i64>>::try_from(n).map_err(|_| {
                            ::ash_core::Error::Invalid(::std::format!(
                                "integer does not fit in {}",
                                stringify!(#ty)
                            ))
                        })?
                    }
                }
            }
            Kind::Stored {
                optional: true,
                atom: None,
                ..
            } if option_inner(&self.ty).is_some_and(is_integer) => {
                let inner = option_inner(&self.ty).unwrap();
                quote! {
                    #ident: match ::ash_core::optional_int(fields, #name)? {
                        ::std::option::Option::Some(n) => ::std::option::Option::Some(
                            <#inner as ::std::convert::TryFrom<i64>>::try_from(n).map_err(|_| {
                                ::ash_core::Error::Invalid(::std::format!(
                                    "integer does not fit in {}",
                                    stringify!(#inner)
                                ))
                            })?
                        ),
                        ::std::option::Option::None => ::std::option::Option::None,
                    }
                }
            }
            Kind::Stored {
                optional: false,
                atom: Some(_),
                ..
            } => quote! {
                #ident: #ty::parse(&::ash_core::required_string(fields, #name)?)?
            },
            Kind::Calc { .. } => quote! {
                #ident: ::ash_core::optional_int(fields, #name)?
            },
            Kind::BelongsTo { .. } | Kind::HasMany { .. } => quote! {
                #ident: ::ash_core::Rel::NotLoaded
            },
            Kind::Stored { .. } => quote! {
                compile_error!(concat!("unsupported stored field type for ", #name))
            },
        }
    }

    pub fn attach_arm(&self) -> Option<TokenStream> {
        let ident = &self.ident;
        let name = ident.to_string();
        match &self.kind {
            Kind::BelongsTo { dest, .. } => Some(quote! {
                #name => {
                    self.#ident = ::ash_core::Rel::Loaded(match related.first() {
                        Some(row) => Some(<#dest as ::ash_core::Resource>::from_fields(row)?),
                        None => None,
                    });
                    Ok(())
                }
            }),
            Kind::HasMany { dest, .. } => Some(quote! {
                #name => {
                    self.#ident = ::ash_core::Rel::Loaded(
                        related
                            .iter()
                            .map(<#dest as ::ash_core::Resource>::from_fields)
                            .collect::<::ash_core::Result<::std::vec::Vec<_>>>()?,
                    );
                    Ok(())
                }
            }),
            _ => None,
        }
    }

    pub fn field_token(&self, owner: &Ident) -> TokenStream {
        let ident = &self.ident;
        let name = ident.to_string();
        match &self.kind {
            Kind::Pk => quote! {
                pub const #ident: ::ash_core::Attr<super::#owner, ::uuid::Uuid> =
                    ::ash_core::Attr::new(#name);
            },
            Kind::Stored { is_enum: true, .. } => {
                let inner = option_inner(&self.ty).unwrap_or(&self.ty);
                quote! {
                    pub const #ident: ::ash_core::Attr<super::#owner, #inner> =
                        ::ash_core::Attr::new(#name);
                }
            }
            Kind::Stored { atom: Some(_), .. } => quote! {
                pub const #ident: ::ash_core::Attr<super::#owner, ::std::string::String> =
                    ::ash_core::Attr::new(#name);
            },
            Kind::Stored { .. }
                if is_uuid(&self.ty) || option_inner(&self.ty).is_some_and(is_uuid) =>
            {
                quote! {
                    pub const #ident: ::ash_core::Attr<super::#owner, ::uuid::Uuid> =
                        ::ash_core::Attr::new(#name);
                }
            }
            Kind::Stored { .. } if is_string(&self.ty) => quote! {
                pub const #ident: ::ash_core::Attr<super::#owner, ::std::string::String> =
                    ::ash_core::Attr::new(#name);
            },
            Kind::Stored { .. }
                if is_integer(&self.ty) || option_inner(&self.ty).is_some_and(is_integer) =>
            {
                let inner = option_inner(&self.ty).unwrap_or(&self.ty);
                quote! {
                    pub const #ident: ::ash_core::Attr<super::#owner, #inner> =
                        ::ash_core::Attr::new(#name);
                }
            }
            Kind::Calc { .. } => quote! {
                pub const #ident: ::ash_core::Calc<super::#owner, i64> =
                    ::ash_core::Calc::new(#name);
            },
            Kind::BelongsTo { dest, .. } | Kind::HasMany { dest, .. } => quote! {
                pub const #ident: ::ash_core::Relation<super::#owner, #dest> =
                    ::ash_core::Relation::new(#name);
            },
            Kind::Stored { .. } => quote! {
                compile_error!(concat!("unsupported fields token for ", #name));
            },
        }
    }
}

pub struct AshArgs {
    pub pk: bool,
    pub belongs_to: bool,
    pub has_many: bool,
    pub calc: Option<String>,
    pub fk: Option<String>,
    pub atom: Option<Vec<String>>,
    pub is_enum: bool,
}

impl AshArgs {
    pub fn from_attrs(attrs: &[Attribute]) -> Result<Self> {
        let mut args = Self {
            pk: false,
            belongs_to: false,
            has_many: false,
            calc: None,
            fk: None,
            atom: None,
            is_enum: false,
        };
        for attr in attrs.iter().filter(|a| a.path().is_ident("ash")) {
            for meta in attr.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)? {
                match meta {
                    Meta::Path(path) if path.is_ident("pk") => args.pk = true,
                    Meta::Path(path) if path.is_ident("belongs_to") => args.belongs_to = true,
                    Meta::Path(path) if path.is_ident("has_many") => args.has_many = true,
                    Meta::Path(path) if path.is_ident("enum") || path.is_ident("ash_enum") => {
                        args.is_enum = true;
                    }
                    Meta::NameValue(nv) if nv.path.is_ident("fk") => {
                        args.fk = Some(lit_string(&nv.value)?);
                    }
                    Meta::NameValue(nv) if nv.path.is_ident("atom") => {
                        args.atom = Some(
                            lit_string(&nv.value)?
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect(),
                        );
                    }
                    Meta::NameValue(nv) if nv.path.is_ident("calc") => {
                        args.calc = Some(parse_string_length(&lit_string(&nv.value)?)?);
                    }
                    other => {
                        return Err(Error::new_spanned(other, "unknown ash attribute"));
                    }
                }
            }
        }
        Ok(args)
    }
}

fn parse_string_length(spec: &str) -> Result<String> {
    let spec = spec.trim();
    let Some(inner) = spec
        .strip_prefix("string_length(")
        .and_then(|s| s.strip_suffix(')'))
    else {
        return Err(Error::new(
            proc_macro2::Span::call_site(),
            "calc currently supports string_length(field)",
        ));
    };
    Ok(inner.trim().to_string())
}
