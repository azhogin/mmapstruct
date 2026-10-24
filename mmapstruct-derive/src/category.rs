//! Field classification: determines how each struct field is stored.

use syn::Type;

use crate::attrs::*;
use crate::utils::*;

#[derive(Clone)]
// `syn::Type` is large and a derive-time value — boxing it would churn every
// codegen pattern for no practical benefit (instances live for the duration
// of one macro expansion).
#[allow(clippy::large_enum_variant)]
pub(crate) enum FieldCategory {
    PodPrimitive,
    PodCustom,
    DynamicString,
    InlineString {
        size: usize,
    },
    FixedArray {
        count: usize,
        elem_is_pod: bool,
    },
    DynamicVec {
        inner: Type,
        elem_is_pod: bool,
        elem_fixed: bool,
    },
    DynamicVecByRef {
        inner: Type,
        dedup: bool,
    },
    DynamicHashMap {
        key_ty: Type,
        val_ty: Type,
        key_layout: HashMapLayout,
        val_layout: HashMapLayout,
    },
    DynamicOption {
        inner: Type,
    },
    DynamicCustom,
}

pub(crate) fn is_pod_field(category: &FieldCategory) -> bool {
    matches!(
        category,
        FieldCategory::PodPrimitive
            | FieldCategory::PodCustom
            | FieldCategory::FixedArray {
                elem_is_pod: true,
                ..
            }
    )
}

pub(crate) fn is_inline_string_field(category: &FieldCategory) -> bool {
    matches!(category, FieldCategory::InlineString { .. })
}

pub(crate) fn is_fixed_array_field(category: &FieldCategory) -> bool {
    matches!(category, FieldCategory::FixedArray { .. })
}

pub(crate) fn classify_field(field: &syn::Field) -> FieldCategory {
    let ty = &field.ty;
    if is_primitive_pod(ty) {
        return FieldCategory::PodPrimitive;
    }
    let (is_byref, dedup, inline_size, _is_bool) = parse_mmap_attrs(&field.attrs);
    if let Some(size) = inline_size {
        if is_string_type(ty) {
            return FieldCategory::InlineString { size };
        }
        panic!("#[mmap(inline(N))] only supported on String fields");
    }
    if is_string_type(ty) {
        return FieldCategory::DynamicString;
    }
    if let Some(inner) = extract_vec_inner(ty) {
        if is_byref {
            return FieldCategory::DynamicVecByRef { inner, dedup };
        }
        let elem_is_pod = is_primitive_pod(&inner) || has_pod_attr(&field.attrs);
        let elem_fixed = has_fixed_attr(&field.attrs);
        return FieldCategory::DynamicVec {
            inner,
            elem_is_pod,
            elem_fixed,
        };
    }
    if let Some(inner) = extract_option_inner(ty) {
        return FieldCategory::DynamicOption { inner };
    }
    if let Some((key_ty, val_ty)) = extract_hashmap_inner(ty) {
        let (key_attr, val_attr) = parse_hashmap_attrs(&field.attrs);
        let key_layout = key_attr.unwrap_or_else(|| detect_layout(&key_ty));
        let val_layout = val_attr.unwrap_or_else(|| detect_layout(&val_ty));
        return FieldCategory::DynamicHashMap {
            key_ty,
            val_ty,
            key_layout,
            val_layout,
        };
    }
    if let Some((inner, count)) = extract_fixed_array_inner(ty) {
        let elem_is_pod = is_pod_type(&inner);
        return FieldCategory::FixedArray { count, elem_is_pod };
    }
    if has_pod_attr(&field.attrs) {
        return FieldCategory::PodCustom;
    }
    FieldCategory::DynamicCustom
}
