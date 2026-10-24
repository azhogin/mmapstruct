//! Shared write helpers (`generate_fixed_writes`, `generate_inline_dyn_writes`) for struct and enum codegen.

use quote::quote;

use crate::attrs::*;
use crate::category::*;
use crate::field_info::*;

/// Generate the fixed-section writer calls (pod + inline string + byref).
///
/// `field_ref` returns the expression to access a field.
/// - Struct: `|id| quote! { &self.#id }`
/// - Enum:   `|id| quote! { #id }`
pub(crate) fn generate_fixed_writes<F>(
    fields: &[FieldInfo],
    field_ref: F,
) -> Vec<proc_macro2::TokenStream>
where
    F: Fn(&proc_macro2::Ident) -> proc_macro2::TokenStream,
{
    let fixed_indices: Vec<usize> = fields
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            is_pod_field(&f.category)
                || is_inline_string_field(&f.category)
                || is_fixed_array_field(&f.category)
                || (f.is_fixed_byref())
        })
        .map(|(i, _)| i)
        .collect();
    fixed_indices.iter().map(|&fixed_idx| {
        let fi = &fields[fixed_idx];
        let fident = &fi.ident;
        let field = field_ref(fident);
        if is_inline_string_field(&fi.category) {
            if let FieldCategory::InlineString { size } = &fi.category {
                quote! {
                    ctx.write_inline_string(#field, #size);
                }
            } else {
                unreachable!()
            }
        } else if fi.is_fixed_byref() {
            let dedup_literal = fi.dedup;
            match &fi.category {
                FieldCategory::DynamicOption { .. } => {
                    quote! {
                        let base = ctx.current_offset();
                        if let Some(val) = #field {
                            ctx.write_byref(val, start, #dedup_literal);
                        } else {
                            ctx.write_pod(&::mmapstruct::traits::ByRefRange { begin: 0, end: 0 });
                        }
                    }
                }
                _ => {
                    quote! {
                        ctx.write_byref(#field, start, #dedup_literal);
                    }
                }
            }
        } else if is_fixed_array_field(&fi.category) {
            let ty = &fi.ty;
            quote! {
                // Clamp to 8: the reader/schema layout consts clamp
                // alignment the same way (type_align_expr), so the writer
                // and the const-eval layout agree by construction.
                ctx.align_from(start, std::mem::align_of::<#ty>().min(8));
                unsafe {
                    let bytes = std::slice::from_raw_parts((#field as *const #ty).cast::<u8>(), std::mem::size_of::<#ty>());
                    ctx.write_raw(bytes);
                }
            }
        } else if is_pod_field(&fi.category) {
            let ty = &fi.ty;
            quote! {
                // Clamp to 8: the reader/schema layout consts clamp
                // alignment the same way (type_align_expr), so the writer
                // and the const-eval layout agree by construction.
                ctx.align_from(start, std::mem::align_of::<#ty>().min(8));
                ctx.write_pod(#field);
            }
        } else {
            quote! {}
        }
    }).collect()
}

/// Generate the inline-dynamic writer blocks (pod array, string, fixed array, etc).
///
/// `field_ref` returns the expression to access a field.
/// - Struct: `|id| quote! { &self.#id }`
/// - Enum:   `|id| quote! { #id }`
pub(crate) fn generate_inline_dyn_writes<F>(
    fields: &[FieldInfo],
    inline_dyn_indices: &[usize],
    field_ref: F,
) -> Vec<proc_macro2::TokenStream>
where
    F: Fn(&proc_macro2::Ident) -> proc_macro2::TokenStream,
{
    inline_dyn_indices.iter().map(|&idx| {
        let fi = &fields[idx];
        let fident = &fi.ident;
        let field = field_ref(fident);
        match &fi.category {
            FieldCategory::DynamicVec { inner, elem_is_pod, elem_fixed } if *elem_is_pod => {
                quote! {
                    {
                        ctx.align_from(vs, std::mem::align_of::<#inner>());
                        let before = (ctx.current_offset() - vs) as u64;
                        dyn_offsets.push(before);
                        ::mmapstruct::pod_array::write_pod_vec(#field, ctx);
                    }
                }
            }
            _ => {
                let write_expr = match &fi.category {
                    FieldCategory::DynamicString => {
                        quote! { ::mmapstruct::traits::MmapWrite::write(#field, ctx); }
                    }
                    FieldCategory::DynamicVecByRef { dedup, .. } => {
                        quote! { ::mmapstruct::byref_array::write_byref_array(#field, ctx, start, #dedup); }
                    }
                    FieldCategory::DynamicVec { elem_fixed, .. } => {
                        if *elem_fixed {
                            quote! { ::mmapstruct::fixed_array::write_fixed_array(#field, ctx); }
                        } else {
                            quote! { ::mmapstruct::indexed_array::write_indexed_array(#field, ctx); }
                        }
                    }
                    FieldCategory::DynamicHashMap { key_layout, val_layout, .. } => {
                        let mod_name = hash_map_module_name(*key_layout, *val_layout);
                        quote! { ::mmapstruct::hash_map::#mod_name::write_hash_map(#field, ctx); }
                    }
                    _ => {
                        quote! { ::mmapstruct::traits::MmapWrite::write(#field, ctx); }
                    }
                };
                quote! {
                    {
                        ctx.align_to(8);
                        let before = (ctx.current_offset() - vs) as u64;
                        dyn_offsets.push(before);
                        #write_expr
                    }
                }
            }
        }
    }).collect()
}
