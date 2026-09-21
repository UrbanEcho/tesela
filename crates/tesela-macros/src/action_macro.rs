//! `#[action]` attribute macro implementation.

use crate::helpers::type_to_string;
use darling::FromMeta;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse_macro_input;

#[derive(Debug, Default, FromMeta)]
struct ActionArgs {
    #[darling(default)]
    subject: Option<String>,
    #[darling(default)]
    risk: Option<String>,
    #[darling(default)]
    effects: Option<String>,
    #[darling(default)]
    idempotent: darling::util::Flag,
    #[darling(default)]
    concurrency: Option<String>,
    #[darling(default)]
    display: Option<String>,
    #[darling(default)]
    description: Option<String>,
}

pub(crate) fn expand(args: TokenStream, input: TokenStream) -> TokenStream {
    let attr_args = match darling::ast::NestedMeta::parse_meta_list(args.into()) {
        Ok(value) => value,
        Err(error) => return TokenStream::from(error.to_compile_error()),
    };
    if let Some(error) = reject_removed(&attr_args) {
        return TokenStream::from(error.to_compile_error());
    }
    let macro_args = match ActionArgs::from_list(&attr_args) {
        Ok(value) => value,
        Err(error) => return TokenStream::from(error.write_errors()),
    };

    let input_fn = parse_macro_input!(input as syn::ItemFn);
    let fn_name = &input_fn.sig.ident;
    let fn_name_str = fn_name.to_string();
    let api_name_str = fn_name_str.clone();

    let subject = match macro_args.subject.as_deref() {
        Some(name) => quote!(Some(::tesela::__private::ApiName::new_unchecked(#name))),
        None => quote!(None),
    };

    let risk = match macro_args.risk.as_deref().map_or("low", |value| value) {
        "low" => quote!(::tesela::__private::RiskLevel::Low),
        "medium" => quote!(::tesela::__private::RiskLevel::Medium),
        "high" => quote!(::tesela::__private::RiskLevel::High),
        other => {
            return syn::Error::new_spanned(&input_fn, format!("unknown action risk '{other}'"))
                .to_compile_error()
                .into();
        }
    };

    let effects = match macro_args.effects.as_deref() {
        // An action that declares nothing is assumed to write: over-declaring a
        // write costs an approval prompt, under-declaring one is a security bug.
        None | Some("write") => quote!(::tesela::__private::ActionEffects::new()),
        Some("read_only") => quote!(::tesela::__private::ActionEffects::read_only()),
        Some("external") => quote!(::tesela::__private::ActionEffects {
            reads: true,
            writes: true,
            external: true,
            idempotent: false,
        }),
        Some(other) => {
            return syn::Error::new_spanned(
                &input_fn,
                format!(
                    "unknown action effects '{other}': expected \"read_only\", \"write\" or \"external\""
                ),
            )
            .to_compile_error()
            .into();
        }
    };
    let effects = if macro_args.idempotent.is_present() {
        quote!(::tesela::__private::ActionEffects { idempotent: true, ..#effects })
    } else {
        effects
    };

    let concurrency = match macro_args.concurrency.as_deref() {
        None | Some("serial") => quote!(::tesela::__private::Concurrency::Serial),
        Some("optimistic") => quote!(::tesela::__private::Concurrency::Optimistic),
        Some("unrestricted") => quote!(::tesela::__private::Concurrency::Unrestricted),
        Some(other) => {
            return syn::Error::new_spanned(
                &input_fn,
                format!(
                    "unknown action concurrency '{other}': expected \"serial\", \"optimistic\" or \"unrestricted\""
                ),
            )
            .to_compile_error()
            .into();
        }
    };

    let display_name = macro_args.display.unwrap_or_else(|| fn_name_str.clone());
    let description = macro_args.description.unwrap_or_default();

    let params = &input_fn.sig.inputs;
    let mut schema_props = Vec::new();
    for param in params {
        if let syn::FnArg::Typed(pt) = param
            && let syn::Pat::Ident(pi) = pt.pat.as_ref()
        {
            let param_name = pi.ident.to_string();
            let ty_str = type_to_string(&pt.ty);
            let base_type = ty_str.trim_start_matches("Option<").trim_end_matches('>');
            let dt_str = match base_type {
                "String" | "&str" => "string",
                "i8" | "i16" | "i32" | "i64" | "u8" | "u16" | "u32" | "u64" => "integer",
                "f32" | "f64" => "number",
                "bool" => "boolean",
                _ => "string",
            };
            schema_props.push(quote! {
                (#param_name.to_string(), ::tesela::__private::serde_json::json!({"type": #dt_str}))
            });
        }
    }

    let action_struct_name = format_ident!("{}Action", to_pascal_case(&fn_name_str));

    let expanded = quote! {
        #input_fn

        /// Generated action type accessor.
        pub struct #action_struct_name;

        impl #action_struct_name {
            /// Return the Tesela `ActionType` definition for this action.
            pub fn tesela_action_type() -> ::tesela::__private::ActionType {
                let props: std::collections::HashMap<String, ::tesela::__private::serde_json::Value> =
                    vec![ #(#schema_props),* ].into_iter().collect();
                ::tesela::__private::ActionType {
                    api_name: ::tesela::__private::ApiName::new_unchecked(#api_name_str),
                    display: Some(#display_name.to_string()),
                    description: if #description.is_empty() { None } else { Some(#description.to_string()) },
                    subject: #subject,
                    input_schema: if props.is_empty() {
                        None
                    } else {
                        Some(::tesela::__private::Value::new(::tesela::__private::serde_json::json!({
                            "type": "object",
                            "properties": props,
                        })))
                    },
                    output_schema: None,
                    risk_level: #risk,
                    effects: #effects,
                    reads: Vec::new(),
                    writes: Vec::new(),
                    creates: Vec::new(),
                    external_effects: Vec::new(),
                    approval: ::tesela::__private::ApprovalPolicy::Policy,
                    requires_revision: false,
                    preconditions: Vec::new(),
                    concurrency: #concurrency,
                    idempotency_key: None,
                    deprecated_at: None,
                }
            }
        }
    };

    TokenStream::from(expanded)
}

/// Arguments this macro used to accept, and why it no longer does.
///
/// `handler` in particular named a callback the macro never wired up: the
/// declaration claimed a binding that only the host can make.
fn reject_removed(args: &[darling::ast::NestedMeta]) -> Option<syn::Error> {
    for arg in args {
        let darling::ast::NestedMeta::Meta(meta) = arg else {
            continue;
        };
        let Some(ident) = meta.path().get_ident() else {
            continue;
        };
        let reason = match ident.to_string().as_str() {
            "handler" | "handler_kind" => {
                "an action no longer declares a handler: `ActionType` says what the action is and what it may touch, and the host binds an executor to it at runtime"
            }
            "mode" => {
                "an action no longer carries a mode: use `effects` to say what it reads and writes, and `concurrency` to say what the host must guarantee"
            }
            _ => continue,
        };
        return Some(syn::Error::new(ident.span(), reason));
    }
    None
}

/// Convert a `snake_case` identifier to `PascalCase`.
fn to_pascal_case(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut capitalize = true;
    for ch in value.chars() {
        if ch == '_' {
            capitalize = true;
        } else if capitalize {
            out.push(ch.to_ascii_uppercase());
            capitalize = false;
        } else {
            out.push(ch);
        }
    }
    out
}
