//! Generates `MmapRead`/`MmapWrite`/`MmapFromDefault` impls for enums.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Fields, DeriveInput};

use crate::utils::*;
use crate::attrs::*;
use crate::category::*;
use crate::field_info::*;
use crate::layout::*;
use crate::write_helpers;
#[cfg(feature = "dyn")]
use crate::schema::generate_enum_schema_impl;

#[cfg(feature = "dyn")]
fn repr_to_disc_size(repr_name: &str) -> proc_macro2::TokenStream {
    match repr_name {
        "u8" | "i8" => quote! { 1u64 },
        "u16" | "i16" => quote! { 2u64 },
        "u32" | "i32" => quote! { 4u64 },
        "u64" | "i64" => quote! { 8u64 },
        _ => panic!("unsupported repr for pod enum: {}", repr_name),
    }
}

/// Per-variant generated code for enum handling.
pub(crate) struct VarGen {
    pub(crate) ident: proc_macro2::Ident,
    pub(crate) field_idents: Vec<proc_macro2::Ident>,
    /// Used by `generate_enum_schema_impl` in `schema.rs` (only with `feature = "dyn"`).
    #[allow(dead_code)]
    pub(crate) fields: Vec<FieldInfo>,
    /// Used by `generate_enum_schema_impl` in `schema.rs` (only with `feature = "dyn"`).
    #[allow(dead_code)]
    pub(crate) inline_dyn_indices: Vec<usize>,
    pub(crate) reader_fields: Vec<proc_macro2::TokenStream>,
    pub(crate) reader_init: proc_macro2::TokenStream,
    pub(crate) write_body: proc_macro2::TokenStream,
    pub(crate) from_body: proc_macro2::TokenStream,
    pub(crate) default_body: proc_macro2::TokenStream,
    pub(crate) layout_hash: proc_macro2::TokenStream,
    pub(crate) is_tuple: bool,
}

