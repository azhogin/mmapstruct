//! Computes field offsets and sizes for the fixed section layout.

use quote::{format_ident, quote};

use crate::utils::*;
use crate::category::*;
use crate::field_info::*;

pub(crate) struct StructLayout {
    pub(crate) inline_dyn_count: usize,
}

pub(crate) fn compute_layout(fields: &[FieldInfo]) -> StructLayout {
    let inline_dyn_count = fields
        .iter()
        .filter(|f| {
            !is_pod_field(&f.category)
                && !is_inline_string_field(&f.category)
                && !(f.is_fixed_byref())
        })
        .count();
    StructLayout { inline_dyn_count }
}

pub(crate) fn collect_inline_dyn_indices(fields: &[FieldInfo]) -> Vec<usize> {
    fields
        .iter()
        .enumerate()
        .filter(|(_, f)| {
            !is_pod_field(&f.category)
                && !is_inline_string_field(&f.category)
                && !(f.is_fixed_byref())
        })
        .map(|(i, _)| i)
        .collect()
}

pub(crate) fn fixed_section_index(fields: &[FieldInfo], field_name: &str) -> usize {
    fields
        .iter()
        .filter(|f| is_pod_field(&f.category) || is_inline_string_field(&f.category) || f.has_byref_attr)
        .position(|f| f.name == field_name)
        .expect("field not in fixed section")
}

pub(crate) fn align_offset(offset: usize, align: usize) -> usize {
    if align == 0 {
        return offset;
    }
    (offset + align - 1) & !(align - 1)
}

/// Generate `const FO: [usize; N]` (field offsets) and `const FS: usize` (fixed section size)
/// for all fixed-section fields (pod, inline-string, byref).  Uses `std::mem::size_of`
/// for custom pod types so the compiler resolves the actual size.
pub(crate) fn generate_fixed_layout_consts(
    struct_name: &proc_macro2::Ident,
    fields: &[FieldInfo],
    inline_dyn_count: usize,
) -> proc_macro2::TokenStream {
    let fixed: Vec<&FieldInfo> = fields
        .iter()
        .filter(|f| {
            is_pod_field(&f.category)
                || is_inline_string_field(&f.category)
                || (f.is_fixed_byref())
        })
        .collect();
    let n = fixed.len();
    if n == 0 {
        let hs_expr = if inline_dyn_count > 0 {
            let ot_size = (inline_dyn_count + 1) * 8;
            quote! { #ot_size }
        } else {
            quote! { 0usize }
        };
        let fo_name = format_ident!("__MMAP_FO_{}", struct_name);
        let fs_name = format_ident!("__MMAP_FS_{}", struct_name);
        let hs_name = format_ident!("__MMAP_HS_{}", struct_name);
        return quote! {
            #[allow(non_upper_case_globals)]
            const #fo_name : [usize; 0usize] = [];
            #[allow(non_upper_case_globals)]
            const #fs_name : usize = 0usize;
            #[allow(non_upper_case_globals)]
            const #hs_name : usize = #hs_expr;
        };
    }

    let fo_name = format_ident!("__MMAP_FO_{}", struct_name);
    let fs_name = format_ident!("__MMAP_FS_{}", struct_name);
    let hs_name = format_ident!("__MMAP_HS_{}", struct_name);

    let mut fo_stmts: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut fs_stmts: Vec<proc_macro2::TokenStream> = Vec::new();

    for (i, ff) in fixed.iter().enumerate() {
        if is_pod_field(&ff.category) {
            let align = type_align_expr(&ff.ty);
            let size = type_size_expr(&ff.ty);
            fo_stmts.push(quote! { __off = (__off + #align - 1) & !(#align - 1); });
            fs_stmts.push(quote! { __off = (__off + #align - 1) & !(#align - 1); });
            fo_stmts.push(quote! { __fo[#i] = __off; });
            fo_stmts.push(quote! { __off += #size; });
            fs_stmts.push(quote! { __off += #size; });
        } else if is_inline_string_field(&ff.category) {
            if let FieldCategory::InlineString { size } = &ff.category {
                fo_stmts.push(quote! { __fo[#i] = __off; });
                fo_stmts.push(quote! { __off += #size; });
                fs_stmts.push(quote! { __off += #size; });
            }
        } else if ff.has_byref_attr {
            fo_stmts.push(quote! { __off = (__off + 3) & !3; });
            fs_stmts.push(quote! { __off = (__off + 3) & !3; });
            fo_stmts.push(quote! { __fo[#i] = __off; });
            fo_stmts.push(quote! { __off += 8; });
            fs_stmts.push(quote! { __off += 8; });
        }
    }

    fs_stmts.push(quote! { __off = (__off + 7) & !7; });

    let hs_expr = if inline_dyn_count > 0 {
        let ot_size = (inline_dyn_count + 1) * 8;
        quote! { #fs_name + #ot_size }
    } else {
        quote! { #fs_name }
    };

    quote! {
        #[allow(non_upper_case_globals)]
        const #fo_name: [usize; #n] = {
            let mut __off = 0usize;
            let mut __fo = [0usize; #n];
            #(#fo_stmts)*
            __fo
        };
        #[allow(non_upper_case_globals)]
        const #fs_name: usize = {
            let mut __off = 0usize;
            #(#fs_stmts)*
            __off
        };
        #[allow(non_upper_case_globals)]
        const #hs_name: usize = #hs_expr;
    }
}

/// Generate reader field init code for a dynamic field
pub(crate) fn gen_dyn_reader_init(
    dyn_idx: usize,
    struct_name: &proc_macro2::Ident,
    dyn_count: usize,
) -> proc_macro2::TokenStream {
    let hs_name = format_ident!("__MMAP_HS_{}", struct_name);
    let fs_name = format_ident!("__MMAP_FS_{}", struct_name);
    quote! {
        {
            let ot_start = range.start + #fs_name;
            let vs = range.start + #hs_name;
            let off0 = u64::from_le_bytes(data[ot_start + #dyn_idx * 8 .. ot_start + #dyn_idx * 8 + 8].try_into().unwrap()) as usize;
            let off1 = if #dyn_idx + 1 < #dyn_count {
                u64::from_le_bytes(data[ot_start + (#dyn_idx + 1) * 8 .. ot_start + (#dyn_idx + 1) * 8 + 8].try_into().unwrap()) as usize
            } else {
                u64::from_le_bytes(data[ot_start + #dyn_count * 8 .. ot_start + (#dyn_count + 1) * 8].try_into().unwrap()) as usize
            };
            (vs + off0, vs + off1)
        }
    }
}
