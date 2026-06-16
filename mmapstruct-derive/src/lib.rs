use proc_macro::TokenStream;
use quote::{quote, format_ident};
use syn::{
    parse_macro_input, DeriveInput, Data, Fields, Type, PathArguments, GenericArgument, Meta, LitInt,
};

use syn::punctuated::Punctuated;

/// Known primitive pod type names.
const PRIMITIVE_POD: &[&str] = &[
    "u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64",
    "f32", "f64", "bool", "usize", "isize",
];

fn is_f32_or_f64(ty: &Type) -> bool {
    type_ident_name(ty).map(|n| n == "f32" || n == "f64").unwrap_or(false)
}

fn type_ident_name(ty: &Type) -> Option<String> {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().map(|s| s.ident.to_string())
    } else {
        None
    }
}

fn is_primitive_pod(ty: &Type) -> bool {
    type_ident_name(ty)
        .map(|n| PRIMITIVE_POD.contains(&n.as_str()))
        .unwrap_or(false)
}

fn is_string_type(ty: &Type) -> bool {
    type_ident_name(ty).map(|n| n == "String").unwrap_or(false)
}

fn extract_vec_inner(ty: &Type) -> Option<Type> {
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

fn extract_option_inner(ty: &Type) -> Option<Type> {
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

fn extract_hashmap_inner(ty: &Type) -> Option<(Type, Type)> {
    if let Type::Path(tp) = ty {
        let seg = tp.path.segments.last()?;
        if seg.ident == "HashMap" {
            if let PathArguments::AngleBracketed(args) = &seg.arguments {
                let mut iter = args.args.iter();
                if let (Some(GenericArgument::Type(key)), Some(GenericArgument::Type(val))) = (iter.next(), iter.next()) {
                    return Some((key.clone(), val.clone()));
                }
            }
        }
    }
    None
}

fn has_struct_scope_attr(attrs: &[syn::Attribute]) -> bool {
    for attr in attrs {
        if !attr.path().is_ident("mmap") { continue; }
        if let Ok(metas) = attr.parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated) {
            for meta in &metas {
                if let Meta::Path(p) = &meta {
                    if p.is_ident("scope") { return true; }
                }
            }
        } else if let Ok(m) = attr.parse_args::<Meta>() {
            if let Meta::Path(p) = &m {
                if p.is_ident("scope") { return true; }
            }
        }
    }
    false
}

fn has_pod_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if let Meta::Path(p) = &a.meta {
            p.is_ident("pod")
        } else {
            false
        }
    })
}

fn has_fixed_attr(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        if let Meta::Path(p) = &a.meta {
            p.is_ident("fixed")
        } else {
            false
        }
    })
}

#[derive(Clone, Copy, PartialEq)]
enum HashMapLayout {
    Pod,
    Inlined,
}

fn detect_layout(ty: &Type) -> HashMapLayout {
    if is_primitive_pod(ty) { HashMapLayout::Pod } else { HashMapLayout::Inlined }
}

fn hash_map_module_name(key: HashMapLayout, val: HashMapLayout) -> proc_macro2::Ident {
    let k = match key { HashMapLayout::Pod => "podK", HashMapLayout::Inlined => "inlinedK" };
    let v = match val { HashMapLayout::Pod => "podV", HashMapLayout::Inlined => "inlinedV" };
    format_ident!("{}_{}", k, v)
}

