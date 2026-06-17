//! The `FieldInfo` struct, `reader_type()`, and `analyze_fields()`.

use quote::quote;
use syn::Fields;

use crate::attrs::*;
use crate::category::*;

#[derive(Clone)]
pub(crate) struct FieldInfo {
    pub(crate) name: String,
    pub(crate) ty: syn::Type,
    pub(crate) category: FieldCategory,
    pub(crate) is_byref: bool,
    pub(crate) dedup: bool,
    pub(crate) ident: proc_macro2::Ident,
}

impl FieldInfo {
    pub(crate) fn is_fixed_byref(&self) -> bool {
        self.is_byref && !matches!(&self.category, FieldCategory::DynamicVecByRef { .. })
    }
}

pub(crate) fn reader_type(fi: &FieldInfo) -> proc_macro2::TokenStream {
    match &fi.category {
        FieldCategory::PodPrimitive | FieldCategory::PodCustom => {
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
        FieldCategory::DynamicCustom => {
            let ty = &fi.ty;
            quote! { <#ty as ::mmapstruct::traits::MmapRead>::Reader<'a> }
        }
    }
}

pub(crate) struct FieldAnalysis {
    pub fields: Vec<FieldInfo>,
    pub has_dynamic: bool,
}

pub(crate) fn analyze_fields(fields: &syn::Fields) -> FieldAnalysis {
    let named_fields = match fields {
        Fields::Named(f) => &f.named,
        _ => {
            return FieldAnalysis {
                fields: Vec::new(),
                has_dynamic: false,
            }
        }
    };

    let mut all_fields: Vec<FieldInfo> = Vec::new();

    for field in named_fields.iter() {
        let category = classify_field(field);
        let (is_byref, dedup, _inline_size) = parse_mmap_attrs(&field.attrs);

        all_fields.push(FieldInfo {
            name: field.ident.as_ref().unwrap().to_string(),
            ty: field.ty.clone(),
            category,
            is_byref,
            dedup,
            ident: field.ident.as_ref().unwrap().clone(),
        });
    }

    let has_dynamic = all_fields.iter().any(|f| !is_pod_field(&f.category));
    FieldAnalysis {
        fields: all_fields,
        has_dynamic,
    }
}
