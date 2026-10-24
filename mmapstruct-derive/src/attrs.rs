//! Parsing `#[mmap(...)]` attributes on struct fields.

use quote::format_ident;
use syn::punctuated::Punctuated;
use syn::{LitInt, Meta, Type};

use crate::utils::*;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum HashMapLayout {
    Pod,
    Inlined,
}

pub(crate) fn has_struct_scope_attr(attrs: &[syn::Attribute]) -> bool {
    has_struct_mmap_ident(attrs, "scope")
}

pub(crate) fn has_struct_skip_default_attr(attrs: &[syn::Attribute]) -> bool {
    has_struct_mmap_ident(attrs, "skip_default")
}

fn has_struct_mmap_ident(attrs: &[syn::Attribute], ident: &str) -> bool {
    for attr in attrs {
        if !attr.path().is_ident("mmap") {
            continue;
        }
        if let Ok(metas) =
            attr.parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            for meta in &metas {
                if let Meta::Path(p) = &meta {
                    if p.is_ident(ident) {
                        return true;
                    }
                }
            }
        } else if let Ok(Meta::Path(p)) = attr.parse_args::<Meta>() {
            if p.is_ident(ident) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn has_pod_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if let Meta::Path(p) = &a.meta {
            p.is_ident("pod")
        } else {
            false
        }
    })
}

pub(crate) fn has_fixed_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if let Meta::Path(p) = &a.meta {
            p.is_ident("fixed")
        } else {
            false
        }
    })
}

pub(crate) fn detect_layout(ty: &Type) -> HashMapLayout {
    if is_primitive_pod(ty) {
        HashMapLayout::Pod
    } else {
        HashMapLayout::Inlined
    }
}

pub(crate) fn hash_map_module_name(key: HashMapLayout, val: HashMapLayout) -> proc_macro2::Ident {
    let k = match key {
        HashMapLayout::Pod => "podK",
        HashMapLayout::Inlined => "inlinedK",
    };
    let v = match val {
        HashMapLayout::Pod => "podV",
        HashMapLayout::Inlined => "inlinedV",
    };
    format_ident!("{}_{}", k, v)
}

pub(crate) fn parse_hashmap_attrs(
    attrs: &[syn::Attribute],
) -> (Option<HashMapLayout>, Option<HashMapLayout>) {
    let mut key = None;
    let mut val = None;
    for attr in attrs {
        if !attr.path().is_ident("mmap") {
            continue;
        }
        if let Ok(metas) =
            attr.parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            for meta in &metas {
                if let Meta::NameValue(nv) = meta {
                    if nv.path.is_ident("key") {
                        if let syn::Expr::Path(ep) = &nv.value {
                            key = ep.path.get_ident().map(|id| match id.to_string().as_str() {
                                "pod" => HashMapLayout::Pod,
                                "inlined" => HashMapLayout::Inlined,
                                _ => panic!("unknown HashMap key layout: {}", id),
                            });
                        }
                    }
                    if nv.path.is_ident("value") {
                        if let syn::Expr::Path(ep) = &nv.value {
                            val = ep.path.get_ident().map(|id| match id.to_string().as_str() {
                                "pod" => HashMapLayout::Pod,
                                "inlined" => HashMapLayout::Inlined,
                                _ => panic!("unknown HashMap value layout: {}", id),
                            });
                        }
                    }
                }
            }
        }
    }
    (key, val)
}

pub(crate) fn parse_mmap_attrs(attrs: &[syn::Attribute]) -> (bool, bool, Option<usize>, bool) {
    let mut byref = false;
    let mut dedup = false;
    let mut inline_size = None;
    let mut is_bool = false;
    for attr in attrs {
        if !attr.path().is_ident("mmap") {
            continue;
        }
        if let Ok(metas) =
            attr.parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            for meta in &metas {
                if let Meta::Path(p) = meta {
                    if p.is_ident("byref") {
                        byref = true;
                    }
                    if p.is_ident("dedup") {
                        dedup = true;
                    }
                    if p.is_ident("bool") {
                        is_bool = true;
                    }
                }
                if let Meta::List(list) = meta {
                    if list.path.is_ident("inline") {
                        if let Ok(n) = list.parse_args::<LitInt>() {
                            inline_size = Some(n.base10_parse::<usize>().unwrap());
                        }
                    }
                }
            }
        } else if let Ok(Meta::Path(p)) = attr.parse_args::<Meta>() {
            if p.is_ident("byref") {
                byref = true;
            }
            if p.is_ident("bool") {
                is_bool = true;
            }
        }
    }
    (byref, dedup, inline_size, is_bool)
}
