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
    let mut variant_consts: Vec<proc_macro2::TokenStream> = Vec::new();

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
        let has_dynamic = all_fields.iter().any(|f| !is_pod_field(&f.category) || f.is_fixed_byref());

        // Fixed-section layout via const-eval — mirrors the struct path
        // (`generate_fixed_layout_consts`): real size_of/align_of for every
        // fixed field, including non-primitive pod fields (PodCustom) where a
        // macro-time guess (8) desynced reader/schema offsets from the writer
        // (see bugs/enum-podcustom-field-layout-8-vs-write-4.md). The reader
        // and the schema reference these consts; the writer writes the real
        // sizes, so all three agree by construction.
        // The post-discriminant data start is uniformly 8 (u32 disc + align
        // to 8), matching the writer's `ctx.align_to(8)` and both schema
        // readers' variant data offsets.
        let vprefix = variant_const_prefix(name, vident);
        let fo_name = format_ident!("__MMAP_FO_{}", vprefix);
        let fs_name = format_ident!("__MMAP_FS_{}", vprefix);
        let hs_name = format_ident!("__MMAP_HS_{}", vprefix);
        variant_consts.push(generate_fixed_layout_consts(
            &vprefix,
            &all_fields,
            inline_dyn_indices.len(),
        ));
        let dyn_cnt = inline_dyn_indices.len();

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

        let rs = quote! { range.start + 8 };
        let rs_ot = quote! { #rs + #fs_name };
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
                let idx = fixed_section_index(&all_fields, fi.name.as_str());
                let ty = &fi.ty;
                let expr = quote! { ::mmapstruct::bytemuck::from_bytes(&data[#rs + #fo_name[#idx] .. #rs + #fo_name[#idx] + std::mem::size_of::<#ty>()]) };
                if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
            } else if is_inline_string {
                let idx = fixed_section_index(&all_fields, fi.name.as_str());
                let sz = match &fi.category {
                    FieldCategory::InlineString { size } => *size,
                    _ => unreachable!(),
                };
                let expr = quote! { <String as ::mmapstruct::traits::MmapRead>::read(data, #rs + #fo_name[#idx] .. #rs + #fo_name[#idx] + #sz) };
                if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
            } else if fi.has_byref_attr {
                let idx = fixed_section_index(&all_fields, fi.name.as_str());
                match &fi.category {
                    FieldCategory::DynamicOption { inner } => {
                        let expr = quote! {{
                            let base = #rs;
                            let br: ::mmapstruct::traits::ByRefRange = *::mmapstruct::bytemuck::from_bytes(&data[base + #fo_name[#idx] .. base + #fo_name[#idx] + 8]);
                            if br.begin == 0 && br.end == 0 { None }
                            else { Some(<#inner as ::mmapstruct::traits::MmapRead>::read(data, br.range(base))) }
                        }};
                        if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                    }
                    _ => {
                        let ty = &fi.ty;
                        let expr = quote! {{
                            let base = #rs;
                            ::mmapstruct::reader::read_by_ref::<#ty>(data, base, #rs + #fo_name[#idx] .. #rs + #fo_name[#idx] + 8)
                        }};
                        if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
                    }
                }
            } else if is_fixed_arr {
                let idx = fixed_section_index(&all_fields, fi.name.as_str());
                let ty = &fi.ty;
                let expr = quote! {
                    unsafe {
                        &*(data.as_ptr().add(#rs + #fo_name[#idx]) as *const #ty)
                    }
                };
                if is_tuple { rinit.push(expr); } else { rinit.push(quote! { #id: #expr }); }
            } else {
                let di = inline_dyn_indices
                    .iter()
                    .position(|&i| all_fields[i].name == fi.name)
                    .expect("field not found in inline_dyn_indices (else branch)");
                let rc = quote! {{
                    let ot_start = #rs + #fs_name;
                    let vs = #rs + #hs_name;
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
                        inner,
                        elem_is_pod,
                        elem_fixed,
                        ..
                    } => {
                        if *elem_is_pod {
                            let expr = quote! {
                                {
                                    let (s, _e) = #rc;
                                    let count = u64::from_le_bytes(data[s..s+8].try_into().unwrap()) as usize;
                                    let elem_size = ::std::mem::size_of::<#inner>();
                                    ::mmapstruct::bytemuck::cast_slice(&data[s+8 .. s+8 + count * elem_size])
                                }
                            };
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

        let wbody = if !has_dynamic {
            let fw_pod = write_helpers::generate_fixed_writes(&all_fields, &field_ref);
            quote! {
                ctx.write_u32_le(#disc as u32);
                ctx.align_to(8);
                let start = ctx.current_offset();
                #(#fw_pod)*
                ctx.align_from(start, 8);
            }
        } else {
            quote! {
                ctx.write_u32_le(#disc as u32);
                ctx.align_to(8);
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
            layout_hash: quote! { ::mmapstruct::traits::combine_hashes(&[#disc_u64 as u64, #(#variant_field_hashes),*, #fs_name as u64, #hs_name as u64]) },
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

    let reader_ord_impl = generate_enum_reader_ord_impl(name, &reader_name, &vars);

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
        #(#variant_consts)*
        pub enum #reader_name<'a> {
            #(#enum_def_variants),*
        }
        #read_impl
        #write_impl
        #from_impl
        #default_impl
        #layout_hash_impl
        #reader_ord_impl
        #schema_impl
    };

    output.into()
}

/// Generate `impl ReaderOrd` for a data-carrying enum: compare the variant
/// discriminant first, then the fields of the active variant (field-by-field,
/// mirroring the mixed-struct impl in `from_default.rs`).
///
/// Skipped when any variant contains a field type without `ReaderOrd`
/// (`Vec`, `HashMap`, `Option<complex>`, …) — same limitation as the struct
/// path (see `ideas/idea_complex_zero_to_hero_cmp.md`).
pub(crate) fn generate_enum_reader_ord_impl(
    name: &proc_macro2::Ident,
    reader_name: &proc_macro2::Ident,
    vars: &[VarGen],
) -> proc_macro2::TokenStream {
    let is_simple = |cat: &FieldCategory| -> bool {
        match cat {
            // PodCustom is deliberately NOT simple here: the struct path's
            // ReaderOrd requires `Ord` on pod-custom fields, and requiring it
            // on enums would break code that compiled without `ReaderOrd`.
            FieldCategory::PodPrimitive
            | FieldCategory::DynamicString
            | FieldCategory::InlineString { .. } => true,
            FieldCategory::DynamicOption { inner } => {
                is_primitive_pod(inner) || is_string_type(inner)
            }
            _ => false,
        }
    };
    if vars
        .iter()
        .any(|v| v.fields.iter().any(|f| !is_simple(&f.category)))
    {
        return quote! {};
    }

    let disc_arms_reader: Vec<_> = vars
        .iter()
        .enumerate()
        .map(|(disc, v)| {
            let vi = &v.ident;
            let d = disc as u32;
            if v.is_tuple {
                quote! { #reader_name::#vi(..) => #d, }
            } else if v.field_idents.is_empty() {
                quote! { #reader_name::#vi => #d, }
            } else {
                quote! { #reader_name::#vi { .. } => #d, }
            }
        })
        .collect();
    let disc_arms_key: Vec<_> = vars
        .iter()
        .enumerate()
        .map(|(disc, v)| {
            let vi = &v.ident;
            let d = disc as u32;
            if v.is_tuple {
                quote! { #name::#vi(..) => #d, }
            } else if v.field_idents.is_empty() {
                quote! { #name::#vi => #d, }
            } else {
                quote! { #name::#vi { .. } => #d, }
            }
        })
        .collect();

    let pair_arms: Vec<_> = vars
        .iter()
        .map(|v| {
            let vi = &v.ident;
            let field_cmp = |fi: &FieldInfo, r: &proc_macro2::Ident, k: &proc_macro2::Ident| {
                match &fi.category {
                    FieldCategory::PodPrimitive | FieldCategory::PodCustom => {
                        if is_f32_or_f64(&fi.ty) {
                            quote! { (*#r).partial_cmp(#k).unwrap_or(std::cmp::Ordering::Equal) }
                        } else {
                            quote! { (*#r).cmp(#k) }
                        }
                    }
                    FieldCategory::DynamicString | FieldCategory::InlineString { .. } => {
                        quote! { (*#r).cmp(#k.as_str()) }
                    }
                    FieldCategory::DynamicOption { inner } => {
                        quote! {
                            match (#r, #k) {
                                (Some(rr), Some(kk)) => <#inner as ::mmapstruct::traits::ReaderOrd>::cmp_reader(rr, kk),
                                (None, None) => std::cmp::Ordering::Equal,
                                (None, Some(_)) => std::cmp::Ordering::Less,
                                (Some(_), None) => std::cmp::Ordering::Greater,
                            }
                        }
                    }
                    _ => quote! { std::cmp::Ordering::Equal },
                }
            };

            if v.fields.is_empty() {
                quote! { (#reader_name::#vi, #name::#vi) => std::cmp::Ordering::Equal, }
            } else if v.is_tuple {
                let rids: Vec<_> = v.field_idents.iter().map(|i| format_ident!("__r{}", i)).collect();
                let kids: Vec<_> = v.field_idents.iter().map(|i| format_ident!("__k{}", i)).collect();
                let cmps: Vec<_> = v
                    .fields
                    .iter()
                    .zip(rids.iter().zip(kids.iter()))
                    .map(|(fi, (r, k))| field_cmp(fi, r, k))
                    .collect();
                let body = if cmps.len() == 1 {
                    cmps.into_iter().next().unwrap()
                } else {
                    let mut iter = cmps.into_iter();
                    let first = iter.next().unwrap();
                    iter.fold(first, |acc, cmp| quote! { #acc.then(#cmp) })
                };
                quote! {
                    (#reader_name::#vi(#(#rids),*), #name::#vi(#(#kids),*)) => #body,
                }
            } else {
                let rbinds: Vec<_> = v
                    .fields
                    .iter()
                    .map(|fi| {
                        let id = &fi.ident;
                        let rb = format_ident!("__r_{}", fi.ident);
                        quote! { #id: #rb }
                    })
                    .collect();
                let kbinds: Vec<_> = v
                    .fields
                    .iter()
                    .map(|fi| {
                        let id = &fi.ident;
                        let kb = format_ident!("__k_{}", fi.ident);
                        quote! { #id: #kb }
                    })
                    .collect();
                let cmps: Vec<_> = v
                    .fields
                    .iter()
                    .map(|fi| {
                        let r = format_ident!("__r_{}", fi.ident);
                        let k = format_ident!("__k_{}", fi.ident);
                        field_cmp(fi, &r, &k)
                    })
                    .collect();
                let body = if cmps.len() == 1 {
                    cmps.into_iter().next().unwrap()
                } else {
                    let mut iter = cmps.into_iter();
                    let first = iter.next().unwrap();
                    iter.fold(first, |acc, cmp| quote! { #acc.then(#cmp) })
                };
                quote! {
                    (#reader_name::#vi { #(#rbinds),* }, #name::#vi { #(#kbinds),* }) => #body,
                }
            }
        })
        .collect();

    quote! {
        impl ::mmapstruct::traits::ReaderOrd for #name {
            fn cmp_reader(reader: &#reader_name<'_>, key: &Self) -> std::cmp::Ordering {
                fn __mmap_disc_reader(r: &#reader_name<'_>) -> u32 {
                    match r {
                        #(#disc_arms_reader)*
                    }
                }
                fn __mmap_disc_key(k: &#name) -> u32 {
                    match k {
                        #(#disc_arms_key)*
                    }
                }
                let dr = __mmap_disc_reader(reader);
                let dk = __mmap_disc_key(key);
                if dr != dk {
                    return dr.cmp(&dk);
                }
                match (reader, key) {
                    #(#pair_arms)*
                    // Different variants were already ordered by the
                    // discriminant comparison above; this arm is unreachable.
                    _ => std::cmp::Ordering::Equal,
                }
            }
        }
    }
}