/// Fast path for `#[repr(Int)]` enums with all unit variants.
///
/// Generates `MmapPod` + `LayoutHash` + `Default` + `bytemuck::{Pod, Zeroable}`
/// (no reader/writer structs).  Does NOT generate `GetSchema` — user can provide
/// their own schema mapping.
fn handle_pod_enum(input: &DeriveInput, de: &syn::DataEnum) -> TokenStream {
    let name = &input.ident;

    // Which integer repr was used (e.g. "u32", "u8").
    let repr_name = find_repr_int(&input.attrs).expect("pod enum requires #[repr(Int)]");

    // First variant is the default (zero discriminant = Zeroable).
    let first_variant = de.variants.iter().next()
        .expect("pod enum must have at least one variant").ident.clone();

    // Byte string for the repr tag in the hash.
    let repr_tag = format!("#[repr({})]", repr_name);

    // Collect discriminants 0u64, 1u64, 2u64, ...
    let variant_count = de.variants.len();
    let mut disc_hashes = Vec::with_capacity(variant_count + 1);
    disc_hashes.push(quote! { ::mmapstruct::traits::fnv1a(#repr_tag.as_bytes()) });
    for disc in 0..variant_count {
        let d = disc as u64;
        disc_hashes.push(quote! { #d });
    }

    let output = quote! {
        #[allow(unsafe_code)]
        unsafe impl ::mmapstruct::bytemuck::Pod for #name {}
        #[allow(unsafe_code)]
        unsafe impl ::mmapstruct::bytemuck::Zeroable for #name {}

        impl ::mmapstruct::traits::MmapPod for #name {}

        impl Default for #name {
            fn default() -> Self {
                #name::#first_variant
            }
        }

        impl ::mmapstruct::traits::LayoutHash for #name {
            const HASH: u64 = ::mmapstruct::traits::combine_hashes(&[
                #(#disc_hashes),*
            ]);
        }
    };

    let schema_impl = {
        #[cfg(feature = "dyn")]
        {
            let name_str = name.to_string();
            let enum_schema_name = format_ident!("__mmap_ENUM_{}", name);

            let mut variant_schema_statics = Vec::new();
            let mut variant_schema_refs = Vec::new();

            for v in de.variants.iter() {
                let vident = &v.ident;
                let schema_name = format_ident!("__mmap_ENUMVAR_{}_{}", name, vident);
                let field_statics_name = format_ident!("__mmap_ENUMVAR_FIELDS_{}_{}", name, vident);
                variant_schema_statics.push(quote! {
                    #[allow(non_upper_case_globals)]
                    static #field_statics_name: [::mmapstruct::schema::FieldDescriptor; 0] = [];
                    #[allow(non_upper_case_globals)]
                    static #schema_name: ::mmapstruct::schema::StructSchema = ::mmapstruct::schema::StructSchema {
                        name: concat!(#name_str, "::", stringify!(#vident)),
                        fields: &#field_statics_name,
                        fixed_section_size: 0u64,
                        header_size: 0u64,
                        inline_dyn_count: 0usize,
                    };
                });
                variant_schema_refs.push(quote! { &#schema_name });
            }

            let disc_size = repr_to_disc_size(repr_name);
            let schema_refs = variant_schema_refs;
            let var_statics = variant_schema_statics
                .into_iter()
                .collect::<proc_macro2::TokenStream>();

            quote! {
                #var_statics
                #[allow(non_upper_case_globals)]
                static #enum_schema_name: ::mmapstruct::schema::EnumSchema = ::mmapstruct::schema::EnumSchema {
                    name: #name_str,
                    variants: &[#(#schema_refs),*],
                    disc_size: #disc_size,
                };
                impl ::mmapstruct::schema::GetSchema for #name {
                    fn get_struct_schema() -> &'static ::mmapstruct::schema::StructSchema {
                        panic!("GetSchema::get_struct_schema() called on enum type {}; use root_value_type() instead", stringify!(#name));
                    }
                    fn root_value_type() -> &'static ::mmapstruct::schema::DynValueType {
                        static VT: ::mmapstruct::schema::DynValueType = ::mmapstruct::schema::DynValueType::Enum(&#enum_schema_name);
                        &VT
                    }
                }
            }
        }
        #[cfg(not(feature = "dyn"))]
        {
            quote! {}
        }
    };

    let output = quote! {
        #output
        #schema_impl
    };

    output.into()
}

pub(crate) fn handle_enum(input: &DeriveInput, de: &syn::DataEnum) -> TokenStream {
    let name = &input.ident;

    // Fast path: #[repr(u32)] enum with all unit variants → MmapPod.
    if de.variants.iter().all(|v| v.fields.is_empty()) {
        if has_repr_int(&input.attrs) {
            return handle_pod_enum(input, de);
        }
        panic!(
            "MmapStruct: enum `{}` has only unit (fieldless) variants and must have \
             a #[repr(Int)] attribute (e.g. `#[repr(u32)]`) to define its binary \
             discriminant size. Supported repr types: u8, u16, u32, u64, i8, i16, i32, i64.",
            name,
        );
    }

    let reader_name = format_ident!("{}Reader", name);

    let mut vars = Vec::new();

    for (disc, variant) in de.variants.iter().enumerate() {
        let vident = &variant.ident;

        if variant.fields.is_empty() {
            vars.push(VarGen {
                ident: vident.clone(),
                field_idents: Vec::new(),
                fields: Vec::new(),
                inline_dyn_indices: Vec::new(),
                reader_fields: Vec::new(),
                reader_init: quote! { #reader_name::#vident },
                write_body: quote! { ctx.write_u32_le(#disc as u32); ctx.align_to(8); },
                from_body: quote! { #name::#vident },
                default_body: if disc == 0 {
                    quote! { #name::#vident }
                } else {
                    quote! { unreachable!() }
                },
                layout_hash: quote! { #disc as u64 },
                is_tuple: false,
            });
            continue;
        }

        let is_tuple = matches!(&variant.fields, Fields::Unnamed(_));

        let mut all_fields: Vec<FieldInfo> = Vec::new();

        match &variant.fields {
            Fields::Named(f) => {
                for field in f.named.iter() {
                    let category = classify_field(field);
                    let (is_byref, dedup, _inline_size, is_bool) = parse_mmap_attrs(&field.attrs);

                    if is_bool && !is_u32_type(&field.ty) {
                        panic!("#[mmap(bool)] is only supported on u32 fields");
                    }
                    if is_primitive_pod(&field.ty) && type_ident_name(&field.ty).map_or(false, |n| n == "bool") {
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
            }
            Fields::Unnamed(f) => {
                for (i, field) in f.unnamed.iter().enumerate() {
                    let category = classify_field(field);
                    let (is_byref, dedup, _inline_size, is_bool) = parse_mmap_attrs(&field.attrs);
                    let ident = format_ident!("_{}", i);

                    if is_bool && !is_u32_type(&field.ty) {
                        panic!("#[mmap(bool)] is only supported on u32 fields");
                    }
                    if is_primitive_pod(&field.ty) && type_ident_name(&field.ty).map_or(false, |n| n == "bool") {
                        panic!("bool field is not supported; use u32 with #[mmap(bool)] instead");
                    }

                    all_fields.push(FieldInfo {
                        name: i.to_string(),
                        ty: field.ty.clone(),
                        category,
                        has_byref_attr: is_byref,
                        dedup,
                        ident,
                        is_mmap_bool: is_bool,
                    });
                }
            }
            Fields::Unit => {}
        }

        let inline_dyn_indices = collect_inline_dyn_indices(&all_fields);
        let has_dynamic = all_fields.iter().any(|f| !is_pod_field(&f.category));

        let mut variant_data_align = 1usize;
        for fi in &all_fields {
            let is_pod = matches!(
                fi.category,
                FieldCategory::PodPrimitive | FieldCategory::PodCustom
            );
            if is_pod {
                let sz = known_primitive_size(&fi.ty).unwrap_or(8);
                let al = if sz == 1 { 1 } else { sz.min(8).max(1) };
                if al > variant_data_align {
                    variant_data_align = al;
                }
            } else if fi.has_byref_attr {
                if 4 > variant_data_align {
                    variant_data_align = 4;
                }
            }
        }
        let variant_data_align_for_offset = variant_data_align.max(8);
        let variant_data_offset = align_offset(4, variant_data_align_for_offset);

        let mut fixed_offset = 0usize;
        let mut pod_layout: Vec<(String, usize, usize)> = Vec::new();
        let mut byref_layout: Vec<(String, usize)> = Vec::new();
        for fi in &all_fields {
            let is_pod = matches!(
                fi.category,
                FieldCategory::PodPrimitive | FieldCategory::PodCustom
            );
            if is_pod {
                let sz = known_primitive_size(&fi.ty).unwrap_or(8);
                let al = if sz == 1 { 1 } else { sz.min(8).max(1) };
                fixed_offset = align_offset(fixed_offset, al);
                pod_layout.push((fi.name.clone(), fixed_offset, sz));
                fixed_offset += sz;
            } else if let FieldCategory::InlineString { size } = &fi.category {
                pod_layout.push((fi.name.clone(), fixed_offset, *size));
                fixed_offset += size;
            } else if fi.has_byref_attr {
                fixed_offset = align_offset(fixed_offset, 4);
                byref_layout.push((fi.name.clone(), fixed_offset));
                fixed_offset += 8;
            } else if let FieldCategory::FixedArray { inner_type, count } = &fi.category {
                let elem_sz = known_primitive_size(inner_type)
                    .or_else(|| eval_type_size(inner_type))
                    .unwrap_or(8);
                let total_sz = elem_sz * count;
                let al = if elem_sz == 1 { 1 } else { elem_sz.min(8).max(1) };
                fixed_offset = align_offset(fixed_offset, al);
                pod_layout.push((fi.name.clone(), fixed_offset, total_sz));
                fixed_offset += total_sz;
            }
        }
        fixed_offset = align_offset(fixed_offset, 8);
        let fixed_sz = fixed_offset;
        let dyn_cnt = inline_dyn_indices.len();
        let hdr_sz = fixed_sz + if dyn_cnt > 0 { (dyn_cnt + 1) * 8 } else { 0 };

        let rfields: Vec<_> = all_fields
            .iter()
            .map(|fi| {
                let rt = reader_type(fi);
                if is_tuple {
                    quote! { #rt }
                } else {
                    let id = &fi.ident;
                    quote! { #id: #rt }
                }
            })
            .collect();

        let rs = quote! { range.start + #variant_data_offset };
        let rs_ot = quote! { #rs + #fixed_sz };
        let field_end_offset = quote! { u64::from_le_bytes(data[#rs_ot + #dyn_cnt * 8 .. #rs_ot + (#dyn_cnt + 1) * 8].try_into().unwrap()) as usize };
        let mut rinit = Vec::new();
        for fi in &all_fields {
            let id = &fi.ident;
            let is_pod = matches!(
                fi.category,
                FieldCategory::PodPrimitive | FieldCategory::PodCustom
            );
            let is_inline_string = matches!(fi.category, FieldCategory::InlineString { .. });
            let is_fixed_arr = matches!(fi.category, FieldCategory::FixedArray { .. });
            if is_pod {
                let off = pod_layout
                    .iter()
                    .find(|(n, _, _)| *n == fi.name)
                    .map(|(_, o, _)| *o)
                    .expect("pod not in pod_layout");
                let ty = &fi.ty;
                let expr = quote! { ::mmapstruct::bytemuck::from_bytes(&data[#rs + #off .. #rs + #off + std::mem::size_of::<#ty>()]) };
                if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
            } else if is_inline_string {
                let (off_val, sz) = pod_layout
                    .iter()
                    .find(|(n, _, _)| *n == fi.name)
                    .map(|(_, o, sz)| (*o, *sz))
                    .expect("inline_str not in pod_layout");
                let expr = quote! { <String as ::mmapstruct::traits::MmapRead>::read(data, #rs + #off_val .. #rs + #off_val + #sz) };
                if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
            } else if fi.has_byref_attr {
                let off = byref_layout
                    .iter()
                    .find(|(n, _)| *n == fi.name)
                    .map(|(_, o)| *o)
                    .expect("byref not in byref_layout");
                match &fi.category {
                    FieldCategory::DynamicOption { inner } => {
                        let expr = quote! {{
                            let base = #rs;
                            let br: ::mmapstruct::traits::ByRefRange = *::mmapstruct::bytemuck::from_bytes(&data[base + #off .. base + #off + 8]);
                            if br.begin == 0 && br.end == 0 { None }
                            else { Some(<#inner as ::mmapstruct::traits::MmapRead>::read(data, br.range(base))) }
                        }};
                        if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                    }
                    _ => {
                        let ty = &fi.ty;
                        let expr = quote! {{
                            let base = #rs;
                            ::mmapstruct::reader::read_by_ref::<#ty>(data, base, #rs + #off .. #rs + #off + 8)
                        }};
                        if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                    }
                }
            } else if is_fixed_arr {
                let off = pod_layout
                    .iter()
                    .find(|(n, _, _)| *n == fi.name)
                    .map(|(_, o, _)| *o)
                    .expect("fixed_arr not in pod_layout");
                let ty = &fi.ty;
                let expr = quote! {
                    unsafe {
                        &*(data.as_ptr().add(#rs + #off) as *const #ty)
                    }
                };
                if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
            } else {
                let di = inline_dyn_indices
                    .iter()
                    .position(|&i| all_fields[i].name == fi.name)
                    .expect("field not found in inline_dyn_indices (else branch)");
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
                        let expr = quote! { { let (s, e) = #rc; <#ty as ::mmapstruct::traits::MmapRead>::read(data, s..e) } };
                        if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                    }
                    FieldCategory::DynamicVecByRef { .. } => {
                        let expr = quote! { { let base = #rs; let (s, e) = #rc; ::mmapstruct::byref_array::ByRefArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e), base) } };
                        if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                    }
                    FieldCategory::DynamicVec {
                        elem_is_pod,
                        elem_fixed,
                        ..
                    } => {
                        if *elem_is_pod {
                            let expr = quote! { { let (s, e) = #rc; ::mmapstruct::bytemuck::cast_slice(&data[s..e]) } };
                            if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                        } else if *elem_fixed {
                            let expr = quote! { { let (s, e) = #rc; ::mmapstruct::fixed_array::FixedArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e)) } };
                            if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                        } else {
                            let expr = quote! { { let (s, e) = #rc; ::mmapstruct::indexed_array::IndexedArrayReader::new(data, ::mmapstruct::traits::FileRange::new(s, e)) } };
                            if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                        }
                    }
                    _ => {
                        let expr = quote! { { let (s, e) = #rc; <#ty as ::mmapstruct::traits::MmapRead>::read(data, s..e) } };
                        if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                    }
                }
            }
        }

        let field_ref = |id: &proc_macro2::Ident| quote! { #id };
        let fw = write_helpers::generate_fixed_writes(&all_fields, &field_ref);
        let idb = write_helpers::generate_inline_dyn_writes(&all_fields, &inline_dyn_indices, &field_ref);

        let variant_data_align_inner = variant_data_align.max(8);
        let align_after_disc = quote! { ctx.align_to(#variant_data_align_inner); };

        let wbody = if !has_dynamic {
            let fw_pod = write_helpers::generate_fixed_writes(&all_fields, &field_ref);
            quote! {
                ctx.write_u32_le(#disc as u32);
                #align_after_disc
                let start = ctx.current_offset();
                #(#fw_pod)*
                ctx.align_from(start, 8);
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
                ctx.align_to(8);
            }
        };

        let fconv: Vec<_> = all_fields
            .iter()
            .map(|fi| {
                let id = &fi.ident;
                let expr = match &fi.category {
                    FieldCategory::PodPrimitive | FieldCategory::PodCustom | FieldCategory::FixedArray { .. } => {
                        quote! { *#id }
                    }
                    FieldCategory::DynamicOption { inner } => {
                        if is_primitive_pod(inner) {
                            quote! { #id.map(|v| *v) }
                        } else {
                            quote! { #id.map(|v| v.into()) }
                        }
                    }
                    _ => {
                        quote! { #id.into() }
                    }
                };
                if is_tuple {
                    expr
                } else {
                    quote! { #id: #expr }
                }
            })
            .collect();
        let fbody = if is_tuple {
            quote! { #name::#vident(#(#fconv),*) }
        } else {
            quote! { #name::#vident { #(#fconv),* } }
        };

        let dfields: Vec<_> = all_fields
            .iter()
            .map(|fi| {
                let id = &fi.ident;
                if is_tuple {
                    quote! { Default::default() }
                } else {
                    quote! { #id: Default::default() }
                }
            })
            .collect();
        let dbody = if disc == 0 {
            if is_tuple {
                quote! { #name::#vident(#(#dfields),*) }
            } else {
                quote! { #name::#vident { #(#dfields),* } }
            }
        } else {
            quote! { unreachable!() }
        };

        let disc_u64 = disc as u64;
        let variant_field_hashes: Vec<_> = all_fields.iter().map(|fi| {
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
                    _ => base,
                }
            }
        }).collect();

        let fidents: Vec<_> = all_fields.iter().map(|fi| fi.ident.clone()).collect();
        vars.push(VarGen {
            ident: vident.clone(),
            field_idents: fidents,
            fields: all_fields.clone(),
            inline_dyn_indices: inline_dyn_indices.clone(),
            reader_fields: rfields,
            reader_init: if rinit.is_empty() {
                quote! { #reader_name::#vident }
            } else if is_tuple {
                quote! { #reader_name::#vident(#(#rinit),*) }
            } else {
                quote! { #reader_name::#vident { #(#rinit),* } }
            },
            write_body: wbody,
            from_body: fbody,
            default_body: dbody,
            layout_hash: quote! { ::mmapstruct::traits::combine_hashes(&[#disc_u64 as u64, #(#variant_field_hashes),*, #fixed_sz as u64, #hdr_sz as u64]) },
            is_tuple,
        });
    }

    let enum_def_variants: Vec<_> = vars
        .iter()
        .map(|v| {
            let vi = &v.ident;
            if v.reader_fields.is_empty() {
                quote! { #vi }
            } else if v.is_tuple {
                let fs = &v.reader_fields;
                quote! { #vi(#(#fs),*) }
            } else {
                let fs = &v.reader_fields;
                quote! { #vi { #(#fs),* } }
            }
        })
        .collect();

    let read_arms: Vec<_> = vars
        .iter()
        .enumerate()
        .map(|(disc, v)| {
            let init = &v.reader_init;
            let disc_u32 = disc as u32;
            quote! { #disc_u32 => #init , }
        })
        .collect();

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

    let write_arms: Vec<_> = vars
        .iter()
        .map(|v| {
            let vi = &v.ident;
            let fidents = &v.field_idents;
            let body = &v.write_body;
            if fidents.is_empty() {
                quote! { #name::#vi => { #body } , }
            } else if v.is_tuple {
                quote! { #name::#vi(#(#fidents),*) => { #body } , }
            } else {
                quote! { #name::#vi { #(#fidents),* } => { #body } , }
            }
        })
        .collect();

    let write_impl = quote! {
        impl ::mmapstruct::traits::MmapWrite for #name {
            fn write<'data>(&'data self, ctx: &mut ::mmapstruct::WriteContext<'data>) {
                match self {
                    #(#write_arms)*
                }
            }
        }
    };

    let from_arms: Vec<_> = vars
        .iter()
        .map(|v| {
            let vi = &v.ident;
            let fidents = &v.field_idents;
            let from = &v.from_body;
            if fidents.is_empty() {
                quote! { #reader_name::#vi => #from , }
            } else if v.is_tuple {
                quote! { #reader_name::#vi(#(#fidents),*) => #from , }
            } else {
                quote! { #reader_name::#vi { #(#fidents),* } => #from , }
            }
        })
        .collect();

    let from_impl = quote! {
        impl<'a> From<#reader_name<'a>> for #name {
            fn from(r: #reader_name<'a>) -> Self {
                match r {
                    #(#from_arms)*
                }
            }
        }
    };

    let default_body = vars
        .first()
        .map(|v| &v.default_body)
        .cloned()
        .unwrap_or_else(|| {
            quote! { panic!("empty enum") }
        });

    let default_impl = quote! {
        impl Default for #name {
            fn default() -> Self {
                #default_body
            }
        }
    };

    let variant_hashes: Vec<_> = vars.iter().map(|v| &v.layout_hash).collect();
    let layout_hash_impl = quote! {
        impl ::mmapstruct::traits::LayoutHash for #name {
            const HASH: u64 = ::mmapstruct::traits::combine_hashes(&[
                #(#variant_hashes),*
            ]);
        }
    };

    let schema_impl = {
        #[cfg(feature = "dyn")]
        {
            generate_enum_schema_impl(name, &vars)
        }
        #[cfg(not(feature = "dyn"))]
        {
            quote! {}
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
        #schema_impl
    };

    output.into()
}
