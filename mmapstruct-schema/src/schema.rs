//! Schema type definitions — MmapStruct-backed binary format.
//!
//! These types are used to describe mmapstruct data layouts at runtime,
//! enabling zero-copy navigation without compile-time struct definitions.

use mmapstruct::MmapStruct;

// ---------------------------------------------------------------------------
// Serialized schema types — MmapStruct-backed binary format
// ---------------------------------------------------------------------------

/// Top-level schema descriptor.
#[derive(MmapStruct, Debug, PartialEq)]
pub struct Schema {
    /// Flat table of all type definitions referenced by the struct.
    pub types: Vec<TypeDef>,
    /// Index into `types` for the root type.
    pub root_type: u32,
}

/// Primitive scalar types.
#[derive(MmapStruct, Debug, PartialEq, Clone, Copy)]
#[repr(u32)]
pub enum PrimitiveDef {
    U8,
    U16,
    U32,
    U64,
    I8,
    I16,
    I32,
    I64,
    F32,
    F64,
    Bool,
    Usize,
    Isize,
}

impl From<&PrimitiveDef> for PrimitiveDef {
    fn from(v: &PrimitiveDef) -> Self {
        *v
    }
}

/// A single type definition in the schema type table.
#[derive(MmapStruct, Debug, PartialEq)]
pub enum TypeDef {
    Primitive(PrimitiveDef),
    Str {
        size: Option<u32>,
    },
    PodStruct {
        size: u64,
    },
    Struct {
        name: String,
        fields: Vec<FieldDef>,
        fixed_section_size: u64,
        header_size: u64,
        inline_dyn_count: u32,
    },
    Enum {
        name: String,
        variants: Vec<VariantDef>,
        variant_type_indices: Vec<u32>,
        disc_size: u64,
    },
    Array {
        elem_type: u32,
        layout: ArrayLayoutDef,
    },
    Option {
        inner_type: u32,
    },
    ByRef {
        inner_type: u32,
    },
    HashMap {
        key_type: u32,
        val_type: u32,
        #[mmap(bool)]
        key_is_pod: u32,
        #[mmap(bool)]
        val_is_pod: u32,
    },
}

/// A single variant inside an enum type definition.
#[derive(MmapStruct, Debug, PartialEq)]
pub struct VariantDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub fixed_section_size: u64,
    pub header_size: u64,
    pub inline_dyn_count: u32,
}

/// A single field inside a struct or variant.
#[derive(MmapStruct, Debug, PartialEq)]
pub struct FieldDef {
    pub name: String,
    pub type_index: u32,
    pub offset_in_fixed: u64,
    pub inline_dyn_index: Option<u32>,
}

/// Memory layout strategy for array elements.
#[derive(MmapStruct, Debug, PartialEq)]
pub enum ArrayLayoutDef {
    Pod { stride: u64 },
    FixedPod { stride: u64, count: u64 },
    Indexed,
    Fixed { stride: u64 },
}
