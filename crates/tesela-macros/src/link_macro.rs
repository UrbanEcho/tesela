//! `#[derive(Link)]` implementation.

use crate::helpers::{tesela_attrs, to_snake_case, unsupported_attr};
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, LitStr, Type, parse_macro_input};

#[derive(Default)]
struct Args {
    from: Option<Type>,
    to: Option<Type>,
    cardinality: Option<Ident>,
    inverse: Option<Type>,
    name: Option<LitStr>,
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_checked(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
fn expand_checked(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let ident = &input.ident;
    let args = parse_args(&input)?;
    let from = args
        .from
        .ok_or_else(|| syn::Error::new(ident.span(), "Link requires from = Object"))?;
    let to = args
        .to
        .ok_or_else(|| syn::Error::new(ident.span(), "Link requires to = Object"))?;
    let api_name = args
        .name
        .map_or_else(|| to_snake_case(&ident.to_string()), |value| value.value());
    let cardinality = match args
        .cardinality
        .as_ref()
        .map(Ident::to_string)
        .as_deref()
        .unwrap_or("many")
    {
        "one" => quote!(::tesela::__private::LinkCardinality::OneToOne),
        "many" => quote!(::tesela::__private::LinkCardinality::OneToMany),
        "many_to_many" => quote!(::tesela::__private::LinkCardinality::ManyToMany),
        other => {
            return Err(syn::Error::new(
                ident.span(),
                format!("unknown cardinality '{other}'"),
            ));
        }
    };
    let inverse = args.inverse.map_or_else(
        || quote!(None),
        |inverse| quote!(Some(<#inverse as ::tesela::Link>::definition().api_name)),
    );
    Ok(quote! {
        impl ::tesela::Link for #ident {
            type Source = #from;
            type Target = #to;
            fn definition() -> ::tesela::__private::LinkType {
                ::tesela::__private::LinkType { api_name: ::tesela::__private::ApiName::new_unchecked(#api_name), display: None, from: <#from as ::tesela::Object>::api_name(), to: <#to as ::tesela::Object>::api_name(), cardinality: #cardinality, inverse: #inverse, deprecated_at: None }
            }
        }
    })
}
fn parse_args(input: &DeriveInput) -> syn::Result<Args> {
    let mut args = Args::default();
    for attr in tesela_attrs(&input.attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("from") {
                args.from = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("to") {
                args.to = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("cardinality") {
                args.cardinality = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("inverse") {
                args.inverse = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("name") {
                args.name = Some(meta.value()?.parse()?);
                return Ok(());
            }
            Err(unsupported_attr(&meta, "Link"))
        })?;
    }
    Ok(args)
}
