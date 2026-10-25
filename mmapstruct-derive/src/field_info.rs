//! The `FieldInfo` struct, `reader_type()`, and `analyze_fields()`.

use quote::{format_ident, quote};
use syn::Fields;

use crate::attrs::*;
use crate::category::*;
use crate::utils::*;

#[derive(Clone)]
pub(crate) struct FieldInfo {
    pub(crate) name: String,
    pub(crate) ty: syn::Type,
    pub(crate) category: FieldCategory,
    pub(crate) has_byref_attr: bool,
    pub(crate) dedup: bool,
    pub(crate) ident: proc_macro2::Ident,
    pub(crate) is_mmap_bool: bool,
}

impl FieldInfo {
    /// Byref stored as a fixed-section `ByRefRange` placeholder (standalone byref fields,
    /// as opposed to `DynamicVecByRef` which stores ByRefRanges in an indexed array).
    pub(crate) fn is_fixed_byref(&self) -> bool {
        self.has_byref_attr && !matches!(&self.category, FieldCategory::DynamicVecByRef { .. })
    }
}

pub(crate) fn reader_type(fi: &FieldInfo) -> proc_macro2::TokenStream {
    match &fi.category {
        FieldCategory::PodPrimitive
        | FieldCategory::PodCustom
        | FieldCategory::FixedArray { .. } => {
            let ty = &fi.ty;
            quote! { &'a #ty }
        }
        FieldCategory::DynamicString | FieldCategory::InlineString { .. } => {
            quote! { &'a str }
        }
        FieldCategory::DynamicVec {
            inner,
            elem_is_pod,
            elem_fixed,
        } => {
            if *elem_is_pod {
                quote! { &'a [#inner] }
            } else if *elem_fixed {
                quote! { ::mmapstruct::fixed_array::FixedArrayReader<'a, #inner> }
            } else {
                quote! { ::mmapstruct::indexed_array::IndexedArrayReader<'a, #inner> }
            }
        }
        FieldCategory::DynamicVecByRef { inner, .. } => {
            quote! { ::mmapstruct::byref_array::ByRefArrayReader<'a, #inner> }
        }
        FieldCategory::DynamicHashMap {
            key_ty,
            val_ty,
            key_layout,
            val_layout,
        } => {
            let mod_name = hash_map_module_name(*key_layout, *val_layout);
            quote! { ::mmapstruct::hash_map::#mod_name::HashMapReader<'a, #key_ty, #val_ty> }
        }
        FieldCategory::DynamicOption { inner } => {
            quote! { Option<<#inner as ::mmapstruct::traits::MmapRead>::Reader<'a>> }
        }
        FieldCategory::DynamicList { inner } => {
            quote! { ::mmapstruct::list::ListReader<'a, #inner> }
        }
        FieldCategory::DynamicCustom => {
            let ty = &fi.ty;
            quote! { <#ty as ::mmapstruct::traits::MmapRead>::Reader<'a> }
        }
    }
}

pub(crate) struct FieldAnalysis {
    pub fields: Vec<FieldInfo>,
    pub has_dynamic: bool,
    pub is_tuple: bool,
}

fn analyze_named_fields(fields: &syn::Fields) -> FieldAnalysis {
    let named = match fields {
        Fields::Named(f) => &f.named,
        _ => unreachable!(),
    };
    let mut all_fields: Vec<FieldInfo> = Vec::new();

    for field in named.iter() {
        let category = classify_field(field);
        let (is_byref, dedup, _inline_size, is_bool) = parse_mmap_attrs(&field.attrs);

        // Validate #[mmap(bool)]
        if is_bool && !is_u32_type(&field.ty) {
            panic!("#[mmap(bool)] is only supported on u32 fields");
        }
        // Forbid bare bool type
        if is_primitive_pod(&field.ty) && type_ident_name(&field.ty).is_some_and(|n| n == "bool") {
            panic!("bool field is not supported; use u32 with #[mmap(bool)] instead");
        }

        all_fields.push(FieldInfo {
            name: field.ident.as_ref().unwrap().to_string(),
            ty: field.ty.clone(),
            category,
            has_byref_attr: is_byref,
            dedup,
            ident: field.ident.as_ref().unwrap().clone(),
            is_mmap_bool: is_bool,
        });
    }

    let has_dynamic = all_fields
        .iter()
        .any(|f| !is_pod_field(&f.category) || f.is_fixed_byref());
    FieldAnalysis {
        fields: all_fields,
        has_dynamic,
        is_tuple: false,
    }
}

fn analyze_unnamed_fields(fields: &syn::Fields) -> FieldAnalysis {
    let unnamed = match fields {
        Fields::Unnamed(f) => &f.unnamed,
        _ => unreachable!(),
    };
    let mut all_fields: Vec<FieldInfo> = Vec::new();

    for (i, field) in unnamed.iter().enumerate() {
        let ident = format_ident!("_{}", i);
        let name = i.to_string();
        let category = classify_field(field);
        let (is_byref, dedup, _inline_size, is_bool) = parse_mmap_attrs(&field.attrs);

        if is_bool && !is_u32_type(&field.ty) {
            panic!("#[mmap(bool)] is only supported on u32 fields");
        }
        if is_primitive_pod(&field.ty) && type_ident_name(&field.ty).is_some_and(|n| n == "bool") {
            panic!("bool field is not supported; use u32 with #[mmap(bool)] instead");
        }

        all_fields.push(FieldInfo {
            name,
            ty: field.ty.clone(),
            category,
            has_byref_attr: is_byref,
            dedup,
            ident,
            is_mmap_bool: is_bool,
        });
    }

    let has_dynamic = all_fields
        .iter()
        .any(|f| !is_pod_field(&f.category) || f.is_fixed_byref());
    FieldAnalysis {
        fields: all_fields,
        has_dynamic,
        is_tuple: true,
    }
}

pub(crate) fn analyze_fields(fields: &syn::Fields) -> FieldAnalysis {
    match fields {
        Fields::Named(_) => analyze_named_fields(fields),
        Fields::Unnamed(_) => analyze_unnamed_fields(fields),
        Fields::Unit => FieldAnalysis {
            fields: Vec::new(),
            has_dynamic: false,
            is_tuple: false,
        },
    }
}

/// Convert a FieldInfo to a syn::Member for field access.
pub(crate) fn field_member(fi: &FieldInfo, is_tuple: bool) -> syn::Member {
    if is_tuple {
        let idx: usize = fi
            .name
            .parse()
            .expect("tuple field name should be a number");
        syn::Member::Unnamed(syn::Index::from(idx))
    } else {
        syn::Member::Named(fi.ident.clone())
    }
}
