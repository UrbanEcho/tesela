//! `#[derive(Action)]` implementation.

use crate::helpers::{is_option, tesela_attrs, to_snake_case, type_to_string, unsupported_attr};
use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Ident, Type, parse_macro_input};

#[derive(Default)]
struct Args {
    subject: Option<Type>,
    returns: Option<Type>,
    reads: Vec<Type>,
    writes: Vec<Type>,
    creates: Vec<Type>,
    external: Vec<String>,
    pure: bool,
    idempotent: bool,
    concurrency: Option<Ident>,
    approval: Option<Ident>,
    risk: Option<Ident>,
    requires_revision: bool,
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
    if !args.pure
        && args.reads.is_empty()
        && args.writes.is_empty()
        && args.creates.is_empty()
        && args.external.is_empty()
    {
        return Err(syn::Error::new(
            ident.span(),
            "Action requires explicit reads(...), writes(...), creates(...), external(...) or pure",
        ));
    }
    let output = args
        .returns
        .ok_or_else(|| syn::Error::new(ident.span(), "Action requires returns = Type"))?;
    let output_schema_type = json_schema_type(&type_to_string(&output));
    let fields = match &input.data {
        syn::Data::Struct(data) => match &data.fields {
            syn::Fields::Named(fields) => fields.named.iter().collect::<Vec<_>>(),
            _ => {
                return Err(syn::Error::new(
                    ident.span(),
                    "Action requires named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new(
                ident.span(),
                "Action can only be derived for a struct",
            ));
        }
    };
    let api_name = to_snake_case(&ident.to_string());
    let subject = args.subject.map_or_else(
        || quote!(None),
        |value| quote!(Some(<#value as ::tesela::Object>::api_name())),
    );
    let reads = args
        .reads
        .iter()
        .map(|value| quote!(<#value as ::tesela::Object>::api_name()));
    let writes = args
        .writes
        .iter()
        .map(|value| quote!(<#value as ::tesela::Object>::api_name()));
    let creates = args
        .creates
        .iter()
        .map(|value| quote!(<#value as ::tesela::Object>::api_name()));
    let external = args
        .external
        .iter()
        .map(|value| quote!(::tesela::__private::ApiName::new_unchecked(#value)));
    let reads_effect = !args.reads.is_empty();
    let writes_effect = !args.writes.is_empty() || !args.creates.is_empty();
    let external_effect = !args.external.is_empty();
    let idempotent = args.pure || args.idempotent;
    let effects = quote!(::tesela::__private::ActionEffects { reads: #reads_effect, writes: #writes_effect, external: #external_effect, idempotent: #idempotent });
    let concurrency = enum_value(
        args.concurrency.as_ref(),
        "concurrency",
        &[
            ("serial", "Serial"),
            ("optimistic", "Optimistic"),
            ("unrestricted", "Unrestricted"),
        ],
        "::tesela::__private::Concurrency",
        "serial",
    )?;
    let approval = enum_value(
        args.approval.as_ref(),
        "approval",
        &[
            ("never", "Never"),
            ("policy", "Policy"),
            ("always", "Always"),
        ],
        "::tesela::__private::ApprovalPolicy",
        "policy",
    )?;
    let risk = enum_value(
        args.risk.as_ref(),
        "risk",
        &[("low", "Low"), ("medium", "Medium"), ("high", "High")],
        "::tesela::__private::RiskLevel",
        "low",
    )?;
    let requires_revision = args.requires_revision;
    let mut encoders = Vec::new();
    let mut decoders = Vec::new();
    let mut initializers = Vec::new();
    let mut schema = Vec::new();
    let mut required = Vec::new();
    for field in fields {
        let field_ident = field.ident.as_ref().expect("named field");
        let field_name = field_ident.to_string();
        let ty = &field.ty;
        encoders.push(quote! { map.insert(#field_name.to_owned(), ::tesela::__private::serde_json::to_value(&self.#field_ident).map_err(|error| ::tesela::Error::bad_request(error.to_string()))?); });
        decoders.push(quote! { let #field_ident: #ty = ::tesela::model::decode_value(values.remove(#field_name).map(::tesela::__private::Value::new), #field_name)?; });
        initializers.push(quote!(#field_ident));
        let schema_type = json_schema_type(&type_to_string(ty));
        schema.push(quote! { properties.insert(#field_name.to_owned(), ::tesela::__private::serde_json::json!({"type": #schema_type})); });
        if !is_option(&type_to_string(ty)) {
            required.push(field_name.clone());
        }
    }
    Ok(quote! {
        impl ::tesela::Action for #ident {
            type Output = #output;
            fn definition() -> ::tesela::__private::ActionType {
                let mut properties = ::tesela::__private::serde_json::Map::new(); #(#schema)*
                ::tesela::__private::ActionType { api_name: ::tesela::__private::ApiName::new_unchecked(#api_name), display: None, description: None, subject: #subject, input_schema: Some(::tesela::__private::Value::new(::tesela::__private::serde_json::json!({"type":"object","properties":properties,"required":[#(#required),*]}))), output_schema: Some(::tesela::__private::Value::new(::tesela::__private::serde_json::json!({"type": #output_schema_type}))), risk_level: #risk, effects: #effects, reads: vec![#(#reads),*], writes: vec![#(#writes),*], creates: vec![#(#creates),*], external_effects: vec![#(#external),*], approval: #approval, requires_revision: #requires_revision, preconditions: Vec::new(), concurrency: #concurrency, idempotency_key: None, deprecated_at: None }
            }
            fn encode(&self) -> Result<::tesela::__private::Value, ::tesela::Error> { let mut map = ::tesela::__private::serde_json::Map::new(); #(#encoders)* Ok(::tesela::__private::Value::new(map.into())) }
            fn decode_input(value: ::tesela::__private::Value) -> Result<Self, ::tesela::Error> { let mut values = value.0.as_object().cloned().ok_or_else(|| ::tesela::Error::bad_request("action input must be an object"))?; #(#decoders)* Ok(Self { #(#initializers),* }) }
            fn decode(value: Option<::tesela::__private::Value>) -> Result<Self::Output, ::tesela::Error> { let value = value.ok_or_else(|| ::tesela::Error::adapter("action returned no output"))?; ::tesela::__private::serde_json::from_value(value.into()).map_err(|error| ::tesela::Error::adapter(error.to_string())) }
            fn encode_output(value: &Self::Output) -> Result<::tesela::__private::Value, ::tesela::Error> { ::tesela::__private::serde_json::to_value(value).map(::tesela::__private::Value::new).map_err(|error| ::tesela::Error::adapter(error.to_string())) }
        }
    })
}

fn parse_args(input: &DeriveInput) -> syn::Result<Args> {
    let mut args = Args::default();
    for attr in tesela_attrs(&input.attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("subject") {
                args.subject = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("returns") {
                args.returns = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("pure") {
                args.pure = true;
                return Ok(());
            }
            if meta.path.is_ident("idempotent") {
                args.idempotent = true;
                return Ok(());
            }
            if meta.path.is_ident("requires_revision") {
                args.requires_revision = true;
                return Ok(());
            }
            for (name, value) in [
                ("concurrency", &mut args.concurrency),
                ("approval", &mut args.approval),
                ("risk", &mut args.risk),
            ] {
                if meta.path.is_ident(name) {
                    *value = Some(meta.value()?.parse()?);
                    return Ok(());
                }
            }
            for (name, values) in [
                ("reads", &mut args.reads),
                ("writes", &mut args.writes),
                ("creates", &mut args.creates),
            ] {
                if meta.path.is_ident(name) {
                    meta.parse_nested_meta(|inner| {
                        values.push(Type::Path(syn::TypePath {
                            qself: None,
                            path: inner.path.clone(),
                        }));
                        Ok(())
                    })?;
                    return Ok(());
                }
            }
            if meta.path.is_ident("external") {
                meta.parse_nested_meta(|inner| {
                    let name = inner
                        .path
                        .get_ident()
                        .ok_or_else(|| inner.error("external effects must be identifiers"))?;
                    args.external.push(name.to_string());
                    Ok(())
                })?;
                return Ok(());
            }
            Err(unsupported_attr(&meta, "Action"))
        })?;
    }
    Ok(args)
}

fn enum_value(
    value: Option<&Ident>,
    name: &str,
    allowed: &[(&str, &str)],
    path: &str,
    default: &str,
) -> syn::Result<proc_macro2::TokenStream> {
    let selected = value.map_or_else(|| default.to_owned(), ToString::to_string);
    let variant = allowed
        .iter()
        .find_map(|(input, variant)| (*input == selected).then_some(*variant))
        .ok_or_else(|| {
            syn::Error::new(
                value.expect("invalid value exists").span(),
                format!(
                    "invalid {name}; expected {}",
                    allowed
                        .iter()
                        .map(|item| item.0)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })?;
    let path: syn::Path = syn::parse_str(path).expect("static enum path");
    let variant = Ident::new(variant, proc_macro2::Span::call_site());
    Ok(quote!(#path::#variant))
}

fn json_schema_type(ty: &str) -> &'static str {
    let ty = ty
        .strip_prefix("Option<")
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(ty);
    if ty == "bool" {
        "boolean"
    } else if matches!(
        ty,
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize"
    ) {
        "integer"
    } else if matches!(ty, "f32" | "f64") {
        "number"
    } else if ty.starts_with("Vec<") || ty.starts_with('[') {
        "array"
    } else if ty == "serde_json::Value" || ty == "Value" {
        "object"
    } else {
        "string"
    }
}
