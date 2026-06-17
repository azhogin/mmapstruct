//! Generates `GetSchema` implementations (feature = "dyn").

use quote::{format_ident, quote};

use crate::utils::*;
use crate::attrs::*;
use crate::category::*;
use crate::field_info::*;
use crate::layout::*;

/// Return the canonical `&'static DynValueType` reference for a primitive type name,
/// or `None` if the type is not a known primitive.
fn primitive_vt_ref(type_name: &str) -> Option<proc_macro2::TokenStream> {
    match type_name {
        "u8" => Some(quote! { &::mmapstruct::schema::VT_U8 }),
        "u16" => Some(quote! { &::mmapstruct::schema::VT_U16 }),
        "u32" => Some(quote! { &::mmapstruct::schema::VT_U32 }),
        "u64" => Some(quote! { &::mmapstruct::schema::VT_U64 }),
        "i8" => Some(quote! { &::mmapstruct::schema::VT_I8 }),
        "i16" => Some(quote! { &::mmapstruct::schema::VT_I16 }),
        "i32" => Some(quote! { &::mmapstruct::schema::VT_I32 }),
        "i64" => Some(quote! { &::mmapstruct::schema::VT_I64 }),
        "f32" => Some(quote! { &::mmapstruct::schema::VT_F32 }),
        "f64" => Some(quote! { &::mmapstruct::schema::VT_F64 }),
        "bool" => Some(quote! { &::mmapstruct::schema::VT_BOOL }),
        "usize" => Some(quote! { &::mmapstruct::schema::VT_USIZE }),
        "isize" => Some(quote! { &::mmapstruct::schema::VT_ISIZE }),
        _ => None,
    }
}