fn parse_hashmap_attrs(attrs: &[syn::Attribute]) -> (Option<HashMapLayout>, Option<HashMapLayout>) {
    let mut key = None;
    let mut val = None;
    for attr in attrs {
        if !attr.path().is_ident("mmap") { continue; }
        if let Ok(metas) = attr.parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated) {
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

fn parse_mmap_attrs(attrs: &[syn::Attribute]) -> (bool, bool, Option<usize>) {
    let mut byref = false;
    let mut dedup = false;
    let mut inline_size = None;
    for attr in attrs {
        if !attr.path().is_ident("mmap") { continue; }
        // Parse comma-separated list: byref, dedup, inline(N)
        if let Ok(metas) = attr.parse_args_with(Punctuated::<Meta, syn::Token![,]>::parse_terminated) {
            for meta in &metas {
                if let Meta::Path(p) = meta {
                    if p.is_ident("byref") { byref = true; }
                    if p.is_ident("dedup") { dedup = true; }
                }
                if let Meta::List(list) = meta {
                    if list.path.is_ident("inline") {
                        if let Ok(n) = list.parse_args::<LitInt>() {
                            inline_size = Some(n.base10_parse::<usize>().unwrap());
                        }
                    }
                }
            }
        } else if let Ok(m) = attr.parse_args::<Meta>() {
            // Single meta: byref
            if let Meta::Path(p) = &m {
                if p.is_ident("byref") { byref = true; }
            }
        }
    }
    (byref, dedup, inline_size)
}

#[derive(Clone)]
enum FieldCategory {
    PodPrimitive,
    PodCustom,
    DynamicString,
    InlineString { size: usize },
    DynamicVec { inner: Type, elem_is_pod: bool, elem_fixed: bool },
    DynamicVecByRef { inner: Type, dedup: bool },
    DynamicHashMap { key_ty: Type, val_ty: Type, key_layout: HashMapLayout, val_layout: HashMapLayout },
    DynamicOption { inner: Type },
    DynamicCustom,
}

struct FieldInfo {
    name: String,
    ty: Type,
    category: FieldCategory,
    is_byref: bool,
    dedup: bool,
    ident: proc_macro2::Ident,
}

// ---------------------------------------------------------------------------
// Layout computation types
// ---------------------------------------------------------------------------

/// Complete layout of a mixed struct's fixed + inline dynamic sections.
struct StructLayout {
    inline_dyn_count: usize,
}

fn is_pod_field(category: &FieldCategory) -> bool {
    matches!(category, FieldCategory::PodPrimitive | FieldCategory::PodCustom)
}

fn is_inline_string_field(category: &FieldCategory) -> bool {
    matches!(category, FieldCategory::InlineString { .. })
}

fn classify_field(field: &syn::Field) -> FieldCategory {
    let ty = &field.ty;
    if is_primitive_pod(ty) { return FieldCategory::PodPrimitive; }
    let (is_byref, dedup, inline_size) = parse_mmap_attrs(&field.attrs);
    if let Some(size) = inline_size {
        if is_string_type(ty) {
            return FieldCategory::InlineString { size };
        }
        panic!("#[mmap(inline(N))] only supported on String fields");
    }
    if is_string_type(ty) { return FieldCategory::DynamicString; }
    if let Some(inner) = extract_vec_inner(ty) {
        if is_byref {
            return FieldCategory::DynamicVecByRef { inner, dedup };
        }
        let elem_is_pod = is_primitive_pod(&inner) || has_pod_attr(&field.attrs);
        let elem_fixed = has_fixed_attr(&field.attrs);
        return FieldCategory::DynamicVec { inner, elem_is_pod, elem_fixed };
    }
    if let Some(inner) = extract_option_inner(ty) {
        return FieldCategory::DynamicOption { inner };
    }
    if let Some((key_ty, val_ty)) = extract_hashmap_inner(ty) {
        let (key_attr, val_attr) = parse_hashmap_attrs(&field.attrs);
        let key_layout = key_attr.unwrap_or_else(|| detect_layout(&key_ty));
        let val_layout = val_attr.unwrap_or_else(|| detect_layout(&val_ty));
        return FieldCategory::DynamicHashMap { key_ty, val_ty, key_layout, val_layout };
    }
    if has_pod_attr(&field.attrs) { return FieldCategory::PodCustom; }
    FieldCategory::DynamicCustom
}

fn reader_type(fi: &FieldInfo) -> proc_macro2::TokenStream {
    match &fi.category {
        FieldCategory::PodPrimitive | FieldCategory::PodCustom => {
            let ty = &fi.ty;
            quote! { &'a #ty }
        }
        FieldCategory::DynamicString | FieldCategory::InlineString { .. } => {
            quote! { &'a str }
        }
        FieldCategory::DynamicVec { inner, elem_is_pod, elem_fixed } => {
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
        FieldCategory::DynamicHashMap { key_ty, val_ty, key_layout, val_layout } => {
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

fn known_primitive_size(ty: &Type) -> Option<usize> {
    let name = type_ident_name(ty)?;
    match name.as_str() {
        "u8" | "i8" | "bool" => Some(1),
        "u16" | "i16" => Some(2),
        "u32" | "i32" | "f32" => Some(4),
        "u64" | "i64" | "f64" | "usize" | "isize" => Some(8),
        _ => None,
    }
}

/// Expression that evaluates to `size_of::<T>()` at compile time.
fn type_size_expr(ty: &Type) -> proc_macro2::TokenStream {
    if let Some(sz) = known_primitive_size(ty) {
        quote! { #sz }
    } else {
        quote! { ::std::mem::size_of::<#ty>() }
    }
}

/// Expression that evaluates to the mmapstruct alignment of `T` (capped at 8).
fn type_align_expr(ty: &Type) -> proc_macro2::TokenStream {
    if let Some(sz) = known_primitive_size(ty) {
        let align = sz.min(8).max(1);
        quote! { #align }
    } else {
        quote! { { let a = ::std::mem::align_of::<#ty>(); if a > 8 { 8usize } else if a == 0 { 1usize } else { a } } }
    }
}

/// Generate `const FO: [usize; N]` (field offsets) and `const FS: usize` (fixed section size)
/// for all fixed-section fields (pod, inline-string, byref).  Uses `std::mem::size_of`
/// for custom pod types so the compiler resolves the actual size.
fn generate_fixed_layout_consts(
    struct_name: &proc_macro2::Ident,
    fields: &[FieldInfo],
    inline_dyn_count: usize,
) -> proc_macro2::TokenStream {
    let fixed: Vec<&FieldInfo> = fields.iter()
        .filter(|f| is_pod_field(&f.category) || is_inline_string_field(&f.category)
            || (f.is_byref && !matches!(&f.category, FieldCategory::DynamicVecByRef { .. })))
        .collect();
    let n = fixed.len();
    if n == 0 {
        // No fixed-section fields at all
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

    // Build alignment + size code for each fixed-section field
    let mut fo_stmts: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut fs_stmts: Vec<proc_macro2::TokenStream> = Vec::new();

    for (i, ff) in fixed.iter().enumerate() {
        if is_pod_field(&ff.category) {
            let align = type_align_expr(&ff.ty);
            let size = type_size_expr(&ff.ty);
            // Align
            fo_stmts.push(quote! { __off = (__off + #align - 1) & !(#align - 1); });
            fs_stmts.push(quote! { __off = (__off + #align - 1) & !(#align - 1); });
            // Store offset
            fo_stmts.push(quote! { __fo[#i] = __off; });
            // Advance
            fo_stmts.push(quote! { __off += #size; });
            fs_stmts.push(quote! { __off += #size; });
        } else if is_inline_string_field(&ff.category) {
            if let FieldCategory::InlineString { size } = &ff.category {
                fo_stmts.push(quote! { __fo[#i] = __off; });
                fo_stmts.push(quote! { __off += #size; });
                fs_stmts.push(quote! { __off += #size; });
            }
        } else if ff.is_byref {
            // ByRefRange: align 4, size 8
            fo_stmts.push(quote! { __off = (__off + 3) & !3; });
            fs_stmts.push(quote! { __off = (__off + 3) & !3; });
            fo_stmts.push(quote! { __fo[#i] = __off; });
            fo_stmts.push(quote! { __off += 8; });
            fs_stmts.push(quote! { __off += 8; });
        }
    }

    // Final alignment to 8
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

fn align_offset(offset: usize, align: usize) -> usize {
    if align == 0 { return offset; }
    (offset + align - 1) & !(align - 1)
}

/// Generate reader field init code for a dynamic field
fn gen_dyn_reader_init(
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
                // Extra entry at index dyn_count stores total inline dyn data size
                u64::from_le_bytes(data[ot_start + #dyn_count * 8 .. ot_start + (#dyn_count + 1) * 8].try_into().unwrap()) as usize
            };
            (vs + off0, vs + off1)
        }
    }
}

#[proc_macro_derive(MmapStruct, attributes(mmap, pod, fixed))]
pub fn derive_mmap_struct(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match &input.data {
        Data::Struct(ds) => handle_struct(&input, ds),
        Data::Enum(de) => handle_enum(&input, de),
        _ => panic!("MmapStruct only supports structs and enums"),
    }
}

fn analyze_fields(fields: &syn::Fields) -> (Vec<FieldInfo>, Vec<usize>, Vec<usize>, Vec<usize>, Vec<usize>, bool) {
    let named_fields = match fields {
        Fields::Named(f) => &f.named,
        _ => return (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), false),
    };

    let mut all_fields: Vec<FieldInfo> = Vec::new();
    let mut pod_indices: Vec<usize> = Vec::new();
    let mut byref_indices: Vec<usize> = Vec::new();
    let mut inline_string_indices: Vec<usize> = Vec::new();
    let mut inline_dyn_indices: Vec<usize> = Vec::new();

    for (i, field) in named_fields.iter().enumerate() {
        let category = classify_field(field);
        let (is_byref, dedup, _inline_size) = parse_mmap_attrs(&field.attrs);
        let is_pod = matches!(category, FieldCategory::PodPrimitive | FieldCategory::PodCustom);
        let is_inline_string = matches!(category, FieldCategory::InlineString { .. });

        let fi = FieldInfo {
            name: field.ident.as_ref().unwrap().to_string(),
            ty: field.ty.clone(),
            category,
            is_byref,
            dedup,
            ident: field.ident.as_ref().unwrap().clone(),
        };
        if is_pod {
            pod_indices.push(i);
        } else if is_byref && !(is_pod || is_inline_string) {
            // DynamicVecByRef is handled as inline-dyn, not as a fixed byref
            let is_vec_byref = matches!(&fi.category, FieldCategory::DynamicVecByRef { .. });
            if !is_vec_byref {
                byref_indices.push(i);
            } else {
                inline_dyn_indices.push(i);
            }
        } else if is_inline_string {
            inline_string_indices.push(i);
        } else {
            inline_dyn_indices.push(i);
        }
        all_fields.push(fi);
    }

    let has_dynamic = !byref_indices.is_empty() || !inline_dyn_indices.is_empty() || !inline_string_indices.is_empty();
    (all_fields, pod_indices, byref_indices, inline_string_indices, inline_dyn_indices, has_dynamic)
}

// ---------------------------------------------------------------------------
// Layout computation
// ---------------------------------------------------------------------------

/// Compute the number of inline-dynamic fields.
fn compute_layout(fields: &[FieldInfo]) -> StructLayout {
    let inline_dyn_count = fields.iter()
        .filter(|f| !is_pod_field(&f.category) && !is_inline_string_field(&f.category)
            && !(f.is_byref && !matches!(&f.category, FieldCategory::DynamicVecByRef { .. })))
        .count();
    StructLayout { inline_dyn_count }
}

/// Collect indices (in the all_fields array) of inline-dynamic fields.
fn collect_inline_dyn_indices(fields: &[FieldInfo]) -> Vec<usize> {
    fields.iter()
        .enumerate()
        .filter(|(_, f)| !is_pod_field(&f.category) && !is_inline_string_field(&f.category)
            && !(f.is_byref && !matches!(&f.category, FieldCategory::DynamicVecByRef { .. })))
        .map(|(i, _)| i)
        .collect()
}

// ---------------------------------------------------------------------------
// Reader generation
// ---------------------------------------------------------------------------

/// Generate the reader struct declaration (public fields with reader types).
fn generate_reader_struct(reader_name: &proc_macro2::Ident, fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    let decls: Vec<_> = fields.iter().map(|fi| {
        let fident = &fi.ident;
        let rt = reader_type(fi);
        quote! { pub #fident: #rt }
    }).collect();
    quote! { pub struct #reader_name<'a> { #(#decls),* } }
}

/// Generate reader init for a pod field (::mmapstruct::bytemuck::from_bytes).
fn generate_pod_reader_init(field: &FieldInfo, struct_name: &proc_macro2::Ident, all_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    let fident = &field.ident;
    let fo_name = format_ident!("__MMAP_FO_{}", struct_name);
    let idx = fixed_section_index(all_fields, field.name.as_str());
    let ty = &field.ty;
    quote! {
        #fident: ::mmapstruct::bytemuck::from_bytes(&data[range.start + #fo_name[#idx] .. range.start + #fo_name[#idx] + std::mem::size_of::<#ty>()])
    }
}

/// Generate reader init for an inline string field.
fn generate_inline_string_reader_init(field: &FieldInfo, struct_name: &proc_macro2::Ident, all_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
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
fn generate_byref_reader_init(field: &FieldInfo, struct_name: &proc_macro2::Ident, all_fields: &[FieldInfo]) -> proc_macro2::TokenStream {
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
fn generate_inline_dyn_reader_init(field: &FieldInfo, layout: &StructLayout, struct_name: &proc_macro2::Ident, all_fields: &[FieldInfo], inline_dyn_indices: &[usize]) -> proc_macro2::TokenStream {
    let fident = &field.ident;
    let dyn_idx = inline_dyn_indices.iter()
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
        FieldCategory::DynamicVec { elem_is_pod, elem_fixed, .. } => {
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
        FieldCategory::DynamicHashMap { key_layout, val_layout, .. } => {
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
fn generate_reader_inits(fields: &[FieldInfo], layout: &StructLayout, struct_name: &proc_macro2::Ident, inline_dyn_indices: &[usize]) -> Vec<proc_macro2::TokenStream> {
    fields.iter().map(|fi| {
        if is_pod_field(&fi.category) {
            generate_pod_reader_init(fi, struct_name, fields)
        } else if is_inline_string_field(&fi.category) {
            generate_inline_string_reader_init(fi, struct_name, fields)
        } else if fi.is_byref && !matches!(&fi.category, FieldCategory::DynamicVecByRef { .. }) {
            generate_byref_reader_init(fi, struct_name, fields)
        } else {
            generate_inline_dyn_reader_init(fi, layout, struct_name, fields, inline_dyn_indices)
        }
    }).collect()
}

/// Generate the full `impl MmapRead` block.
fn generate_read_impl(name: &proc_macro2::Ident, reader_name: &proc_macro2::Ident, fields: &[FieldInfo], layout: &StructLayout, inline_dyn_indices: &[usize]) -> proc_macro2::TokenStream {
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

// ---------------------------------------------------------------------------
// Writer generation
// ---------------------------------------------------------------------------

/// Generate the fixed-section writer calls (pod + inline string + byref).
fn generate_fixed_writes(fields: &[FieldInfo]) -> Vec<proc_macro2::TokenStream> {
    fields.iter().map(|fi| {
        let fident = &fi.ident;
        if is_pod_field(&fi.category) {
            quote! { ctx.write_pod(&self.#fident); }
        } else if is_inline_string_field(&fi.category) {
            if let FieldCategory::InlineString { size } = &fi.category {
                quote! { ctx.write_inline_string(&self.#fident, #size); }
            } else {
                unreachable!()
            }
        } else if fi.is_byref && !matches!(&fi.category, FieldCategory::DynamicVecByRef { .. }) {
            let dedup_literal = fi.dedup;
            match &fi.category {
                FieldCategory::DynamicOption { .. } => {
                    quote! {
                        if let Some(val) = &self.#fident {
                            ctx.write_byref(val, start, #dedup_literal);
                        } else {
                            ctx.write_pod(&::mmapstruct::traits::ByRefRange { begin: 0, end: 0 });
                        }
                    }
                }
                _ => {
                    quote! {
                        ctx.write_byref(&self.#fident, start, #dedup_literal);
                    }
                }
            }
        } else {
            quote! {}
        }
    }).collect()
}

/// Generate the inline-dynamic writer blocks (pod array, string, fixed array, etc).
fn generate_inline_dyn_writes(fields: &[FieldInfo], inline_dyn_indices: &[usize]) -> Vec<proc_macro2::TokenStream> {
    inline_dyn_indices.iter().map(|&idx| {
        let fi = &fields[idx];
        let fident = &fi.ident;
        match &fi.category {
            FieldCategory::DynamicVec { inner, elem_is_pod, elem_fixed } if *elem_is_pod => {
                quote! {
                    {
                        ctx.align_from(vs, std::mem::align_of::<#inner>());
                        let before = (ctx.current_offset() - vs) as u64;
                        dyn_offsets.push(before);
                        ::mmapstruct::pod_array::write_pod_array(&self.#fident, ctx);
                    }
                }
            }
            _ => {
                let write_expr = match &fi.category {
                    FieldCategory::DynamicString => {
                        quote! { ::mmapstruct::traits::MmapWrite::write(&self.#fident, ctx); }
                    }
                    FieldCategory::DynamicVecByRef { dedup, .. } => {
                        quote! { ::mmapstruct::byref_array::write_byref_array(&self.#fident, ctx, start, #dedup); }
                    }
                    FieldCategory::DynamicVec { elem_fixed, .. } => {
                        if *elem_fixed {
                            quote! { ::mmapstruct::fixed_array::write_fixed_array(&self.#fident, ctx); }
                        } else {
                            quote! { ::mmapstruct::indexed_array::write_indexed_array(&self.#fident, ctx); }
                        }
                    }
                    FieldCategory::DynamicHashMap { key_layout, val_layout, .. } => {
                        let mod_name = hash_map_module_name(*key_layout, *val_layout);
                        quote! { ::mmapstruct::hash_map::#mod_name::write_hash_map(&self.#fident, ctx); }
                    }
                    _ => {
                        quote! { ::mmapstruct::traits::MmapWrite::write(&self.#fident, ctx); }
                    }
                };
                quote! {
                    {
                        let before = (ctx.current_offset() - vs) as u64;
                        dyn_offsets.push(before);
                        #write_expr
                    }
                }
            }
        }
    }).collect()
}

/// Compute maximum field alignment for a list of fields.
fn compute_max_field_align(fields: &[FieldInfo]) -> usize {
    let mut align = 1usize;
    for fi in fields {
        if is_pod_field(&fi.category) {
            let sz = known_primitive_size(&fi.ty).unwrap_or(8);
            let al = if sz <= 1 { 1 } else { sz.min(8) };
            if al > align { align = al; }
        } else if fi.is_byref {
            // ByRefRange has u32 fields → alignment 4
            if 4 > align { align = 4; }
        }
        // inline strings have alignment 1 (bytes), inline dyn handled via offset table
    }
    align
}

/// Generate the full `impl MmapWrite` block.
/// If `scoped` is true, wraps the body with `scope_begin()` / `scope_end()` for local postponed processing,
/// and pads to max field alignment after scope_end.
fn generate_write_impl(name: &proc_macro2::Ident, fields: &[FieldInfo], layout: &StructLayout, inline_dyn_indices: &[usize], scoped: bool) -> proc_macro2::TokenStream {
    let fixed_writes = generate_fixed_writes(fields);
    let inline_dyn_writes = generate_inline_dyn_writes(fields, inline_dyn_indices);
    let inline_dyn_count = layout.inline_dyn_count;

    let inner_body = if inline_dyn_count > 0 {
        quote! {
            let start = ctx.current_offset();

            // Fixed section: pod fields + inline strings + byref placeholders
            #(#fixed_writes)*
            // Align from start to 8 for offset table (ensures fixed_section_size matches layout)
            ctx.align_from(start, 8);

            // Offset table: entries per field + extra entry for total inline dyn data size
            let ot_start = ctx.current_offset();
            ctx.write_padding((#inline_dyn_count + 1) * 8);
            let vs = ctx.current_offset();
            let mut dyn_offsets: Vec<u64> = Vec::new();
            #(#inline_dyn_writes)*
            let total_dyn_size = (ctx.current_offset() - vs) as u64;
            for (i, &off) in dyn_offsets.iter().enumerate() {
                ctx.write_u64_le_at(off, ot_start + i * 8);
            }
            // Write total inline dyn data size to the extra entry
            ctx.write_u64_le_at(total_dyn_size, ot_start + #inline_dyn_count * 8);
        }
    } else {
        quote! {
            let start = ctx.current_offset();

            // Fixed section: pod fields + inline strings + byref placeholders
            #(#fixed_writes)*
            // Align from start to 8 (matches layout computation)
            ctx.align_from(start, 8);
        }
    };

    let body = if scoped {
        let max_align = compute_max_field_align(fields);
        quote! {
            let _saved = ctx.scope_begin();
            #inner_body
            ctx.scope_end(_saved);
            // Pad to max field alignment so next element in array starts aligned
            ctx.align_to(#max_align);
        }
    } else {
        inner_body
    };

    quote! {
        impl ::mmapstruct::traits::MmapWrite for #name {
            fn write<'data>(&'data self, ctx: &mut ::mmapstruct::WriteContext<'data>) {
                #body
            }
        }
    }
}

// ---------------------------------------------------------------------------
// From / Default generation
// ---------------------------------------------------------------------------

/// Generate the `impl From<Reader> for Struct` block.
fn generate_from_impl(name: &proc_macro2::Ident, reader_name: &proc_macro2::Ident, fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    let conversions: Vec<_> = fields.iter().map(|fi| {
        let fident = &fi.ident;
        let expr = match &fi.category {
            FieldCategory::PodPrimitive | FieldCategory::PodCustom => {
                quote! { *r.#fident }
            }
            FieldCategory::DynamicOption { .. } => {
                quote! { r.#fident.map(|v| v.into()) }
            }
            _ => {
                quote! { r.#fident.into() }
            }
        };
        quote! { #fident: #expr }
    }).collect();

    quote! {
        impl<'a> From<#reader_name<'a>> for #name {
            fn from(r: #reader_name<'a>) -> Self {
                Self { #(#conversions),* }
            }
        }
    }
}

/// Generate `impl ReaderOwned` for a mixed struct (uses the generated `From` impl).
fn generate_reader_owned_impl(name: &proc_macro2::Ident, reader_name: &proc_macro2::Ident, _fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    quote! {
        impl ::mmapstruct::traits::ReaderOwned for #name {
            fn into_owned(reader: Self::Reader<'_>) -> Self {
                reader.into()
            }
        }
    }
}

/// Generate `impl ReaderOrd` for a mixed struct (field-by-field comparison).
fn generate_reader_ord_impl(name: &proc_macro2::Ident, reader_name: &proc_macro2::Ident, fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    // Only generate for structs with simple field types: pod, string, option of simple,
    // or nested structs. Skip Vec/HashMap/ByRefVec/complex custom fields because
    // those types are not used as hashmap keys and may not implement Ord.
    let is_simple_field = |cat: &FieldCategory| -> bool {
        match cat {
            FieldCategory::PodPrimitive
            | FieldCategory::PodCustom
            | FieldCategory::DynamicString
            | FieldCategory::InlineString { .. } => true,
            FieldCategory::DynamicOption { inner: _ } => true, // handled recursively
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
fn generate_default_impl(name: &proc_macro2::Ident, fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    let defaults: Vec<_> = fields.iter().map(|fi| {
        let fident = &fi.ident;
        quote! { #fident: Default::default() }
    }).collect();

    quote! {
        impl Default for #name {
            fn default() -> Self {
                Self { #(#defaults),* }
            }
        }
    }
}

/// Generate `impl LayoutHash` for a struct.
///
/// Hash includes:
/// - Per-field hash (type identity + encoding modifiers)
/// - Layout constants (FS, HS)
fn generate_layout_hash_impl(name: &proc_macro2::Ident, fields: &[FieldInfo]) -> proc_macro2::TokenStream {
    let field_hashes: Vec<_> = fields.iter().map(|fi| {
        let ty = &fi.ty;
        let base = quote! { <#ty as ::mmapstruct::traits::LayoutHash>::HASH };

        if fi.is_byref {
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

    quote! {
        impl ::mmapstruct::traits::LayoutHash for #name {
            const HASH: u64 = ::mmapstruct::traits::combine_hashes(&[
                #(#field_hashes),*,
                #fs_name as u64,
                #hs_name as u64,
            ]);
        }
    }
}

#[cfg(feature = "dyn")]
fn generate_schema_impl(
    name: &proc_macro2::Ident,
    fields: &[FieldInfo],
    inline_dyn_indices: &[usize],
) -> proc_macro2::TokenStream {
    let schema_name = format_ident!("__mmap_SCHEMA_{}", name);
    let field_statics_name = format_ident!("__mmap_FIELDS_{}", name);

    // Determine inline_dyn index for each field (None for pod/inline-string/byref)
    let mut next_dyn_idx = 0usize;
    let field_dyn_indices: Vec<Option<usize>> = fields.iter().map(|fi| {
        if is_pod_field(&fi.category) || is_inline_string_field(&fi.category)
            || (fi.is_byref && !matches!(&fi.category, FieldCategory::DynamicVecByRef { .. }))
        {
            None
        } else {
            let idx = next_dyn_idx;
            next_dyn_idx += 1;
            Some(idx)
        }
    }).collect();

    // Generate per-field value_type statics + any resolver functions
    let mut value_type_statics = Vec::new();
    let mut resolver_fns = Vec::new();

    for fi in fields.iter() {
        let vt_name = format_ident!("__mmap_VT_{}_{}", name, fi.ident);
        let (vt_expr, extra) = field_value_type(name, fi, &fi.ident);
        value_type_statics.push(quote! {
            #[allow(non_upper_case_globals)]
            static #vt_name: ::mmapstruct::schema::DynValueType = #vt_expr;
        });
        resolver_fns.extend(extra);
    }

    // Build field descriptor tokens
    let field_descs: Vec<_> = fields.iter().enumerate().map(|(i, fi)| {
        let vt_name = format_ident!("__mmap_VT_{}_{}", name, fi.ident);
        let field_name = &fi.name;
        let off = fields_field_offset(name, fields, fi);
        let dyn_idx = field_dyn_indices[i];
        let inline_dyn_idx_tok = match dyn_idx {
            Some(idx) => quote! { Some(#idx) },
            None => quote! { None },
        };
        let is_byref = fi.is_byref;
        quote! {
            ::mmapstruct::schema::FieldDescriptor {
                name: #field_name,
                offset_in_fixed: #off,
                inline_dyn_index: #inline_dyn_idx_tok,
                value_type: &#vt_name,
                is_byref: #is_byref,
            }
        }
    }).collect();

    let inline_dyn_count = inline_dyn_indices.len();
    let fs_name = format_ident!("__MMAP_FS_{}", name);
    let hs_name = format_ident!("__MMAP_HS_{}", name);
    let name_str = name.to_string();
    let field_count = field_descs.len();

    // Flatten Vec<TokenStream> into single TokenStream for quote! interpolation
    let resolver_stream = resolver_fns.into_iter().collect::<proc_macro2::TokenStream>();
    let vt_stream = value_type_statics.into_iter().collect::<proc_macro2::TokenStream>();

    let root_vt_name = format_ident!("__mmap_ROOT_VT_{}", name);
    let root_resolve_name = format_ident!("__mmap_resolve_ROOT_{}", name);
    let root_resolve_fn = quote! {
        #[allow(non_snake_case)]
        fn #root_resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
            &#schema_name
        }
    };
    let root_vt_static = quote! {
        #[allow(non_upper_case_globals)]
        static #root_vt_name: ::mmapstruct::schema::DynValueType =
            ::mmapstruct::schema::DynValueType::Struct(#root_resolve_name);
    };

    quote! {
        #resolver_stream
        #vt_stream
        #[allow(non_upper_case_globals)]
        static #field_statics_name: [::mmapstruct::schema::FieldDescriptor; #field_count] = [#(#field_descs),*];
        #[allow(non_upper_case_globals)]
        static #schema_name: ::mmapstruct::schema::StructSchema = ::mmapstruct::schema::StructSchema {
            name: #name_str,
            fields: &#field_statics_name,
            fixed_section_size: #fs_name as u64,
            header_size: #hs_name as u64,
            inline_dyn_count: #inline_dyn_count,
        };
        impl ::mmapstruct::schema::GetSchema for #name {
            fn get_schema() -> &'static ::mmapstruct::schema::StructSchema {
                &#schema_name
            }
            fn root_value_type() -> &'static ::mmapstruct::schema::DynValueType {
                &#root_vt_name
            }
        }
        #root_resolve_fn
        #root_vt_static
    }
}

/// Returns (DynValueType expression, resolver functions) for a field.
#[cfg(feature = "dyn")]
fn field_value_type(
    struct_name: &proc_macro2::Ident,
    fi: &FieldInfo,
    field_ident: &proc_macro2::Ident,
) -> (proc_macro2::TokenStream, Vec<proc_macro2::TokenStream>) {
    match &fi.category {
        FieldCategory::PodPrimitive => {
            let ty_str = type_ident_name(&fi.ty).unwrap_or_default();
            let variant = match ty_str.as_str() {
                "u32" => quote! { ::mmapstruct::schema::DynValueType::U32 },
                "i32" => quote! { ::mmapstruct::schema::DynValueType::I32 },
                "u64" => quote! { ::mmapstruct::schema::DynValueType::U64 },
                "i64" => quote! { ::mmapstruct::schema::DynValueType::I64 },
                "f32" => quote! { ::mmapstruct::schema::DynValueType::F32 },
                "f64" => quote! { ::mmapstruct::schema::DynValueType::F64 },
                "bool" => quote! { ::mmapstruct::schema::DynValueType::Bool },
                "u16" => quote! { ::mmapstruct::schema::DynValueType::U16 },
                "i16" => quote! { ::mmapstruct::schema::DynValueType::I16 },
                "u8" => quote! { ::mmapstruct::schema::DynValueType::U8 },
                "i8" => quote! { ::mmapstruct::schema::DynValueType::I8 },
                "usize" => quote! { ::mmapstruct::schema::DynValueType::Usize },
                "isize" => quote! { ::mmapstruct::schema::DynValueType::Isize },
                _ => quote! { ::mmapstruct::schema::DynValueType::U32 },
            };
            (variant, Vec::new())
        }
        FieldCategory::PodCustom => {
            let ty = &fi.ty;
            let resolve_name = format_ident!("__mmap_resolve_{}_{}", struct_name, field_ident);
            let resolver = quote! {
                #[allow(non_snake_case)]
                fn #resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
                    <#ty as ::mmapstruct::schema::GetSchema>::get_schema()
                }
            };

            // Resolver for non-pod custom types
            (quote! { ::mmapstruct::schema::DynValueType::Struct(#resolve_name) }, vec![resolver])
        }
        FieldCategory::DynamicString => {
            (quote! { ::mmapstruct::schema::DynValueType::Str(None) }, Vec::new())
        }
        FieldCategory::InlineString { size } => {
            let s = *size as u64;
            (quote! { ::mmapstruct::schema::DynValueType::Str(Some(#s)) }, Vec::new())
        }
        FieldCategory::DynamicVec { inner, elem_is_pod, elem_fixed } => {
            let arr_name = format_ident!("__mmap_ARR_{}_{}", struct_name, field_ident);
            let elem_vt_name = format_ident!("__mmap_VT_{}_{}_elem", struct_name, field_ident);

                let (elem_vt_expr, resolver_fns) = if *elem_is_pod {
                let ty_str = type_ident_name(inner).unwrap_or_default();
                let (variant, extra) = match ty_str.as_str() {
                    "u32" => (quote! { ::mmapstruct::schema::DynValueType::U32 }, Vec::new()),
                    "i32" => (quote! { ::mmapstruct::schema::DynValueType::I32 }, Vec::new()),
                    "u64" => (quote! { ::mmapstruct::schema::DynValueType::U64 }, Vec::new()),
                    "i64" => (quote! { ::mmapstruct::schema::DynValueType::I64 }, Vec::new()),
                    "f32" => (quote! { ::mmapstruct::schema::DynValueType::F32 }, Vec::new()),
                    "f64" => (quote! { ::mmapstruct::schema::DynValueType::F64 }, Vec::new()),
                    "bool" => (quote! { ::mmapstruct::schema::DynValueType::Bool }, Vec::new()),
                    "u16" => (quote! { ::mmapstruct::schema::DynValueType::U16 }, Vec::new()),
                    "i16" => (quote! { ::mmapstruct::schema::DynValueType::I16 }, Vec::new()),
                    "u8" => (quote! { ::mmapstruct::schema::DynValueType::U8 }, Vec::new()),
                    "i8" => (quote! { ::mmapstruct::schema::DynValueType::I8 }, Vec::new()),
                    "usize" => (quote! { ::mmapstruct::schema::DynValueType::Usize }, Vec::new()),
                    "isize" => (quote! { ::mmapstruct::schema::DynValueType::Isize }, Vec::new()),
                    _ => {
                        let resolve_name = format_ident!("__mmap_resolve_{}_{}_elem", struct_name, field_ident);
                        let resolver = quote! {
                            #[allow(non_snake_case)]
                            fn #resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
                                <#inner as ::mmapstruct::schema::GetSchema>::get_schema()
                            }
                        };
                        (quote! { ::mmapstruct::schema::DynValueType::Struct(#resolve_name) }, vec![resolver])
                    },
                };
                (variant, extra)
            } else if is_string_type(inner) {
                (quote! { ::mmapstruct::schema::DynValueType::Str(None) }, Vec::new())
            } else {
                let resolve_name = format_ident!("__mmap_resolve_{}_{}_elem", struct_name, field_ident);
                let resolver = quote! {
                    #[allow(non_snake_case)]
                    fn #resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
                        <#inner as ::mmapstruct::schema::GetSchema>::get_schema()
                    }
                };
                (quote! { ::mmapstruct::schema::DynValueType::Struct(#resolve_name) }, vec![resolver])
            };

            let layout_tok = if *elem_is_pod {
                let stride_expr = type_size_expr(inner);
                quote! { ::mmapstruct::schema::ArrayLayout::Pod { stride: #stride_expr as u64 } }
            } else if *elem_fixed {
                let stride_expr = type_size_expr(inner);
                quote! { ::mmapstruct::schema::ArrayLayout::Fixed { stride: #stride_expr as u64 } }
            } else {
                quote! { ::mmapstruct::schema::ArrayLayout::Indexed }
            };

            let arr_static = quote! {
                #[allow(non_upper_case_globals)]
                static #elem_vt_name: ::mmapstruct::schema::DynValueType = #elem_vt_expr;
                #[allow(non_upper_case_globals)]
                static #arr_name: ::mmapstruct::schema::ArraySchema = ::mmapstruct::schema::ArraySchema {
                    count_offset: 0u64,
                    elem_type: &#elem_vt_name,
                    layout: #layout_tok,
                };
            };

            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Array(&#arr_name) };
            let mut all = resolver_fns;
            all.push(arr_static);
            (vt_expr, all)
        }
        FieldCategory::DynamicVecByRef { inner, .. } => {
            let arr_name = format_ident!("__mmap_ARR_{}_{}", struct_name, field_ident);
            let elem_vt_name = format_ident!("__mmap_VT_{}_{}_elem", struct_name, field_ident);

            let (inner_vt_expr, resolver_fns) = inner_dyn_value_type(struct_name, field_ident, inner);

            let elem_byref_name = format_ident!("__mmap_VT_{}_{}_byref", struct_name, field_ident);
            let arr_static = quote! {
                #[allow(non_upper_case_globals)]
                static #elem_vt_name: ::mmapstruct::schema::DynValueType = #inner_vt_expr;
                #[allow(non_upper_case_globals)]
                static #elem_byref_name: ::mmapstruct::schema::DynValueType = ::mmapstruct::schema::DynValueType::ByRef(&#elem_vt_name);
                #[allow(non_upper_case_globals)]
                static #arr_name: ::mmapstruct::schema::ArraySchema = ::mmapstruct::schema::ArraySchema {
                    count_offset: 0u64,
                    elem_type: &#elem_byref_name,
                    layout: ::mmapstruct::schema::ArrayLayout::Indexed,
                };
            };

            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Array(&#arr_name) };
            let mut all = resolver_fns;
            all.push(arr_static);
            (vt_expr, all)
        }
        FieldCategory::DynamicOption { inner } => {
            let opt_name = format_ident!("__mmap_OPT_{}_{}", struct_name, field_ident);
            let (inner_vt, extra) = inner_dyn_value_type(struct_name, field_ident, inner);
            let opt_static = quote! {
                #[allow(non_upper_case_globals)]
                static #opt_name: ::mmapstruct::schema::DynValueType = #inner_vt;
            };
            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Option(&#opt_name) };
            (vt_expr, {
                let mut all = extra;
                all.push(opt_static);
                all
            })
        }
        FieldCategory::DynamicHashMap { key_ty, val_ty, key_layout, val_layout } => {
            let map_schema_name = format_ident!("__mmap_HM_{}_{}", struct_name, field_ident);
            let key_vt_name = format_ident!("__mmap_HM_{}_{}_key", struct_name, field_ident);
            let val_vt_name = format_ident!("__mmap_HM_{}_{}_val", struct_name, field_ident);
            let (key_vt, key_extra) = inner_dyn_value_type(struct_name, &format_ident!("{}_{}_key", field_ident, ""), key_ty);
            let (val_vt, val_extra) = inner_dyn_value_type(struct_name, &format_ident!("{}_{}_val", field_ident, ""), val_ty);
            let key_is_pod = matches!(key_layout, HashMapLayout::Pod);
            let val_is_pod = matches!(val_layout, HashMapLayout::Pod);
            let map_schema_static = quote! {
                #[allow(non_upper_case_globals)]
                static #key_vt_name: ::mmapstruct::schema::DynValueType = #key_vt;
                #[allow(non_upper_case_globals)]
                static #val_vt_name: ::mmapstruct::schema::DynValueType = #val_vt;
                #[allow(non_upper_case_globals)]
                static #map_schema_name: ::mmapstruct::schema::HashMapSchema = ::mmapstruct::schema::HashMapSchema {
                    key_type: &#key_vt_name,
                    value_type: &#val_vt_name,
                    key_is_pod: #key_is_pod,
                    val_is_pod: #val_is_pod,
                };
            };
            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::HashMap(&#map_schema_name) };
            let mut all: Vec<_> = key_extra.into_iter().chain(val_extra).collect();
            all.push(map_schema_static);
            (vt_expr, all)
        }
        FieldCategory::DynamicCustom => {
            let ty = &fi.ty;
            let resolve_name = format_ident!("__mmap_resolve_{}_{}", struct_name, field_ident);
            let resolver = quote! {
                fn #resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
                    <#ty as ::mmapstruct::schema::GetSchema>::get_schema()
                }
            };
            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Struct(#resolve_name) };
            (vt_expr, vec![resolver])
        }
    }
}

/// Generate inner DynValueType for Option's inner type or similar nested types.
#[cfg(feature = "dyn")]
fn inner_dyn_value_type(
    struct_name: &proc_macro2::Ident,
    field_ident: &proc_macro2::Ident,
    inner_ty: &syn::Type,
) -> (proc_macro2::TokenStream, Vec<proc_macro2::TokenStream>) {
    if is_primitive_pod(inner_ty) {
        let ty_str = type_ident_name(inner_ty).unwrap_or_default();
        let variant = match ty_str.as_str() {
            "u32" => quote! { ::mmapstruct::schema::DynValueType::U32 },
            "i32" => quote! { ::mmapstruct::schema::DynValueType::I32 },
            "u64" => quote! { ::mmapstruct::schema::DynValueType::U64 },
            "i64" => quote! { ::mmapstruct::schema::DynValueType::I64 },
            "f32" => quote! { ::mmapstruct::schema::DynValueType::F32 },
            "f64" => quote! { ::mmapstruct::schema::DynValueType::F64 },
            "bool" => quote! { ::mmapstruct::schema::DynValueType::Bool },
            "u16" => quote! { ::mmapstruct::schema::DynValueType::U16 },
            "i16" => quote! { ::mmapstruct::schema::DynValueType::I16 },
            "u8" => quote! { ::mmapstruct::schema::DynValueType::U8 },
            "i8" => quote! { ::mmapstruct::schema::DynValueType::I8 },
            "usize" => quote! { ::mmapstruct::schema::DynValueType::Usize },
            "isize" => quote! { ::mmapstruct::schema::DynValueType::Isize },
            _ => quote! { ::mmapstruct::schema::DynValueType::U32 },
        };
        return (variant, Vec::new());
    }
    if is_string_type(inner_ty) {
        return (quote! { ::mmapstruct::schema::DynValueType::Str(None) }, Vec::new());
    }
    // Assume mixed struct — generate fn pointer to GetSchema
    let resolve_name = format_ident!("__mmap_resolve_{}_{}_inner", struct_name, field_ident);
    let resolver = quote! {
        #[allow(non_snake_case)]
        fn #resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
            <#inner_ty as ::mmapstruct::schema::GetSchema>::get_schema()
        }
    };
    let vt = quote! { ::mmapstruct::schema::DynValueType::Struct(#resolve_name) };
    (vt, vec![resolver])
}

/// Find a field's byte offset within the fixed section.
#[cfg(feature = "dyn")]
fn fields_field_offset(name: &proc_macro2::Ident, fields: &[FieldInfo], fi: &FieldInfo) -> proc_macro2::TokenStream {
    let fo_name = format_ident!("__MMAP_FO_{}", name);
    if is_pod_field(&fi.category) || is_inline_string_field(&fi.category)
        || (fi.is_byref && !matches!(&fi.category, FieldCategory::DynamicVecByRef { .. }))
    {
        let idx = fixed_section_index(fields, fi.name.as_str());
        quote! { #fo_name[#idx] as u64 }
    } else {
        quote! { 0u64 }
    }
}

fn handle_struct(input: &DeriveInput, ds: &syn::DataStruct) -> TokenStream {
    let name = &input.ident;
    let reader_name = format_ident!("{}Reader", name);
    let is_scoped = has_struct_scope_attr(&input.attrs);

    let (all_fields, .., has_dynamic) = analyze_fields(&ds.fields);

    if !has_dynamic {
        let layout = compute_layout(&all_fields);
        let fixed_layout_consts = generate_fixed_layout_consts(name, &all_fields, layout.inline_dyn_count);

        // Pod structs implement MmapPod + LayoutHash
        let pod_impl = quote! { impl ::mmapstruct::traits::MmapPod for #name {} };
        let layout_hash_impl = generate_layout_hash_impl(name, &all_fields);

        // Schema + GetSchema for DSL access (only with feature = "dyn")
        let schema_impl = {
            #[cfg(feature = "dyn")]
            { generate_schema_impl(name, &all_fields, &[]) }
            #[cfg(not(feature = "dyn"))]
            { quote! {} }
        };

        return (quote! {
            #fixed_layout_consts
            #pod_impl
            #layout_hash_impl
            #schema_impl
        }).into();
    }

    let layout = compute_layout(&all_fields);
    let inline_dyn_indices = collect_inline_dyn_indices(&all_fields);

    // Compile‑time layout consts (uses std::mem::size_of for custom pod types)
    let fixed_layout_consts = generate_fixed_layout_consts(name, &all_fields, layout.inline_dyn_count);

    let reader_struct = generate_reader_struct(&reader_name, &all_fields);
    let read_impl = generate_read_impl(name, &reader_name, &all_fields, &layout, &inline_dyn_indices);
    let write_impl = generate_write_impl(name, &all_fields, &layout, &inline_dyn_indices, is_scoped);
    let from_impl = generate_from_impl(name, &reader_name, &all_fields);
    let reader_owned_impl = generate_reader_owned_impl(name, &reader_name, &all_fields);
    let reader_ord_impl = generate_reader_ord_impl(name, &reader_name, &all_fields);
    let default_impl = generate_default_impl(name, &all_fields);
    let layout_hash_impl = generate_layout_hash_impl(name, &all_fields);

    let schema_impl = {
        #[cfg(feature = "dyn")]
        { generate_schema_impl(name, &all_fields, &inline_dyn_indices) }
        #[cfg(not(feature = "dyn"))]
        { quote! {} }
    };

    (quote! {
        #fixed_layout_consts
        #layout_hash_impl
        #reader_struct
        #read_impl
        #write_impl
        #from_impl
        #reader_owned_impl
        #reader_ord_impl
        #default_impl
        #schema_impl
    }).into()
}

/// Return the index of `field_name` in the fixed‑section field list (pod, inline‑string, byref).
fn fixed_section_index(fields: &[FieldInfo], field_name: &str) -> usize {
    fields.iter()
        .filter(|f| is_pod_field(&f.category) || is_inline_string_field(&f.category) || f.is_byref)
        .position(|f| f.name == field_name)
        .expect("field not in fixed section")
}

fn handle_enum(input: &DeriveInput, de: &syn::DataEnum) -> TokenStream {
    let name = &input.ident;
    let reader_name = format_ident!("{}Reader", name);

    // Per-variant generated code
    struct VarGen {
        ident: proc_macro2::Ident,
        field_idents: Vec<proc_macro2::Ident>,
        reader_fields: Vec<proc_macro2::TokenStream>,
        reader_init: proc_macro2::TokenStream,
        write_body: proc_macro2::TokenStream,
        from_body: proc_macro2::TokenStream,
        default_body: proc_macro2::TokenStream,
        layout_hash: proc_macro2::TokenStream,
    }

    let mut vars = Vec::new();

    for (disc, variant) in de.variants.iter().enumerate() {
        let vident = &variant.ident;

        if variant.fields.is_empty() {
            // Unit variant
            vars.push(VarGen {
                ident: vident.clone(),
                field_idents: Vec::new(),
                reader_fields: Vec::new(),
                reader_init: quote! { #reader_name::#vident },
                write_body: quote! { ctx.write_u32_le(#disc as u32); },
                from_body: quote! { #name::#vident },
                default_body: if disc == 0 { quote! { #name::#vident } } else { quote! { unreachable!() } },
                layout_hash: quote! { #disc as u64 },
            });
            continue;
        }

        let named = match &variant.fields {
            Fields::Named(f) => &f.named,
            _ => panic!("MmapStruct enum requires named variant fields"),
        };

        // Analyze fields
        let mut all_fields: Vec<FieldInfo> = Vec::new();
        let mut pod_indices = Vec::new();
        let mut byref_indices = Vec::new();
        let mut inline_string_indices = Vec::new();
        let mut inline_dyn_indices = Vec::new();

        for (i, field) in named.iter().enumerate() {
        let category = classify_field(field);
        let (is_byref, dedup, _inline_size) = parse_mmap_attrs(&field.attrs);
        let is_pod = matches!(category, FieldCategory::PodPrimitive | FieldCategory::PodCustom);
        let is_inline_string = matches!(category, FieldCategory::InlineString { .. });

        let fi = FieldInfo {
            name: field.ident.as_ref().unwrap().to_string(),
            ty: field.ty.clone(),
            category,
                is_byref,
                dedup,
                ident: field.ident.as_ref().unwrap().clone(),
            };
            if is_pod { pod_indices.push(i); }
            else if is_byref { byref_indices.push(i); }
            else if is_inline_string { inline_string_indices.push(i); }
            else { inline_dyn_indices.push(i); }
            all_fields.push(fi);
        }

        let has_dynamic = !byref_indices.is_empty() || !inline_dyn_indices.is_empty() || !inline_string_indices.is_empty();

        // Compute max alignment of variant fields
        let mut variant_data_align = 1usize;
        for fi in &all_fields {
            let is_pod = matches!(fi.category, FieldCategory::PodPrimitive | FieldCategory::PodCustom);
            if is_pod {
                let sz = known_primitive_size(&fi.ty).unwrap_or(8);
                let al = if sz == 1 { 1 } else { sz.min(8).max(1) };
                if al > variant_data_align { variant_data_align = al; }
            } else if fi.is_byref {
                // ByRefRange alignment is 4
                if 4 > variant_data_align { variant_data_align = 4; }
            }
        }
        // Align variant data start after 4-byte discriminant
        let variant_data_offset = align_offset(4, variant_data_align);

        // Layout
        let mut fixed_offset = 0usize;
        let mut pod_layout: Vec<(String, usize, usize)> = Vec::new();
        let mut byref_layout: Vec<(String, usize)> = Vec::new();
        for fi in &all_fields {
            let is_pod = matches!(fi.category, FieldCategory::PodPrimitive | FieldCategory::PodCustom);
            if is_pod {
                let sz = known_primitive_size(&fi.ty).unwrap_or(8);
                let al = if sz == 1 { 1 } else { sz.min(8).max(1) };
                fixed_offset = align_offset(fixed_offset, al);
                pod_layout.push((fi.name.clone(), fixed_offset, sz));
                fixed_offset += sz;
            } else if let FieldCategory::InlineString { size } = &fi.category {
                pod_layout.push((fi.name.clone(), fixed_offset, *size));
                fixed_offset += size;
            } else if fi.is_byref {
                fixed_offset = align_offset(fixed_offset, 4);
                byref_layout.push((fi.name.clone(), fixed_offset));
                fixed_offset += 8;
            }
        }
        fixed_offset = align_offset(fixed_offset, 8);
        let fixed_sz = fixed_offset;
        let dyn_cnt = inline_dyn_indices.len();
        // +1 for total inline dyn data size entry at end of offset table (only if there are inline dyn fields)
        let hdr_sz = fixed_sz + if dyn_cnt > 0 { (dyn_cnt + 1) * 8 } else { 0 };

        // Reader field decls (no pub — enum variant fields can't have visibility)
        let rfields: Vec<_> = all_fields.iter().map(|fi| {
            let id = &fi.ident;
            let rt = reader_type(fi);
            quote! { #id: #rt }
        }).collect();

        // Reader field inits (range.start + variant_data_offset for discriminant offset)
        let rs = quote! { range.start + #variant_data_offset };
        // Extra entry at index dyn_cnt stores total inline dyn data size
        let rs_ot = quote! { #rs + #fixed_sz };
        let field_end_offset = quote! { u64::from_le_bytes(data[#rs_ot + #dyn_cnt * 8 .. #rs_ot + (#dyn_cnt + 1) * 8].try_into().unwrap()) as usize };
        let mut rinit = Vec::new();
        for fi in &all_fields {
            let id = &fi.ident;
            let is_pod = matches!(fi.category, FieldCategory::PodPrimitive | FieldCategory::PodCustom);
            let is_inline_string = matches!(fi.category, FieldCategory::InlineString { .. });
            if is_pod {
                let off = pod_layout.iter().find(|(n,_,_)| *n == fi.name).map(|(_,o,_)|*o).unwrap();
                let ty = &fi.ty;
                rinit.push(quote! {
                    #id: ::mmapstruct::bytemuck::from_bytes(&data[#rs + #off .. #rs + #off + std::mem::size_of::<#ty>()])
                });
            } else if is_inline_string {
                let (off_val, sz) = pod_layout.iter()
                    .find(|(n,_,_)| *n == fi.name)
                    .map(|(_,o,sz)| (*o, *sz))
                    .unwrap();
                rinit.push(quote! {
                    #id: <String as ::mmapstruct::traits::MmapRead>::read(data, #rs + #off_val .. #rs + #off_val + #sz)
                });
            } else if fi.is_byref {
                let off = byref_layout.iter().find(|(n,_)| *n == fi.name).map(|(_,o)|*o).unwrap();
                match &fi.category {
                    FieldCategory::DynamicOption { inner } => {
                        rinit.push(quote! {
                            #id: {
                                let base = #rs;
                                let br: ::mmapstruct::traits::ByRefRange = *::mmapstruct::bytemuck::from_bytes(&data[base + #off .. base + #off + 8]);
                                if br.begin == 0 && br.end == 0 { None }
                                else { Some(<#inner as ::mmapstruct::traits::MmapRead>::read(data, br.range(base))) }
                            }
                        });
                    }
                    _ => {
                        let ty = &fi.ty;
                        rinit.push(quote! {
                            #id: {
                                let base = #rs;
                                ::mmapstruct::reader::read_by_ref::<#ty>(data, base, #rs + #off .. #rs + #off + 8)
                            }
                        });
                    }
                }
            } else {
                let di = inline_dyn_indices.iter().position(|&i| all_fields[i].name == fi.name).unwrap();
                let rc = quote! {{
                    let ot_start = #rs + #fixed_sz;
                    let vs = #rs + #hdr_sz;
                    let off0 = u64::from_le_bytes(data[ot_start + #di * 8 .. ot_start + #di * 8 + 8].try_into().unwrap()) as usize;
                    let off1 = if #di + 1 < #dyn_cnt {
                        u64::from_le_bytes(data[ot_start + (#di + 1) * 8 .. ot_start + (#di + 1) * 8 + 8].try_into().unwrap()) as usize
                    } else { #field_end_offset };
                    (vs + off0, vs + off1)
                }};
                let ty = &fi.ty;
                match &fi.category {
                    FieldCategory::DynamicString => {
                        rinit.push(quote! { #id: { let (s, e) = #rc; <#ty as ::mmapstruct::traits::MmapRead>::read(data, s..e) } });
                    }
                    FieldCategory::DynamicVecByRef { .. } => {
                        rinit.push(quote! { #id: { let base = #rs; let (s, e) = #rc; ::mmapstruct::byref_array::ByRefArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e), base) } });
                    }
                    FieldCategory::DynamicVec { elem_is_pod, elem_fixed, .. } => {
                        if *elem_is_pod {
                            rinit.push(quote! { #id: { let (s, e) = #rc; ::mmapstruct::bytemuck::cast_slice(&data[s..e]) } });
                        } else if *elem_fixed {
                            rinit.push(quote! { #id: { let (s, e) = #rc; ::mmapstruct::fixed_array::FixedArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e)) } });
                        } else {
                            rinit.push(quote! { #id: { let (s, e) = #rc; ::mmapstruct::indexed_array::IndexedArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e)) } });
                        }
                    }
                    _ => {
                        rinit.push(quote! { #id: { let (s, e) = #rc; <#ty as ::mmapstruct::traits::MmapRead>::read(data, s..e) } });
                    }
                }
            }
        }

        // Write body (uses bare field idents, not self.)
        let fw: Vec<_> = all_fields.iter().map(|fi| {
            let id = &fi.ident;
            let is_pod = matches!(fi.category, FieldCategory::PodPrimitive | FieldCategory::PodCustom);
            let is_inline_string = matches!(fi.category, FieldCategory::InlineString { .. });
            if is_pod { quote! { ctx.write_pod(#id); } }
            else if is_inline_string {
                if let FieldCategory::InlineString { size } = &fi.category {
                    quote! { ctx.write_inline_string(#id, #size); }
                } else { unreachable!() }
            }
            else if fi.is_byref && !matches!(&fi.category, FieldCategory::DynamicVecByRef { .. }) {
                let dedup = fi.dedup;
                match &fi.category {
                    FieldCategory::DynamicOption { .. } => {
                        quote! { if let Some(val) = #id { ctx.write_byref(val, start, #dedup); } else { ctx.write_pod(&::mmapstruct::traits::ByRefRange { begin: 0, end: 0 }); } }
                    }
                    _ => { quote! { ctx.write_byref(#id, start, #dedup); } }
                }
            } else { quote! {} }
        }).collect();

        let idb: Vec<_> = inline_dyn_indices.iter().map(|&idx| {
            let fi = &all_fields[idx];
            let id = &fi.ident;
            match &fi.category {
                FieldCategory::DynamicVec { inner, elem_is_pod, elem_fixed } if *elem_is_pod => {
                    quote! { { ctx.align_from(vs, std::mem::align_of::<#inner>()); let before = (ctx.current_offset() - vs) as u64; dyn_offsets.push(before); ::mmapstruct::pod_array::write_pod_array(#id, ctx); } }
                }
                _ => {
                    let we = match &fi.category {
                        FieldCategory::DynamicString => { quote! { ::mmapstruct::traits::MmapWrite::write(#id, ctx); } }
                        FieldCategory::DynamicVecByRef { dedup, .. } => {
                            quote! { ::mmapstruct::byref_array::write_byref_array(#id, ctx, start, #dedup); }
                        }
                        FieldCategory::DynamicVec { elem_fixed, .. } => {
                            if *elem_fixed { quote! { ::mmapstruct::fixed_array::write_fixed_array(#id, ctx); } }
                            else { quote! { ::mmapstruct::indexed_array::write_indexed_array(#id, ctx); } }
                        }
                        _ => { quote! { ::mmapstruct::traits::MmapWrite::write(#id, ctx); } }
                    };
                    quote! { { let before = (ctx.current_offset() - vs) as u64; dyn_offsets.push(before); #we } }
                }
            }
        }).collect();

        // Align variant data after discriminant
        let align_after_disc = if variant_data_align > 4 {
            quote! { ctx.align_to(#variant_data_align); }
        } else {
            quote! {}
        };

        let wbody = if !has_dynamic {
            let writes: Vec<_> = all_fields.iter().map(|fi| {
                let id = &fi.ident;
                quote! { ctx.write_pod(#id); }
            }).collect();
            quote! {
                ctx.write_u32_le(#disc as u32);
                #align_after_disc
                #(#writes)*
            }
        } else {
            quote! {
                ctx.write_u32_le(#disc as u32);
                #align_after_disc
                let start = ctx.current_offset();
                #(#fw)*
                ctx.align_from(start, 8);
                let ot_start = ctx.current_offset();
                ctx.write_padding((#dyn_cnt + 1) * 8);
                let vs = ctx.current_offset();
                let mut dyn_offsets: Vec<u64> = Vec::new();
                #(#idb)*
                let total_dyn_size = (ctx.current_offset() - vs) as u64;
                for (i, &off) in dyn_offsets.iter().enumerate() {
                    ctx.write_u64_le_at(off, ot_start + i * 8);
                }
                ctx.write_u64_le_at(total_dyn_size, ot_start + #dyn_cnt * 8);
            }
        };

        // From body (uses bound field idents directly, not r.)
        let fconv: Vec<_> = all_fields.iter().map(|fi| {
            let id = &fi.ident;
            let expr = match &fi.category {
                FieldCategory::PodPrimitive | FieldCategory::PodCustom => { quote! { *#id } }
                FieldCategory::DynamicOption { .. } => { quote! { #id.map(|v| v.into()) } }
                _ => { quote! { #id.into() } }
            };
            quote! { #id: #expr }
        }).collect();
        let fbody = quote! { #name::#vident { #(#fconv),* } };

        // Default
        let dfields: Vec<_> = all_fields.iter().map(|fi| {
            let id = &fi.ident;
            quote! { #id: Default::default() }
        }).collect();
        let dbody = if disc == 0 {
            quote! { #name::#vident { #(#dfields),* } }
        } else {
            quote! { unreachable!() }
        };

        let disc_u64 = disc as u64;
        let variant_field_hashes: Vec<_> = all_fields.iter().map(|fi| {
            let ty = &fi.ty;
            let base = quote! { <#ty as ::mmapstruct::traits::LayoutHash>::HASH };

            if fi.is_byref {
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
                    _ => base,
                }
            }
        }).collect();

        let fidents: Vec<_> = all_fields.iter().map(|fi| fi.ident.clone()).collect();
        vars.push(VarGen {
            ident: vident.clone(),
            field_idents: fidents,
            reader_fields: rfields,
            reader_init: if rinit.is_empty() {
                quote! { #reader_name::#vident }
            } else {
                quote! { #reader_name::#vident { #(#rinit),* } }
            },
            write_body: wbody,
            from_body: fbody,
            default_body: dbody,
            layout_hash: quote! { ::mmapstruct::traits::combine_hashes(&[#disc_u64 as u64, #(#variant_field_hashes),*, #fixed_sz as u64, #hdr_sz as u64]) },
        });
    }

    // === Generate enum reader ===
    let enum_def_variants: Vec<_> = vars.iter().map(|v| {
        let vi = &v.ident;
        if v.reader_fields.is_empty() {
            quote! { #vi }
        } else {
            let fs = &v.reader_fields;
            quote! { #vi { #(#fs),* } }
        }
    }).collect();

    // === MmapRead ===
    let read_arms: Vec<_> = vars.iter().enumerate().map(|(disc, v)| {
        let init = &v.reader_init;
        let disc_u32 = disc as u32;
        quote! { #disc_u32 => #init , }
    }).collect();

    let read_impl = quote! {
        impl ::mmapstruct::traits::MmapRead for #name {
            type Reader<'a> = #reader_name<'a>;
            fn read<'a>(data: &'a [u8], range: std::ops::Range<usize>) -> Self::Reader<'a> {
                let disc = u32::from_le_bytes(data[range.start .. range.start + 4].try_into().unwrap());
                match disc {
                    #(#read_arms)*
                    _ => panic!("unknown discriminant {} for {}", disc, stringify!(#name)),
                }
            }
        }
    };

    // === MmapWrite ===
    let write_arms: Vec<_> = vars.iter().map(|v| {
        let vi = &v.ident;
        let fidents = &v.field_idents;
        let body = &v.write_body;
        if fidents.is_empty() {
            quote! { #name::#vi => { #body } , }
        } else {
            quote! { #name::#vi { #(#fidents),* } => { #body } , }
        }
    }).collect();

    let write_impl = quote! {
        impl ::mmapstruct::traits::MmapWrite for #name {
            fn write<'data>(&'data self, ctx: &mut ::mmapstruct::WriteContext<'data>) {
                match self {
                    #(#write_arms)*
                }
            }
        }
    };

    // === From<Reader> ===
    let from_arms: Vec<_> = vars.iter().map(|v| {
        let vi = &v.ident;
        let fidents = &v.field_idents;
        let from = &v.from_body;
        if fidents.is_empty() {
            quote! { #reader_name::#vi => #from , }
        } else {
            quote! { #reader_name::#vi { #(#fidents),* } => #from , }
        }
    }).collect();

    let from_impl = quote! {
        impl<'a> From<#reader_name<'a>> for #name {
            fn from(r: #reader_name<'a>) -> Self {
                match r {
                    #(#from_arms)*
                }
            }
        }
    };

    // === Default ===
    // Map Default body for the first variant (others unreachable)
    let default_body = vars.first().map(|v| &v.default_body).cloned().unwrap_or_else(|| {
        quote! { panic!("empty enum") }
    });

    let default_impl = quote! {
        impl Default for #name {
            fn default() -> Self {
                #default_body
            }
        }
    };

    // === LayoutHash ===
    let variant_hashes: Vec<_> = vars.iter().map(|v| &v.layout_hash).collect();
    let layout_hash_impl = quote! {
        impl ::mmapstruct::traits::LayoutHash for #name {
            const HASH: u64 = ::mmapstruct::traits::combine_hashes(&[
                #(#variant_hashes),*
            ]);
        }
    };

    let output = quote! {
        pub enum #reader_name<'a> {
            #(#enum_def_variants),*
        }
        #read_impl
        #write_impl
        #from_impl
        #default_impl
        #layout_hash_impl
    };

    output.into()
}
