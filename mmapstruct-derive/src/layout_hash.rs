//! Generates `LayoutHash` implementations for layout change detection.

use quote::{format_ident, quote};

use crate::category::*;
use crate::field_info::*;
use crate::utils::{known_primitive_size, type_size_expr};

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
                FieldCategory::DynamicVec { inner, elem_is_pod, elem_fixed } => {
                    if *elem_is_pod {
                        if known_primitive_size(inner).is_some() {
                            quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_POD_VEC, #base]) }
                        } else {
                            // Custom pod element (may lack a derive, hence no
                            // LayoutHash): hash by its size instead.
                            let sz = type_size_expr(inner);
                            quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_POD_VEC, ::mmapstruct::traits::ENC_POD_CUSTOM, (#sz) as u64]) }
                        }
                    } else if *elem_fixed {
                        quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_FIXED_VEC, #base]) }
                    } else {
                        base
                    }
                }
                FieldCategory::DynamicVecByRef { .. } => {
                    quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_BYREF_VEC, #base]) }
                }
                FieldCategory::FixedArray { count, .. } => {
                    let c = *count as u64;
                    quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_FIXED_ARRAY, #base, #c]) }
                }
                FieldCategory::PodCustom => {
                    // Opaque pod blob (may lack a derive, hence no LayoutHash):
                    // its binary contribution is fully determined by its size.
                    let sz = type_size_expr(ty);
                    quote! { ::mmapstruct::traits::combine_hashes(&[::mmapstruct::traits::ENC_POD_CUSTOM, (#sz) as u64]) }
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
