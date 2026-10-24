//! Helper traits and utilities for working with schema type definitions.

use crate::schema::{ArrayLayoutDefReader, TypeDefReader};

#[inline(always)]
pub(crate) const fn align_up(offset: usize, align: usize) -> usize {
    (offset + align - 1) & !(align - 1)
}

pub(crate) trait TypeDefReaderExt {
    fn is_fixed_size(&self) -> bool;
    #[allow(dead_code)]
    fn is_inline_dynamic(&self) -> bool;
    fn is_byref_kind(&self) -> bool;
    fn is_option_kind(&self) -> bool;
}

impl TypeDefReaderExt for TypeDefReader<'_> {
    fn is_fixed_size(&self) -> bool {
        match self {
            TypeDefReader::Primitive(_) => true,
            TypeDefReader::PodStruct { .. } => true,
            TypeDefReader::Str { size } => size.is_some(),
            TypeDefReader::Array { layout, .. } => {
                matches!(layout, ArrayLayoutDefReader::FixedPod { .. })
            }
            TypeDefReader::Enum { .. } => true,
            // A derived struct with no inline-dynamic fields is a plain pod
            // blob (MmapPod) stored raw in the parent's fixed section.
            TypeDefReader::Struct {
                inline_dyn_count, ..
            } => **inline_dyn_count == 0,
            _ => false,
        }
    }

    #[allow(dead_code)]
    fn is_inline_dynamic(&self) -> bool {
        matches!(self, TypeDefReader::Str { size } if size.is_none())
    }

    fn is_byref_kind(&self) -> bool {
        matches!(self, TypeDefReader::ByRef { .. })
    }

    fn is_option_kind(&self) -> bool {
        matches!(self, TypeDefReader::Option { .. })
    }
}