/// Find a field's byte offset within the fixed section.
fn fields_field_offset(
    name: &proc_macro2::Ident,
    fields: &[FieldInfo],
    fi: &FieldInfo,
) -> proc_macro2::TokenStream {
    let fo_name = format_ident!("__MMAP_FO_{}", name);
    if is_pod_field(&fi.category)
        || is_inline_string_field(&fi.category)
        || (fi.is_fixed_byref())
    {
        let idx = fixed_section_index(fields, fi.name.as_str());
        quote! { #fo_name[#idx] as u64 }
    } else {
        quote! { 0u64 }
    }
}

/// Returns (DynValueType expression, resolver functions) for a field.
fn field_value_type(
    struct_name: &proc_macro2::Ident,
    fi: &FieldInfo,
    field_ident: &proc_macro2::Ident,
) -> (proc_macro2::TokenStream, Vec<proc_macro2::TokenStream>) {
    match &fi.category {
        // PodPrimitive is handled by caller via primitive_vt_ref canonical statics.
        FieldCategory::PodPrimitive => unreachable!(),
        FieldCategory::PodCustom => {
            let ty = &fi.ty;
            let resolve_name = format_ident!("__mmap_resolve_{}_{}", struct_name, field_ident);
            let resolver = quote! {
                #[allow(non_snake_case)]
                fn #resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
                    <#ty as ::mmapstruct::schema::GetSchema>::get_struct_schema()
                }
            };

            (
                quote! { ::mmapstruct::schema::DynValueType::Struct(#resolve_name) },
                vec![resolver],
            )
        }
        FieldCategory::DynamicString => (
            quote! { ::mmapstruct::schema::DynValueType::Str(None) },
            Vec::new(),
        ),
        FieldCategory::InlineString { size } => {
            let s = *size as u64;
            (
                quote! { ::mmapstruct::schema::DynValueType::Str(Some(#s)) },
                Vec::new(),
            )
        }
        FieldCategory::DynamicVec {
            inner,
            elem_is_pod,
            elem_fixed,
        } => {
            let arr_name = format_ident!("__mmap_ARR_{}_{}", struct_name, field_ident);
            let elem_vt_name = format_ident!("__mmap_VT_{}_{}_elem", struct_name, field_ident);

            let layout_tok = if *elem_is_pod {
                let stride_expr = type_size_expr(inner);
                quote! { ::mmapstruct::schema::ArrayLayout::Pod { stride: #stride_expr as u64 } }
            } else if *elem_fixed {
                let stride_expr = type_size_expr(inner);
                quote! { ::mmapstruct::schema::ArrayLayout::Fixed { stride: #stride_expr as u64 } }
            } else {
                quote! { ::mmapstruct::schema::ArrayLayout::Indexed }
            };

            let ty_str = type_ident_name(inner).unwrap_or_default();
            if let Some(canonical) = primitive_vt_ref(&ty_str) {
                let arr_static = quote! {
                    #[allow(non_upper_case_globals)]
                    static #arr_name: ::mmapstruct::schema::ArraySchema = ::mmapstruct::schema::ArraySchema {
                        count_offset: 0u64,
                        elem_type: #canonical,
                        layout: #layout_tok,
                    };
                };
                let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Array(&#arr_name) };
                return (vt_expr, vec![arr_static]);
            }

            let (elem_vt_expr, resolver_fns) = if is_string_type(inner) {
                (
                    quote! { ::mmapstruct::schema::DynValueType::Str(None) },
                    Vec::new(),
                )
            } else if *elem_is_pod {
                let resolve_name =
                    format_ident!("__mmap_resolve_{}_{}_elem", struct_name, field_ident);
                let resolver = quote! {
                    #[allow(non_snake_case)]
                    fn #resolve_name() -> &'static ::mmapstruct::schema::StructSchema {
                        <#inner as ::mmapstruct::schema::GetSchema>::get_struct_schema()
                    }
                };
                (
                    quote! { ::mmapstruct::schema::DynValueType::Struct(#resolve_name) },
                    vec![resolver],
                )
            } else {
                let resolve_name =
                    format_ident!("__mmap_resolve_{}_{}_elem", struct_name, field_ident);
                let resolver = quote! {
                    #[allow(non_snake_case)]
                    fn #resolve_name() -> &'static ::mmapstruct::schema::DynValueType {
                        <#inner as ::mmapstruct::schema::GetSchema>::root_value_type()
                    }
                };
                (
                    quote! { ::mmapstruct::schema::DynValueType::Lazy(#resolve_name) },
                    vec![resolver],
                )
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

            let (inner_ref, extra) =
                inner_dyn_value_type(struct_name, field_ident, inner);

            let elem_byref_name = format_ident!("__mmap_VT_{}_{}_byref", struct_name, field_ident);
            let arr_static = quote! {
                #[allow(non_upper_case_globals)]
                static #elem_byref_name: ::mmapstruct::schema::DynValueType = ::mmapstruct::schema::DynValueType::ByRef(#inner_ref);
                #[allow(non_upper_case_globals)]
                static #arr_name: ::mmapstruct::schema::ArraySchema = ::mmapstruct::schema::ArraySchema {
                    count_offset: 0u64,
                    elem_type: &#elem_byref_name,
                    layout: ::mmapstruct::schema::ArrayLayout::Indexed,
                };
            };

            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Array(&#arr_name) };
            let mut all = extra;
            all.push(arr_static);
            (vt_expr, all)
        }
        FieldCategory::DynamicOption { inner } => {
            let (inner_ref, extra) = inner_dyn_value_type(struct_name, field_ident, inner);
            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Option(#inner_ref) };
            (vt_expr, extra)
        }
        FieldCategory::DynamicHashMap {
            key_ty,
            val_ty,
            key_layout,
            val_layout,
        } => {
            let map_schema_name = format_ident!("__mmap_HM_{}_{}", struct_name, field_ident);
            let (key_ref, key_extra) = inner_dyn_value_type(
                struct_name,
                &format_ident!("{}_{}_key", field_ident, ""),
                key_ty,
            );
            let (val_ref, val_extra) = inner_dyn_value_type(
                struct_name,
                &format_ident!("{}_{}_val", field_ident, ""),
                val_ty,
            );
            let key_is_pod = matches!(key_layout, HashMapLayout::Pod);
            let val_is_pod = matches!(val_layout, HashMapLayout::Pod);
            let map_schema_static = quote! {
                #[allow(non_upper_case_globals)]
                static #map_schema_name: ::mmapstruct::schema::HashMapSchema = ::mmapstruct::schema::HashMapSchema {
                    key_type: #key_ref,
                    value_type: #val_ref,
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
                #[allow(non_snake_case)]
                fn #resolve_name() -> &'static ::mmapstruct::schema::DynValueType {
                    <#ty as ::mmapstruct::schema::GetSchema>::root_value_type()
                }
            };
            let vt_expr = quote! { ::mmapstruct::schema::DynValueType::Lazy(#resolve_name) };
            (vt_expr, vec![resolver])
        }
    }
}

/// Generate inner DynValueType for Option's inner type or similar nested types.
///
/// Returns `(ref_expr, extra_defs)` where `ref_expr` is a `&'static DynValueType`
/// expression and `extra_defs` are any supporting static/function definitions.
fn inner_dyn_value_type(
    struct_name: &proc_macro2::Ident,
    field_ident: &proc_macro2::Ident,
    inner_ty: &syn::Type,
) -> (proc_macro2::TokenStream, Vec<proc_macro2::TokenStream>) {
    if is_primitive_pod(inner_ty) {
        let ty_str = type_ident_name(inner_ty).unwrap_or_default();
        if let Some(canonical) = primitive_vt_ref(&ty_str) {
            return (canonical, Vec::new());
        }
        return (quote! { &::mmapstruct::schema::VT_U32 }, Vec::new());
    }
    if is_string_type(inner_ty) {
        return (quote! { &::mmapstruct::schema::VT_STR_DYNAMIC }, Vec::new());
    }
    let resolve_name = format_ident!("__mmap_resolve_{}_{}_inner", struct_name, field_ident);
    let vt_name = format_ident!("__mmap_VT_{}_{}_inner", struct_name, field_ident);
    let resolver = quote! {
        #[allow(non_snake_case)]
        fn #resolve_name() -> &'static ::mmapstruct::schema::DynValueType {
            <#inner_ty as ::mmapstruct::schema::GetSchema>::root_value_type()
        }
    };
    let vt_static = quote! {
        #[allow(non_upper_case_globals)]
        static #vt_name: ::mmapstruct::schema::DynValueType =
            ::mmapstruct::schema::DynValueType::Lazy(#resolve_name);
    };
    (quote! { &#vt_name }, vec![resolver, vt_static])
}

struct FieldSchema {
    descriptors: Vec<proc_macro2::TokenStream>,
    value_type_statics: Vec<proc_macro2::TokenStream>,
    resolver_fns: Vec<proc_macro2::TokenStream>,
    inline_dyn_count: usize,
}

/// Build field descriptors, value type statics, and resolver functions for schema generation.
///
/// Shared between struct (`fields_field_offset`) and enum variant (`variant_field_offset`).
fn build_field_schema(
    fields: &[FieldInfo],
    prefix: &proc_macro2::Ident,
    offset_fn: impl Fn(&[FieldInfo], &FieldInfo) -> proc_macro2::TokenStream,
) -> FieldSchema {
    let mut next_dyn_idx = 0usize;
    let field_dyn_indices: Vec<Option<usize>> = fields
        .iter()
        .map(|fi| {
            if is_pod_field(&fi.category)
                || is_inline_string_field(&fi.category)
                || (fi.is_fixed_byref())
            {
                None
            } else {
                let idx = next_dyn_idx;
                next_dyn_idx += 1;
                Some(idx)
            }
        })
        .collect();

    let mut value_type_statics = Vec::new();
    let mut resolver_fns = Vec::new();
    let mut field_refs = Vec::<proc_macro2::TokenStream>::new();

    for fi in fields.iter() {
        let vt_name = format_ident!("__mmap_VT_{}_{}", prefix, fi.ident);
        if matches!(fi.category, FieldCategory::PodPrimitive) {
            let ty_str = type_ident_name(&fi.ty).unwrap_or_default();
            let canonical = primitive_vt_ref(&ty_str)
                .expect("PodPrimitive must have a canonical VT_* ref");
            field_refs.push(canonical);
        } else if matches!(fi.category, FieldCategory::DynamicString) {
            field_refs.push(quote! { &::mmapstruct::schema::VT_STR_DYNAMIC });
        } else {
            let (vt_expr, extra) = field_value_type(prefix, fi, &fi.ident);
            value_type_statics.push(quote! {
                #[allow(non_upper_case_globals)]
                static #vt_name: ::mmapstruct::schema::DynValueType = #vt_expr;
            });
            field_refs.push(quote! { &#vt_name });
            resolver_fns.extend(extra);
        }
    }

    let descriptors: Vec<_> = fields
        .iter()
        .enumerate()
        .map(|(i, fi)| {
            let field_name = &fi.name;
            let off = offset_fn(fields, fi);
            let dyn_idx = field_dyn_indices[i];
            let inline_dyn_idx_tok = match dyn_idx {
                Some(idx) => quote! { Some(#idx) },
                None => quote! { None },
            };
            let is_byref = fi.is_byref;
            let ref_expr = &field_refs[i];
            quote! {
                ::mmapstruct::schema::FieldDescriptor {
                    name: #field_name,
                    offset_in_fixed: #off,
                    inline_dyn_index: #inline_dyn_idx_tok,
                    value_type: #ref_expr,
                    is_byref: #is_byref,
                }
            }
        })
        .collect();

    let inline_dyn_count = field_dyn_indices.iter().filter(|x| x.is_some()).count();
    FieldSchema {
        descriptors,
        value_type_statics,
        resolver_fns,
        inline_dyn_count,
    }
}

/// Generate Schema + GetSchema for a struct type.
pub(crate) fn generate_schema_impl(
    name: &proc_macro2::Ident,
    fields: &[FieldInfo],
) -> proc_macro2::TokenStream {
    let schema_name = format_ident!("__mmap_SCHEMA_{}", name);
    let field_statics_name = format_ident!("__mmap_FIELDS_{}", name);

    let fs = build_field_schema(fields, name, |flds, fi| fields_field_offset(name, flds, fi));

    let inline_dyn_count = fs.inline_dyn_count;
    let fs_name = format_ident!("__MMAP_FS_{}", name);
    let hs_name = format_ident!("__MMAP_HS_{}", name);
    let name_str = name.to_string();
    let field_descriptors = &fs.descriptors;
    let field_count = fs.descriptors.len();

    let resolver_stream = fs.resolver_fns
        .into_iter()
        .collect::<proc_macro2::TokenStream>();
    let vt_stream = fs.value_type_statics
        .into_iter()
        .collect::<proc_macro2::TokenStream>();

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
        static #field_statics_name: [::mmapstruct::schema::FieldDescriptor; #field_count] = [#(#field_descriptors),*];
        #[allow(non_upper_case_globals)]
        static #schema_name: ::mmapstruct::schema::StructSchema = ::mmapstruct::schema::StructSchema {
            name: #name_str,
            fields: &#field_statics_name,
            fixed_section_size: #fs_name as u64,
            header_size: #hs_name as u64,
            inline_dyn_count: #inline_dyn_count,
        };
        impl ::mmapstruct::schema::GetSchema for #name {
            fn get_struct_schema() -> &'static ::mmapstruct::schema::StructSchema {
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

/// Compute offset_in_fixed for a field in an enum variant.
/// Mirrors the layout computation in `handle_enum`.
fn variant_field_offset(fields: &[FieldInfo], fi: &FieldInfo) -> proc_macro2::TokenStream {
    fn is_fixed(f: &FieldInfo) -> bool {
        is_pod_field(&f.category)
            || is_inline_string_field(&f.category)
            || f.is_fixed_byref()
    }
    if !is_fixed(fi) {
        return quote! { 0u64 };
    }
    let mut offset = 0usize;
    for f in fields {
        if f.name == fi.name {
            break;
        }
        if !is_fixed(f) {
            continue;
        }
        if is_pod_field(&f.category) {
            let sz = known_primitive_size(&f.ty).unwrap_or(8);
            let al = if sz == 1 { 1 } else { sz.min(8).max(1) };
            offset = align_offset(offset, al);
            offset += sz;
        } else if is_inline_string_field(&f.category) {
            if let FieldCategory::InlineString { size } = &f.category {
                offset += size;
            }
        } else if f.is_byref {
            offset = align_offset(offset, 4);
            offset += 8;
        }
    }
    quote! { #offset as u64 }
}

/// Generate Schema + GetSchema for an enum type.
pub(crate) fn generate_enum_schema_impl(
    name: &proc_macro2::Ident,
    vars: &[super::enum_handler::VarGen],
) -> proc_macro2::TokenStream {
    let enum_schema_name = format_ident!("__mmap_ENUM_{}", name);
    let name_str = name.to_string();

    let mut variant_schema_statics = Vec::new();
    let mut variant_schema_refs = Vec::new();

    for v in vars.iter() {
        let vident = &v.ident;

        if v.fields.is_empty() {
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
            continue;
        }

        let fields = &v.fields;
        let vname = format_ident!("{}_{}", name, vident);

        let fs = build_field_schema(fields, &vname, variant_field_offset);

        let inline_dyn_count = fs.inline_dyn_count;
        let schema_name = format_ident!("__mmap_ENUMVAR_{}_{}", name, vident);
        let field_statics_name = format_ident!("__mmap_ENUMVAR_FIELDS_{}_{}", name, vident);
        let field_descriptors = &fs.descriptors;
        let field_count = fs.descriptors.len();
        let vt_stream = fs.value_type_statics
            .into_iter()
            .collect::<proc_macro2::TokenStream>();
        let resolver_stream = fs.resolver_fns
            .into_iter()
            .collect::<proc_macro2::TokenStream>();

        let mut fixed_offset = 0usize;
        for fi in fields {
            if is_pod_field(&fi.category) {
                let sz = known_primitive_size(&fi.ty).unwrap_or(8);
                let al = if sz == 1 { 1 } else { sz.min(8).max(1) };
                fixed_offset = align_offset(fixed_offset, al);
                fixed_offset += sz;
            } else if is_inline_string_field(&fi.category) {
                if let FieldCategory::InlineString { size } = &fi.category {
                    fixed_offset += size;
                }
            } else if fi.is_fixed_byref()
            {
                fixed_offset = align_offset(fixed_offset, 4);
                fixed_offset += 8;
            }
        }
        fixed_offset = align_offset(fixed_offset, 8);
        let fixed_sz = fixed_offset;
        let hdr_sz = fixed_sz
            + if inline_dyn_count > 0 {
                (inline_dyn_count + 1) * 8
            } else {
                0
            };

        let variant_name_str = format!("{}::{}", name_str, vident);
        variant_schema_statics.push(quote! {
            #resolver_stream
            #vt_stream
            #[allow(non_upper_case_globals)]
            static #field_statics_name: [::mmapstruct::schema::FieldDescriptor; #field_count] = [#(#field_descriptors),*];
            #[allow(non_upper_case_globals)]
            static #schema_name: ::mmapstruct::schema::StructSchema = ::mmapstruct::schema::StructSchema {
                name: #variant_name_str,
                fields: &#field_statics_name,
                fixed_section_size: #fixed_sz as u64,
                header_size: #hdr_sz as u64,
                inline_dyn_count: #inline_dyn_count,
            };
        });
        variant_schema_refs.push(quote! { &#schema_name });
    }

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
            disc_size: 4u64,
        };
        impl ::mmapstruct::schema::GetSchema for #name {
            fn get_struct_schema() -> &'static ::mmapstruct::schema::StructSchema {
                panic!("GetSchema::get_struct_schema() called on enum type {}; use root_value_type() instead", #name_str);
            }
            fn root_value_type() -> &'static ::mmapstruct::schema::DynValueType {
                static VT: ::mmapstruct::schema::DynValueType = ::mmapstruct::schema::DynValueType::Enum(&#enum_schema_name);
                &VT
            }
        }
    }
}
