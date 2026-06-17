//! Generates `MmapFromDefault` impls.

use quote::quote;

use crate::utils::*;
use crate::category::*;
use crate::field_info::*;

/// Generate the `impl From<Reader> for Struct` block.
pub(crate) fn generate_from_impl(
    name: &proc_macro2::Ident,
    reader_name: &proc_macro2::Ident,
    fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let conversions: Vec<_> = fields
        .iter()
        .map(|fi| {
            let fident = &fi.ident;
            let expr = match &fi.category {
                FieldCategory::PodPrimitive | FieldCategory::PodCustom => {
                    quote! { *r.#fident }
                }
                FieldCategory::DynamicOption { inner } => {
                    if is_primitive_pod(inner) {
                        quote! { r.#fident.map(|v| *v) }
                    } else {
                        quote! { r.#fident.map(|v| v.into()) }
                    }
                }
                _ => {
                    quote! { r.#fident.into() }
                }
            };
            quote! { #fident: #expr }
        })
        .collect();

    quote! {
        impl<'a> From<#reader_name<'a>> for #name {
            fn from(r: #reader_name<'a>) -> Self {
                Self { #(#conversions),* }
            }
        }
    }
}

/// Generate `impl ReaderOwned` for a mixed struct (uses the generated `From` impl).
pub(crate) fn generate_reader_owned_impl(
    name: &proc_macro2::Ident,
    _reader_name: &proc_macro2::Ident,
    _fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    quote! {
        impl ::mmapstruct::traits::ReaderOwned for #name {
            fn into_owned(reader: Self::Reader<'_>) -> Self {
                reader.into()
            }
        }
    }
}

/// Generate `impl ReaderOrd` for a mixed struct (field-by-field comparison).
pub(crate) fn generate_reader_ord_impl(
    name: &proc_macro2::Ident,
    reader_name: &proc_macro2::Ident,
    fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let is_simple_field = |cat: &FieldCategory| -> bool {
        match cat {
            FieldCategory::PodPrimitive
            | FieldCategory::PodCustom
            | FieldCategory::DynamicString
            | FieldCategory::InlineString { .. } => true,
            FieldCategory::DynamicOption { inner: _ } => true,
            _ => false,
        }
    };
    if fields.iter().any(|fi| !is_simple_field(&fi.category)) {
        return quote! {};
    }

    let comparisons: Vec<_> = fields.iter().map(|fi| {
        let fident = &fi.ident;
        match &fi.category {
            FieldCategory::PodPrimitive | FieldCategory::PodCustom => {
                if is_f32_or_f64(&fi.ty) {
                    quote! { reader.#fident.partial_cmp(&key.#fident).unwrap_or(std::cmp::Ordering::Equal) }
                } else {
                    quote! { reader.#fident.cmp(&key.#fident) }
                }
            }
            FieldCategory::DynamicString | FieldCategory::InlineString { .. } => {
                quote! { reader.#fident.cmp(key.#fident.as_str()) }
            }
            FieldCategory::DynamicOption { inner } => {
                quote! {
                    match (&reader.#fident, &key.#fident) {
                        (Some(r), Some(k)) => <#inner as ::mmapstruct::traits::ReaderOrd>::cmp_reader(r, k),
                        (None, None) => std::cmp::Ordering::Equal,
                        (None, Some(_)) => std::cmp::Ordering::Less,
                        (Some(_), None) => std::cmp::Ordering::Greater,
                    }
                }
            }
            _ => {
                quote! { compile_error!("mmapstruct: ReaderOrd not supported for this field type") }
            }
        }
    }).collect();

    let body = if comparisons.is_empty() {
        quote! { std::cmp::Ordering::Equal }
    } else if comparisons.len() == 1 {
        comparisons.into_iter().next().unwrap()
    } else {
        let mut iter = comparisons.into_iter();
        let first = iter.next().unwrap();
        iter.fold(first, |acc, cmp| quote! { #acc.then(#cmp) })
    };

    quote! {
        impl ::mmapstruct::traits::ReaderOrd for #name {
            fn cmp_reader(reader: &#reader_name<'_>, key: &Self) -> std::cmp::Ordering {
                #body
            }
        }
    }
}

/// Generate the `impl Default` block.
pub(crate) fn generate_default_impl(
    name: &proc_macro2::Ident,
    fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let defaults: Vec<_> = fields
        .iter()
        .map(|fi| {
            let fident = &fi.ident;
            quote! { #fident: Default::default() }
        })
        .collect();

    quote! {
        impl Default for #name {
            fn default() -> Self {
                Self { #(#defaults),* }
            }
        }
    }
}
