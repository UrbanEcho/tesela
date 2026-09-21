//! `#[derive(TraitDef)]` derive macro implementation.

use crate::helpers::{
    is_option, parse_field_args, rust_type_to_data_type, string_value, tesela_attrs, to_snake_case,
    type_to_string, unsupported_attr,
};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::spanned::Spanned;
use syn::{DeriveInput, LitStr, parse_macro_input};

#[derive(Debug, Default)]
struct TraitArgs {
    api_name: Option<String>,
    display: Option<LitStr>,
    description: Option<LitStr>,
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_checked(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_checked(input: DeriveInput) -> syn::Result<TokenStream2> {
    let struct_name = &input.ident;
    let fallback_api_name = to_snake_case(&struct_name.to_string());
    let args = parse_trait_args(&input)?;
    let api_name = args.api_name.unwrap_or(fallback_api_name);
    let display = match args.display {
        Some(value) => quote!(Some(#value.to_string())),
        None => quote!(None),
    };
    let description = match args.description {
        Some(value) => quote!(Some(#value.to_string())),
        None => quote!(None),
    };

    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(named) => named.named.iter().collect::<Vec<_>>(),
            _ => {
                return Err(syn::Error::new(
                    struct_name.span(),
                    "TraitDef requires a struct with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                struct_name.span(),
                "TraitDef can only be applied to structs",
            ));
        }
    };

    let mut property_builders = Vec::with_capacity(fields.len());
    for field in fields {
        let field_name = field
            .ident
            .as_ref()
            .ok_or_else(|| syn::Error::new(field.span(), "TraitDef requires named fields"))?
            .to_string();
        let ty_str = type_to_string(&field.ty);
        let field_args = parse_field_args(&field.attrs)?;
        let data_type = match &field_args.data_type {
            Some(data_type) => quote!(#data_type),
            None => rust_type_to_data_type(&ty_str),
        };
        let field_nullable = field_args.nullable || is_option(&ty_str);
        let field_description = match field_args.description {
            Some(value) => quote!(Some(#value.to_string())),
            None => quote!(None),
        };

        property_builders.push(quote! {
            ::tesela::__private::Property {
                api_name: ::tesela::__private::ApiName::new_unchecked(#field_name),
                display: None,
                description: #field_description,
                data_type: #data_type,
                nullable: if #field_nullable { Some(true) } else { None },
                default: None,
                allowed_values: None,
                tags: Vec::new(),
            }
        });
    }

    Ok(quote! {
        impl #struct_name {
            /// Return the Tesela `Trait` definition for this struct.
            pub fn tesela_trait() -> ::tesela::__private::Trait {
                ::tesela::__private::Trait {
                    api_name: ::tesela::__private::ApiName::new_unchecked(#api_name),
                    display: #display,
                    description: #description,
                    properties: vec![ #(#property_builders),* ],
                }
            }
        }
    })
}

fn parse_trait_args(input: &DeriveInput) -> syn::Result<TraitArgs> {
    let mut args = TraitArgs::default();
    for attr in tesela_attrs(&input.attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                args.api_name = Some(string_value(&meta)?);
                return Ok(());
            }
            if meta.path.is_ident("display") {
                args.display = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("description") {
                args.description = Some(meta.value()?.parse()?);
                return Ok(());
            }
            Err(unsupported_attr(&meta, "TraitDef"))
        })?;
    }
    Ok(args)
}
