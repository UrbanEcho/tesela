//! Shared helper functions for macro code generation.

use quote::quote;
use syn::Attribute;

/// Convert a PascalCase or camelCase identifier to snake_case.
pub(crate) fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(ch.to_ascii_lowercase());
    }
    out
}

/// Map a Rust type path (as a string slice) to a Tesela data type variant.
pub(crate) fn rust_type_to_data_type(ty_str: &str) -> syn::Result<proc_macro2::TokenStream> {
    let ty = ty_str
        .strip_prefix("Option<")
        .or_else(|| ty_str.strip_prefix("std::option::Option<"))
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(ty_str)
        .trim();
    let value = match ty {
        "String" | "std::string::String" | "&str" | "str" => {
            quote!(::tesela::__private::DataType::String)
        }
        "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" | "usize" | "isize" => {
            quote!(::tesela::__private::DataType::Integer)
        }
        "f32" | "f64" => quote!(::tesela::__private::DataType::Float),
        "bool" => quote!(::tesela::__private::DataType::Boolean),
        "uuid::Uuid" | "Uuid" => quote!(::tesela::__private::DataType::Uuid),
        "serde_json::Value" | "Value" => quote!(::tesela::__private::DataType::Json),
        "rust_decimal::Decimal" | "Decimal" => quote!(::tesela::__private::DataType::Decimal),
        "chrono::NaiveDate" | "NaiveDate" => quote!(::tesela::__private::DataType::Date),
        "chrono::NaiveDateTime" | "NaiveDateTime" => {
            quote!(::tesela::__private::DataType::Timestamp)
        }
        "chrono::DateTime<chrono::Utc>" | "DateTime<Utc>" => {
            quote!(::tesela::__private::DataType::TimestampTz)
        }
        value if value.starts_with("Vec<") || value.starts_with('[') => {
            quote!(::tesela::__private::DataType::Array)
        }
        _ => {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "unsupported field type `{ty_str}`; use a supported scalar or #[tesela(data_type = ...)]"
                ),
            ));
        }
    };
    Ok(value)
}

/// Return whether the type string represents an `Option<T>`.
pub(crate) fn is_option(ty_str: &str) -> bool {
    ty_str.starts_with("Option<") || ty_str.starts_with("std::option::Option<")
}

/// Stringify a `syn::Type`.
pub(crate) fn type_to_string(ty: &syn::Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
}

/// The `#[tesela(...)]` attributes attached to an item or field.
pub(crate) fn tesela_attrs(attrs: &[Attribute]) -> impl Iterator<Item = &Attribute> {
    attrs.iter().filter(|attr| attr.path().is_ident("tesela"))
}

/// Read the string literal of a `#[tesela(key = "...")]` entry.
pub(crate) fn string_value(meta: &syn::meta::ParseNestedMeta<'_>) -> syn::Result<String> {
    Ok(meta.value()?.parse::<syn::LitStr>()?.value())
}

/// Field-level `#[tesela(...)]` arguments, shared by `Object` and `Trait`.
#[derive(Debug, Default)]
pub(crate) struct FieldArgs {
    pub(crate) nullable: bool,
    pub(crate) data_type: Option<syn::Expr>,
    pub(crate) description: Option<syn::LitStr>,
}

/// Parse the field-level attributes a property declaration accepts.
pub(crate) fn parse_field_args(attrs: &[Attribute]) -> syn::Result<FieldArgs> {
    let mut args = FieldArgs::default();
    for attr in tesela_attrs(attrs) {
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("nullable") {
                args.nullable = true;
                return Ok(());
            }
            if meta.path.is_ident("data_type") {
                args.data_type = Some(meta.value()?.parse()?);
                return Ok(());
            }
            if meta.path.is_ident("description") {
                args.description = Some(meta.value()?.parse()?);
                return Ok(());
            }
            Err(unsupported_attr(&meta, "field"))
        })?;
    }
    Ok(args)
}

/// The compile error for a `#[tesela(...)]` entry a macro does not accept.
///
/// The entry is named from the parse tree, not from a table here: a curated
/// list of "attributes we no longer accept" duplicates knowledge the IR already
/// owns, goes stale without anything failing when the IR changes again, and has
/// to assert a per-attribute reason it cannot actually know — `junction` is not
/// removed for the same reason `indexed` is, and neither is removed for the
/// same reason a typo is rejected. The rule the design follows is stated once,
/// in the crate documentation; the error only has to point at what is wrong.
pub(crate) fn unsupported_attr(meta: &syn::meta::ParseNestedMeta<'_>, item: &str) -> syn::Error {
    let path = &meta.path;
    meta.error(format!(
        "unsupported {item} #[tesela(...)] attribute `{}`",
        quote!(#path)
    ))
}
