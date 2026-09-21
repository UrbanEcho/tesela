//! `#[derive(LinkType)]` derive macro implementation.

use crate::helpers::{string_value, tesela_attrs, to_snake_case, unsupported_attr};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

#[derive(Debug, Default)]
struct LinkArgs {
    api_name: Option<String>,
    from: Option<String>,
    to: Option<String>,
    cardinality: Option<String>,
    inverse: Option<String>,
    display: Option<String>,
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
    let args = parse_link_args(&input)?;

    let api_name = args.api_name.unwrap_or(fallback_api_name);
    let from = args.from.ok_or_else(|| {
        syn::Error::new(
            struct_name.span(),
            "LinkType requires #[tesela(from = \"...\")]: a link has no meaning without a source",
        )
    })?;
    let to = args.to.ok_or_else(|| {
        syn::Error::new(
            struct_name.span(),
            "LinkType requires #[tesela(to = \"...\")]: a link has no meaning without a target",
        )
    })?;
    let cardinality = match args.cardinality.as_deref().map_or("one_to_many", |v| v) {
        "one_to_one" => quote!(::tesela::__private::LinkCardinality::OneToOne),
        "one_to_many" => quote!(::tesela::__private::LinkCardinality::OneToMany),
        "many_to_many" => quote!(::tesela::__private::LinkCardinality::ManyToMany),
        other => {
            return Err(syn::Error::new_spanned(
                &input,
                format!("unknown link cardinality '{other}'"),
            ));
        }
    };
    let inverse = match args.inverse {
        Some(name) => quote!(Some(::tesela::__private::ApiName::new_unchecked(#name))),
        None => quote!(None),
    };
    let display = match args.display {
        Some(value) => quote!(Some(#value.to_string())),
        None => quote!(None),
    };

    Ok(quote! {
        impl #struct_name {
            /// Return the Tesela `LinkType` definition for this struct.
            pub fn tesela_link_type() -> ::tesela::__private::LinkType {
                ::tesela::__private::LinkType {
                    api_name: ::tesela::__private::ApiName::new_unchecked(#api_name),
                    display: #display,
                    from: ::tesela::__private::ApiName::new_unchecked(#from),
                    to: ::tesela::__private::ApiName::new_unchecked(#to),
                    cardinality: #cardinality,
                    inverse: #inverse,
                    deprecated_at: None,
                }
            }
        }
    })
}

fn parse_link_args(input: &DeriveInput) -> syn::Result<LinkArgs> {
    let mut args = LinkArgs::default();
    for attr in tesela_attrs(&input.attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                args.api_name = Some(string_value(&meta)?);
                return Ok(());
            }
            if meta.path.is_ident("from") {
                args.from = Some(string_value(&meta)?);
                return Ok(());
            }
            if meta.path.is_ident("to") {
                args.to = Some(string_value(&meta)?);
                return Ok(());
            }
            if meta.path.is_ident("cardinality") {
                args.cardinality = Some(string_value(&meta)?);
                return Ok(());
            }
            if meta.path.is_ident("inverse") {
                args.inverse = Some(string_value(&meta)?);
                return Ok(());
            }
            if meta.path.is_ident("display") {
                args.display = Some(string_value(&meta)?);
                return Ok(());
            }
            Err(unsupported_attr(&meta, "LinkType"))
        })?;
    }
    Ok(args)
}
