//! `#[derive(Object)]` implementation.

use crate::helpers::{
    is_option, parse_field_args, rust_type_to_data_type, tesela_attrs, to_snake_case,
    type_to_string, unsupported_attr,
};
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{DeriveInput, Ident, LitStr, parse_macro_input};

#[derive(Default)]
struct Args {
    key: Option<Ident>,
    name: Option<LitStr>,
    display: Option<LitStr>,
    description: Option<LitStr>,
}

pub(crate) fn expand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_checked(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_checked(input: DeriveInput) -> syn::Result<TokenStream2> {
    let name = &input.ident;
    let args = parse_args(&input)?;
    let key = args
        .key
        .ok_or_else(|| syn::Error::new(name.span(), "Object requires #[tesela(key = field)]"))?;
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => fields.named.iter().collect::<Vec<_>>(),
            _ => return Err(syn::Error::new(name.span(), "Object requires named fields")),
        },
        _ => {
            return Err(syn::Error::new(
                name.span(),
                "Object can only be derived for a struct",
            ));
        }
    };
    let key_field = fields
        .iter()
        .find(|field| field.ident.as_ref() == Some(&key))
        .ok_or_else(|| syn::Error::new(key.span(), "key does not name a struct field"))?;
    let key_type = &key_field.ty;
    let api_name = args
        .name
        .map_or_else(|| to_snake_case(&name.to_string()), |value| value.value());
    let display = option_string(args.display);
    let description = option_string(args.description);
    let key_name = key.to_string();
    let mut properties = Vec::new();
    let mut accessors = Vec::new();
    let mut encoders = Vec::new();
    let mut decoders = Vec::new();
    let mut initializers = Vec::new();
    let mut lazy_fields = Vec::new();
    for field in fields {
        let ident = field.ident.as_ref().expect("named field");
        let field_name = ident.to_string();
        let ty = &field.ty;
        let field_args = parse_field_args(&field.attrs)?;
        let ty_string = type_to_string(ty);
        let data_type = match field_args.data_type.as_ref() {
            Some(value) => quote!(#value),
            None => rust_type_to_data_type(&ty_string)?,
        };
        let nullable = field_args.nullable || is_option(&ty_string);
        let description = match field_args.description {
            Some(value) => quote!(Some(#value.to_string())),
            None => quote!(None),
        };
        properties.push(quote! { ::tesela::__private::Property { api_name: ::tesela::__private::ApiName::new_unchecked(#field_name), display: None, description: #description, data_type: #data_type, nullable: if #nullable { Some(true) } else { None }, default: None, allowed_values: None, tags: Vec::new() } });
        accessors.push(quote! { #[must_use] pub const fn #ident() -> ::tesela::Field<Self, #ty> { ::tesela::Field::new(#field_name) } });
        encoders.push(quote! { let name = { static NAME: ::std::sync::OnceLock<::tesela::__private::ApiName> = ::std::sync::OnceLock::new(); ::tesela::model::cached_name(&NAME, #field_name) }; values.insert(name, ::tesela::model::encode_value(&self.#ident)?); });
        lazy_fields.push(
            quote! { #field_name => Ok(Some(::tesela::model::encode_value(&self.#ident)?)), },
        );
        decoders.push(quote! { let #ident: #ty = ::tesela::model::decode_value(record.values.remove(#field_name), #field_name)?; });
        initializers.push(quote!(#ident));
    }
    Ok(quote! {
        impl ::tesela::Object for #name {
            type Key = #key_type;
            fn definition() -> ::tesela::__private::ObjectType {
                ::tesela::__private::ObjectType { api_name: ::tesela::__private::ApiName::new_unchecked(#api_name), display: #display, description: #description, primary_key: ::tesela::__private::ApiName::new_unchecked(#key_name), properties: vec![#(#properties),*], traits: Vec::new(), tags: Vec::new(), deprecated_at: None }
            }
            fn api_name() -> ::tesela::__private::ApiName {
                static NAME: ::std::sync::OnceLock<::tesela::__private::ApiName> = ::std::sync::OnceLock::new();
                ::tesela::model::cached_name(&NAME, #api_name)
            }
            fn encode_field(&self, name: &::tesela::__private::ApiName) -> Result<Option<::tesela::__private::Value>, ::tesela::Error> {
                match name.as_str() { #(#lazy_fields)* _ => Ok(None) }
            }
            fn key(&self) -> Result<Self::Key, ::tesela::Error> { Ok(self.#key.clone()) }
            fn encode(&self) -> Result<std::collections::BTreeMap<::tesela::__private::ApiName, ::tesela::__private::Value>, ::tesela::Error> { let mut values = std::collections::BTreeMap::new(); #(#encoders)* Ok(values) }
            fn decode(mut record: ::tesela::__private::Record) -> Result<Self, ::tesela::Error> { #(#decoders)* Ok(Self { #(#initializers),* }) }
        }
        impl #name { #(#accessors)* }
    })
}

fn parse_args(input: &DeriveInput) -> syn::Result<Args> {
    let mut args = Args::default();
    for attr in tesela_attrs(&input.attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("key") {
                args.key = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("name") {
                args.name = Some(meta.value()?.parse()?);
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
            Err(unsupported_attr(&meta, "Object"))
        })?;
    }
    Ok(args)
}
fn option_string(value: Option<LitStr>) -> TokenStream2 {
    value.map_or_else(|| quote!(None), |value| quote!(Some(#value.to_string())))
}
