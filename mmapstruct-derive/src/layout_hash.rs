//! Generates `LayoutHash` implementations for layout change detection.

use quote::{format_ident, quote};

use crate::category::*;
use crate::field_info::*;

/// Generate `impl LayoutHash` for a struct.
pub(crate) fn generate_layout_hash_impl(
    name: &proc_macro2::Ident,
    fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let mut field_hashes: Vec<_> = fields.iter().map(|fi| {
        let ty = &fi.ty;
        let base = quote! { <#ty as ::mmapstruct::traits::LayoutHash>::HASH };

        if fi.is_fixed_byref() {
            quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_BYREF, #base]) }
        } else {
            match &fi.category {
                FieldCategory::InlineString { size } => {
                    let s = *size as u64;
                    quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_INLINE, #base, #s]) }
                }
                FieldCategory::DynamicVec { elem_is_pod, elem_fixed, .. } => {
                    if *elem_is_pod {
                        quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_POD_VEC, #base]) }
                    } else if *elem_fixed {
                        quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_FIXED_VEC, #base]) }
                    } else {
                        base
                    }
                }
                FieldCategory::DynamicVecByRef { .. } => {
                    quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_BYREF_VEC, #base]) }
                }
                FieldCategory::DynamicHashMap { .. } => {
                    quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_HASHMAP, #base]) }
                }
                _ => base,
            }
        }
    }).collect();

    let fs_name = format_ident!("__MMAP_FS_{}", name);
    let hs_name = format_ident!("__MMAP_HS_{}", name);

    field_hashes.push(quote! { #fs_name as u64 });
    field_hashes.push(quote! { #hs_name as u64 });

    quote! {
        impl ::mmapstruct::traits::LayoutHash for #name {
            const HASH: u64 = ::mmapstruct::traits::combine_hashes(&[
                #(#field_hashes),*
            ]);
        }
    }
}
