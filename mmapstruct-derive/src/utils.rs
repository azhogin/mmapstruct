//! Shared utility functions for type name resolution and helpers.

use quote::quote;
use syn::{Expr, ExprLit, GenericArgument, Lit, PathArguments, Type};

/// Known primitive pod type names.
pub(crate) const PRIMITIVE_POD: &[&str] = &[
    "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64", "bool", "usize", "isize",
];

pub(crate) fn is_f32_or_f64(ty: &Type) -> bool {
    type_ident_name(ty)
        .map(|n| n == "f32" || n == "f64")
        .unwrap_or(false)
}

pub(crate) fn type_ident_name(ty: &Type) -> Option<String> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

pub(crate) fn is_primitive_pod(ty: &Type) -> bool {
    type_ident_name(ty)
        .map(|n| PRIMITIVE_POD.contains(&n.as_str()))
        .unwrap_or(false)
}

pub(crate) fn is_u32_type(ty: &Type) -> bool {
    type_ident_name(ty).map(|n| n == "u32").unwrap_or(false)
}

pub(crate) fn is_string_type(ty: &Type) -> bool {
    type_ident_name(ty).map(|n| n == "String").unwrap_or(false)
}

pub(crate) fn extract_vec_inner(ty: &Type) -> Option<Type> {
    if let Type::Path(tp) = ty {
        let seg = tp.path.segments.last()?;
        if seg.ident == "Vec" {
            if let PathArguments::AngleBracketed(args) = &seg.arguments {
                if let Some(GenericArgument::Type(inner)) = args.args.first() {
                    return Some(inner.clone());
                }
            }
        }
    }
    None
}

pub(crate) fn extract_option_inner(ty: &Type) -> Option<Type> {
    if let Type::Path(tp) = ty {
        let seg = tp.path.segments.last()?;
        if seg.ident == "Option" {
            if let PathArguments::AngleBracketed(args) = &seg.arguments {
                if let Some(GenericArgument::Type(inner)) = args.args.first() {
                    return Some(inner.clone());
                }
            }
        }
    }
    None
}

/// Check if a type is all-pod (primitive or fixed array of primitives, recursively).
pub(crate) fn is_pod_type(ty: &Type) -> bool {
    if is_primitive_pod(ty) {
        return true;
    }
    if let Some((inner, _)) = extract_fixed_array_inner(ty) {
        return is_pod_type(&inner);
    }
    false
}

pub(crate) fn extract_fixed_array_inner(ty: &Type) -> Option<(Type, usize)> {
    if let Type::Array(arr) = ty {
        if let Expr::Lit(ExprLit { lit: Lit::Int(lit), .. }) = &arr.len {
            let n: usize = lit.base10_parse().ok()?;
            Some((*arr.elem.clone(), n))
        } else {
            None
        }
    } else {
        None
    }
}

pub(crate) fn extract_hashmap_inner(ty: &Type) -> Option<(Type, Type)> {
    if let Type::Path(tp) = ty {
        let seg = tp.path.segments.last()?;
        if seg.ident == "HashMap" {
            if let PathArguments::AngleBracketed(args) = &seg.arguments {
                let mut iter = args.args.iter();
                if let (Some(GenericArgument::Type(key)), Some(GenericArgument::Type(val))) =
                    (iter.next(), iter.next())
                {
                    return Some((key.clone(), val.clone()));
                }
            }
        }
    }
    None
}

pub(crate) fn known_primitive_size(ty: &Type) -> Option<usize> {
    let name = type_ident_name(ty)?;
    match name.as_str() {
        "u8" | "i8" | "bool" => Some(1),
        "u16" | "i16" => Some(2),
        "u32" | "i32" | "f32" => Some(4),
        "u64" | "i64" | "f64" | "usize" | "isize" => Some(8),
        _ => None,
    }
}

pub(crate) fn type_size_expr(ty: &Type) -> proc_macro2::TokenStream {
    if let Some(sz) = known_primitive_size(ty) {
        quote! { #sz }
    } else if let Some((inner, count)) = extract_fixed_array_inner(ty) {
        let inner_sz = type_size_expr(&inner);
        quote! { #inner_sz * #count }
    } else {
        quote! { ::std::mem::size_of::<#ty>() }
    }
}

pub(crate) fn type_align_expr(ty: &Type) -> proc_macro2::TokenStream {
    if let Some(sz) = known_primitive_size(ty) {
        let align = sz.min(8).max(1);
        quote! { #align }
    } else if let Some((inner, _)) = extract_fixed_array_inner(ty) {
        type_align_expr(&inner)
    } else {
        quote! { { let a = ::std::mem::align_of::<#ty>(); if a > 8 { 8usize } else if a == 0 { 1usize } else { a } } }
    }
}

/// Find a `#[repr(u32)]` or similar integer repr in the attribute list.
/// Returns `Some("u32")` with the specific repr name, or `None`.
pub(crate) fn find_repr_int(attrs: &[syn::Attribute]) -> Option<&'static str> {
    static REPR_INTS: &[&str] = &["u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64"];
    for attr in attrs {
        if !attr.path().is_ident("repr") {
            continue;
        }
        let Ok(list) = attr.parse_args_with(
            syn::punctuated::Punctuated::<syn::Ident, syn::Token![,]>::parse_terminated,
        ) else {
            continue;
        };
        for ident in &list {
            let name = ident.to_string();
            if let Some(&found) = REPR_INTS.iter().find(|&&s| s == name) {
                return Some(found);
            }
        }
    }
    None
}

/// Convenience: check only whether any integer repr exists.
pub(crate) fn has_repr_int(attrs: &[syn::Attribute]) -> bool {
    find_repr_int(attrs).is_some()
}
