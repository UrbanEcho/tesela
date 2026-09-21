//! `#[derive(ObjectType)]` derive macro implementation.

use crate::helpers::{
    is_option, parse_field_args, rust_type_to_data_type, tesela_attrs, to_snake_case,
    type_to_string, unsupported_attr,
};
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as TokenStream2};
use quote::quote;
use syn::spanned::Spanned;
use syn::{Attribute, DeriveInput, Expr, LitStr, Result, parse_macro_input};

#[derive(Debug, Default)]
struct ObjectTypeArgs {
    api_name: Option<Expr>,
    primary_key: Option<Expr>,
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

fn expand_checked(input: DeriveInput) -> Result<TokenStream2> {
    let struct_name = &input.ident;
    let struct_name_str = struct_name.to_string();
    let fallback_api_name = to_snake_case(&struct_name_str);
    let object_args = parse_object_args(&input.attrs)?;

    let primary_key = object_args.primary_key.ok_or_else(|| {
        syn::Error::new(
            struct_name.span(),
            "ObjectType requires #[tesela(primary_key = ...)]: a record's identity is part of its meaning, so it cannot be inferred",
        )
    })?;
    let api_name = api_name_tokens(object_args.api_name, &fallback_api_name);
    let primary_key = api_name_tokens(Some(primary_key), "");
    let display = option_lit_tokens(object_args.display);
    let description = option_lit_tokens(object_args.description);

    let fields = named_fields(&input)?;
    let mut property_builders = Vec::with_capacity(fields.len());
    let mut field_descriptors = Vec::with_capacity(fields.len());

    for field in fields {
        let field_name = match &field.ident {
            Some(ident) => ident.to_string(),
            None => {
                return Err(syn::Error::new(
                    field.span(),
                    "ObjectType requires named fields",
                ));
            }
        };
        let field_ident = field.ident.as_ref().expect("named field");
        let field_type = &field.ty;
        let ty_str = type_to_string(&field.ty);
        let field_args = parse_field_args(&field.attrs)?;
        let data_type = match &field_args.data_type {
            Some(data_type) => quote!(#data_type),
            None => rust_type_to_data_type(&ty_str),
        };
        let field_nullable = field_args.nullable || is_option(&ty_str);
        let field_description = option_lit_tokens(field_args.description);

        property_builders.push(quote! {
            ::tesela::__private::StaticProperty {
                api_name: #field_name,
                display: None,
                description: #field_description,
                data_type: #data_type,
                nullable: #field_nullable,
            }
        });
        field_descriptors.push(quote! {
            /// Typed semantic field descriptor.
            pub fn #field_ident() -> ::tesela::query::Field<#field_type> {
                ::tesela::query::Field::new(#field_name)
            }
        });
    }

    Ok(quote! {
        impl ::tesela::__private::ObjectTypeDefinition for #struct_name {
            fn definition() -> ::tesela::__private::StaticObjectType {
                const PROPERTIES: &[::tesela::__private::StaticProperty] = &[#(#property_builders),*];

                ::tesela::__private::StaticObjectType {
                    api_name: #api_name,
                    display: #display,
                    description: #description,
                    primary_key: #primary_key,
                    properties: PROPERTIES,
                    traits: &[],
                    tags: &[],
                }
            }
        }

        impl #struct_name {
            #(#field_descriptors)*
            /// Return the Tesela `ObjectType` definition for this struct.
            pub fn tesela_object_type() -> ::tesela::__private::ObjectType {
                <Self as ::tesela::__private::ObjectTypeDefinition>::object_type()
            }
        }
    })
}

fn named_fields(input: &DeriveInput) -> Result<Vec<&syn::Field>> {
    match &input.data {
        syn::Data::Struct(ds) => match &ds.fields {
            syn::Fields::Named(named) => Ok(named.named.iter().collect()),
            _ => Err(syn::Error::new(
                Span::call_site(),
                "ObjectType requires a struct with named fields",
            )),
        },
        _ => Err(syn::Error::new(
            Span::call_site(),
            "ObjectType can only be applied to structs",
        )),
    }
}

fn parse_object_args(attrs: &[Attribute]) -> Result<ObjectTypeArgs> {
    let mut args = ObjectTypeArgs::default();
    for attr in tesela_attrs(attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                args.api_name = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("primary_key") {
                args.primary_key = Some(meta.value()?.parse()?);
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
            Err(unsupported_attr(&meta, "ObjectType"))
        })?;
    }
    Ok(args)
}

fn api_name_tokens(value: Option<Expr>, fallback: &str) -> TokenStream2 {
    match value {
        Some(expr) => quote!(::tesela::__private::ApiNameSource::api_name(&(#expr))),
        None => quote!(#fallback),
    }
}

fn option_lit_tokens(value: Option<LitStr>) -> TokenStream2 {
    match value {
        Some(value) => quote!(Some(#value)),
        None => quote!(None),
    }
}
