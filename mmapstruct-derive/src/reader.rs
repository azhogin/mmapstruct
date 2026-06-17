//! Generates `MmapRead` implementations.

use quote::{format_ident, quote};

use crate::attrs::*;
use crate::category::*;
use crate::field_info::*;
use crate::layout::*;

/// Generate the reader struct declaration (public fields with reader types).
pub(crate) fn generate_reader_struct(
    reader_name: &proc_macro2::Ident,
    fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let decls: Vec<_> = fields
        .iter()
        .map(|fi| {
            let fident = &fi.ident;
            let rt = reader_type(fi);
            quote! { pub #fident: #rt }
        })
        .collect();
    quote! { pub struct #reader_name<'a> { #(#decls),* } }
}

/// Generate reader init for a pod field (::mmapstruct::bytemuck::from_bytes).
pub(crate) fn generate_pod_reader_init(
    field: &FieldInfo,
    struct_name: &proc_macro2::Ident,
    all_fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let fident = &field.ident;
    let fo_name = format_ident!("__MMAP_FO_{}", struct_name);
    let idx = fixed_section_index(all_fields, field.name.as_str());
    let ty = &field.ty;
    quote! {
        #fident: ::mmapstruct::bytemuck::from_bytes(&data[range.start + #fo_name[#idx] .. range.start + #fo_name[#idx] + std::mem::size_of::<#ty>()])
    }
}

/// Generate reader init for an inline string field.
pub(crate) fn generate_inline_string_reader_init(
    field: &FieldInfo,
    struct_name: &proc_macro2::Ident,
    all_fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let fident = &field.ident;
    let fo_name = format_ident!("__MMAP_FO_{}", struct_name);
    let idx = fixed_section_index(all_fields, field.name.as_str());
    let sz = match &field.category {
        FieldCategory::InlineString { size } => *size,
        _ => unreachable!(),
    };
    quote! {
        #fident: {
            let s = range.start + #fo_name[#idx];
            <String as ::mmapstruct::traits::MmapRead>::read(data, s..s + #sz)
        }
    }
}

/// Generate reader init for a byref field (read_by_ref or Option<byref>).
pub(crate) fn generate_byref_reader_init(
    field: &FieldInfo,
    struct_name: &proc_macro2::Ident,
    all_fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let fident = &field.ident;
    let fo_name = format_ident!("__MMAP_FO_{}", struct_name);
    let idx = fixed_section_index(all_fields, field.name.as_str());
    match &field.category {
        FieldCategory::DynamicOption { inner } => {
            quote! {
                #fident: {
                    let base = range.start;
                    let br: ::mmapstruct::traits::ByRefRange = *::mmapstruct::bytemuck::from_bytes(&data[base + #fo_name[#idx] .. base + #fo_name[#idx] + 8]);
                    if br.begin == 0 && br.end == 0 {
                        None
                    } else {
                        Some(<#inner as ::mmapstruct::traits::MmapRead>::read(data, br.range(base)))
                    }
                }
            }
        }
        _ => {
            let ty = &field.ty;
            quote! {
                #fident: {
                    let base = range.start;
                    let byref_range = base + #fo_name[#idx] .. base + #fo_name[#idx] + 8;
                    ::mmapstruct::reader::read_by_ref::<#ty>(data, base, byref_range)
                }
            }
        }
    }
}

/// Generate reader init for an inline dynamic field (string, vec, custom, etc.).
pub(crate) fn generate_inline_dyn_reader_init(
    field: &FieldInfo,
    layout: &StructLayout,
    struct_name: &proc_macro2::Ident,
    all_fields: &[FieldInfo],
    inline_dyn_indices: &[usize],
) -> proc_macro2::TokenStream {
    let fident = &field.ident;
    let dyn_idx = inline_dyn_indices
        .iter()
        .position(|&i| all_fields[i].name == field.name)
        .expect("inline dyn field not in indices");
    let range_calc = gen_dyn_reader_init(dyn_idx, struct_name, layout.inline_dyn_count);
    let ty = &field.ty;
    match &field.category {
        FieldCategory::DynamicString => {
            quote! {
                #fident: {
                    let (s, e) = #range_calc;
                    <#ty as ::mmapstruct::traits::MmapRead>::read(data, s..e)
                }
            }
        }
        FieldCategory::DynamicVecByRef { .. } => {
            quote! {
                #fident: {
                    let base = range.start;
                    let (s, e) = #range_calc;
                    ::mmapstruct::byref_array::ByRefArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e), base)
                }
            }
        }
        FieldCategory::DynamicVec {
            inner: _,
            elem_is_pod,
            elem_fixed,
            ..
        } => {
            if *elem_is_pod {
                quote! {
                    #fident: {
                        let (s, e) = #range_calc;
                        ::mmapstruct::bytemuck::cast_slice(&data[s..e])
                    }
                }
            } else if *elem_fixed {
                quote! {
                    #fident: {
                        let (s, e) = #range_calc;
                        ::mmapstruct::fixed_array::FixedArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e))
                    }
                }
            } else {
                quote! {
                    #fident: {
                        let (s, e) = #range_calc;
                        ::mmapstruct::indexed_array::IndexedArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e))
                    }
                }
            }
        }
        FieldCategory::DynamicHashMap {
            key_layout,
            val_layout,
            ..
        } => {
            let mod_name = hash_map_module_name(*key_layout, *val_layout);
            quote! {
                #fident: {
                    let (s, e) = #range_calc;
                    ::mmapstruct::hash_map::#mod_name::HashMapReader::new(data, ::mmapstruct::traits::FileRange::new(s, e))
                }
            }
        }
        FieldCategory::DynamicOption { .. } | FieldCategory::DynamicCustom => {
            quote! {
                #fident: {
                    let (s, e) = #range_calc;
                    <#ty as ::mmapstruct::traits::MmapRead>::read(data, s..e)
                }
            }
        }
        _ => unreachable!(),
    }
}

/// Generate all reader field initializers for a struct.
pub(crate) fn generate_reader_inits(
    fields: &[FieldInfo],
    layout: &StructLayout,
    struct_name: &proc_macro2::Ident,
    inline_dyn_indices: &[usize],
) -> Vec<proc_macro2::TokenStream> {
    fields
        .iter()
        .map(|fi| {
            if is_pod_field(&fi.category) {
                generate_pod_reader_init(fi, struct_name, fields)
            } else if is_inline_string_field(&fi.category) {
                generate_inline_string_reader_init(fi, struct_name, fields)
            } else if fi.is_fixed_byref()
            {
                generate_byref_reader_init(fi, struct_name, fields)
            } else {
                generate_inline_dyn_reader_init(fi, layout, struct_name, fields, inline_dyn_indices)
            }
        })
        .collect()
}

/// Generate the full `impl MmapRead` block.
pub(crate) fn generate_read_impl(
    name: &proc_macro2::Ident,
    reader_name: &proc_macro2::Ident,
    fields: &[FieldInfo],
    layout: &StructLayout,
    inline_dyn_indices: &[usize],
) -> proc_macro2::TokenStream {
    let reader_inits = generate_reader_inits(fields, layout, name, inline_dyn_indices);
    quote! {
        impl ::mmapstruct::traits::MmapRead for #name {
            type Reader<'a> = #reader_name<'a>;
            fn read<'a>(data: &'a [u8], range: std::ops::Range<usize>) -> Self::Reader<'a> {
                #reader_name { #(#reader_inits),* }
            }
        }
    }
}
