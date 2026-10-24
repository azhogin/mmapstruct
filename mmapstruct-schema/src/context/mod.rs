//! Zero-copy runtime schema context — navigate mmapstruct data using a
//! serialized `Schema` (from `mmapstruct-schema`) without needing
//! `#[derive(MmapStruct)]` at compile time.
//!
//! The schema is read with `Schema::Reader<'_>` — **no** `String`/`Vec` copies.
//! All navigation is zero-copy over both `data` and `schema_bytes`.

mod array;
mod hash_map;
mod mixed_struct;
mod variant;

pub use mixed_struct::FieldInfo;

use crate::value::SchemaValue;
use crate::schema::{ArrayLayoutDefReader, FieldDef, PrimitiveDef, Schema, TypeDefReader};
use mmapstruct::indexed_array::IndexedArrayReader;
use mmapstruct::traits::{mmap_read_pod, MmapRead};

// ---------------------------------------------------------------------------
// RuntimeSchemaCtx
// ---------------------------------------------------------------------------

/// Zero-copy context for reading mmapstruct data using a serialized schema.
///
/// Eagerly resolves all type entries from the schema's type table into
/// `TypeDefReader` enums (still zero-copy — just discriminant + field
/// readers pointing into `schema_bytes`).
pub struct RuntimeSchemaCtx<'a> {
    data: &'a [u8],
    /// Eagerly resolved type readers. Small table (usually < 100 entries).
    types: Vec<TypeDefReader<'a>>,
}

impl<'a> RuntimeSchemaCtx<'a> {
    /// Create a new context backed by `data` (the binary payload) and
    /// `schema_bytes` (a serialized `Schema`).
    ///
    /// Pre-resolves every type entry in the schema table so that subsequent
    /// navigation can read `TypeDefReader` discriminants quickly.
    pub fn new(data: &'a [u8], schema_bytes: &'a [u8]) -> Self {
        let schema =
            <Schema as MmapRead>::read(schema_bytes, 0..schema_bytes.len());
        let type_count = schema.types.len();
        let mut types = Vec::with_capacity(type_count);
        for i in 0..type_count {
            let ty = schema.types.get(i).unwrap();
            types.push(ty);
        }
        Self {
            data,
            types,
        }
    }

    pub fn data(&self) -> &[u8] {
        self.data
    }

    pub fn read_pod<T: bytemuck::Pod>(&self, v: &SchemaValue) -> T {
        let off = v.mmap_begin as usize;
        unsafe { mmap_read_pod::<T>(&self.data[off..off + core::mem::size_of::<T>()]) }
    }

    /// Read raw bytes for a pod struct.
    pub fn read_pod_bytes(&self, v: &SchemaValue) -> &'a [u8] {
        &self.data[v.mmap_begin as usize..v.mmap_end as usize]
    }

    pub fn read_u32(&self, v: &SchemaValue) -> u32 { self.read_pod(v) }
    pub fn read_i32(&self, v: &SchemaValue) -> i32 { self.read_pod(v) }
    pub fn read_u64(&self, v: &SchemaValue) -> u64 { self.read_pod(v) }
    pub fn read_i64(&self, v: &SchemaValue) -> i64 { self.read_pod(v) }
    pub fn read_f32(&self, v: &SchemaValue) -> f32 { self.read_pod(v) }
    pub fn read_f64(&self, v: &SchemaValue) -> f64 { self.read_pod(v) }
    pub fn read_u16(&self, v: &SchemaValue) -> u16 { self.read_pod(v) }
    pub fn read_i16(&self, v: &SchemaValue) -> i16 { self.read_pod(v) }
    pub fn read_u8(&self, v: &SchemaValue) -> u8 { self.read_pod(v) }
    pub fn read_i8(&self, v: &SchemaValue) -> i8 { self.read_pod(v) }
    pub fn read_usize(&self, v: &SchemaValue) -> usize { self.read_u64(v) as usize }
    pub fn read_isize(&self, v: &SchemaValue) -> isize { self.read_i64(v) as isize }
    pub fn read_bool(&self, v: &SchemaValue) -> bool { self.read_pod::<u32>(v) != 0 }

    /// Read a string using the Value's mmap_begin..mmap_end range.
    /// Trailing null bytes are trimmed (for `#[mmap(inline(N))]` strings).
    pub fn read_str(&self, v: &SchemaValue) -> &'a str {
        let raw = &self.data[v.mmap_begin as usize..v.mmap_end as usize];
        let trimmed = if let Some(null_pos) = raw.iter().position(|&b| b == 0) {
            &raw[..null_pos]
        } else {
            raw
        };
        // SAFETY: written by derive with valid UTF-8
        unsafe { std::str::from_utf8_unchecked(trimmed) }
    }

    /// Read a numeric value as `i64` (widening conversion).
    /// Returns `None` for non-numeric or string types.
    pub fn read_as_i64(&self, v: &SchemaValue) -> Option<i64> {
        let t = self.type_reader(v.type_idx);
        match t {
            TypeDefReader::Primitive(p) => match p {
                PrimitiveDef::U32 => Some(self.read_u32(v) as i64),
                PrimitiveDef::U64 => Some(self.read_u64(v) as i64),
                PrimitiveDef::I32 => Some(self.read_i32(v) as i64),
                PrimitiveDef::I64 => Some(self.read_i64(v)),
                PrimitiveDef::F32 => Some(self.read_f32(v) as i64),
                PrimitiveDef::F64 => Some(self.read_f64(v) as i64),
                PrimitiveDef::U16 => Some(self.read_u16(v) as i64),
                PrimitiveDef::I16 => Some(self.read_i16(v) as i64),
                PrimitiveDef::U8 => Some(self.read_u8(v) as i64),
                PrimitiveDef::I8 => Some(self.read_i8(v) as i64),
                PrimitiveDef::Usize => Some(self.read_usize(v) as i64),
                PrimitiveDef::Isize => Some(self.read_isize(v) as i64),
                PrimitiveDef::Bool => Some(self.read_u32(v) as i64),
            },
            _ => None,
        }
    }

    /// Read a numeric value as `f64` (widening conversion).
    pub fn read_as_f64(&self, v: &SchemaValue) -> Option<f64> {
        let t = self.type_reader(v.type_idx);
        match t {
            TypeDefReader::Primitive(p) => match p {
                PrimitiveDef::U32 => Some(self.read_u32(v) as f64),
                PrimitiveDef::U64 => Some(self.read_u64(v) as f64),
                PrimitiveDef::I32 => Some(self.read_i32(v) as f64),
                PrimitiveDef::I64 => Some(self.read_i64(v) as f64),
                PrimitiveDef::F32 => Some(self.read_f32(v) as f64),
                PrimitiveDef::F64 => Some(self.read_f64(v)),
                PrimitiveDef::U16 => Some(self.read_u16(v) as f64),
                PrimitiveDef::I16 => Some(self.read_i16(v) as f64),
                PrimitiveDef::U8 => Some(self.read_u8(v) as f64),
                PrimitiveDef::I8 => Some(self.read_i8(v) as f64),
                PrimitiveDef::Usize => Some(self.read_usize(v) as f64),
                PrimitiveDef::Isize => Some(self.read_isize(v) as f64),
                _ => None,
            },
            _ => None,
        }
    }

    /// Get the resolved `TypeDefReader` for a type index.
    pub fn type_reader(&self, type_idx: u32) -> &TypeDefReader<'a> {
        &self.types[type_idx as usize]
    }

    /// Check if a type is an Array (for field_get traversal decisions).
    pub fn is_array_type(&self, type_idx: u32) -> bool {
        matches!(self.type_reader(type_idx), TypeDefReader::Array { .. })
    }

    /// Check if a type is a Struct (for field_get traversal decisions).
    pub fn is_struct_type(&self, type_idx: u32) -> bool {
        matches!(self.type_reader(type_idx), TypeDefReader::Struct { .. })
    }

    /// Check if a type is a String (for field_get traversal decisions).
    pub fn is_str_type(&self, type_idx: u32) -> bool {
        matches!(self.type_reader(type_idx), TypeDefReader::Str { .. })
    }

    /// Natural alignment for a type index (mirrors `core::mem::align_of` for
    /// concrete types).  Computed structurally for structs/enums/options so
    /// that schema-side alignment matches the writer, which uses the real
    /// Rust `align_of` (e.g. `Option<T>` inner data, HashMap value areas).
    pub(crate) fn type_align(&self, type_idx: u32) -> u64 {
        let t = self.type_reader(type_idx);
        match t {
            TypeDefReader::Primitive(p) => match p {
                PrimitiveDef::U8 | PrimitiveDef::I8 => 1,
                PrimitiveDef::Bool => 4,
                PrimitiveDef::U16 | PrimitiveDef::I16 => 2,
                PrimitiveDef::U32 | PrimitiveDef::I32 | PrimitiveDef::F32 => 4,
                PrimitiveDef::U64
                | PrimitiveDef::I64
                | PrimitiveDef::F64
                | PrimitiveDef::Usize
                | PrimitiveDef::Isize => 8,
            },
            // Opaque pod blob without field info: size-based heuristic.
            TypeDefReader::PodStruct { size } => {
                let s = **size as usize;
                if s == 0 {
                    1
                } else {
                    s.min(8).next_power_of_two() as u64
                }
            }
            // `String`'s Rust alignment is 8 (it is a Vec<u8> wrapper); the writer
            // aligns `Option<String>` inner data with `align_of::<String>()`,
            // so the schema side must mirror that, not the raw-bytes layout.
            TypeDefReader::Str { .. } => 8,
            TypeDefReader::Struct { fields, .. } => {
                // Mirror Rust's align_of: max over the fields' own alignments
                // (dyn fields contribute via their type: Vec/HashMap → 8,
                // String → 1, Option<T> → max(1, align of T), …).
                self.max_field_align(fields).unwrap_or(1)
            }
            TypeDefReader::Enum {
                variant_type_indices,
                disc_size,
                ..
            } => {
                // Mirror Rust's align_of::<Enum>: max over all variants'
                // field alignments, floored by the discriminant's alignment.
                let mut m = **disc_size;
                for vt in variant_type_indices.iter() {
                    if let TypeDefReader::Struct { fields, .. } = self.type_reader(*vt) {
                        if let Some(a) = self.max_field_align(fields) {
                            if a > m {
                                m = a;
                            }
                        }
                    }
                }
                m
            }
            TypeDefReader::Option { inner_type } => self.type_align(**inner_type).max(1),
            TypeDefReader::ByRef { inner_type } => self.type_align(**inner_type),
            TypeDefReader::Array { layout, .. } => match layout {
                ArrayLayoutDefReader::FixedPod { stride, .. } => {
                    let s = **stride as usize;
                    s.min(8).next_power_of_two() as u64
                }
                _ => 8,
            },
            _ => 8,
        }
    }

    /// Max alignment over a field list (`None` for empty lists).
    fn max_field_align(&self, fields: &IndexedArrayReader<'a, FieldDef>) -> Option<u64> {
        let n = fields.len();
        let mut m: Option<u64> = None;
        for i in 0..n {
            let f = fields.get(i).unwrap();
            let a = self.type_align(*f.type_index);
            if m.map_or(true, |mm| a > mm) {
                m = Some(a);
            }
        }
        m
    }

    fn type_size(&self, type_idx: u32) -> Option<u64> {
        let t = self.type_reader(type_idx);
        match t {
            TypeDefReader::Primitive(p) => match p {
                PrimitiveDef::U8 | PrimitiveDef::I8 => Some(1),
                PrimitiveDef::Bool => Some(4),
                PrimitiveDef::U16 | PrimitiveDef::I16 => Some(2),
                PrimitiveDef::U32 | PrimitiveDef::I32 | PrimitiveDef::F32 => Some(4),
                PrimitiveDef::U64
                | PrimitiveDef::I64
                | PrimitiveDef::F64
                | PrimitiveDef::Usize
                | PrimitiveDef::Isize => Some(8),
            },
            TypeDefReader::PodStruct { size } => Some(**size),
            TypeDefReader::Str { size } => {
                size.map(|s| *s as u64)
            }
            TypeDefReader::Array { layout, .. } => match layout {
                ArrayLayoutDefReader::FixedPod { stride, count } => {
                    Some(**stride * **count)
                }
                _ => None,
            },
            TypeDefReader::Enum { disc_size, .. } => Some(**disc_size),
            // Pod struct (no inline-dynamic fields): raw blob of its fixed size.
            TypeDefReader::Struct {
                fixed_section_size,
                inline_dyn_count,
                ..
            } => {
                if **inline_dyn_count == 0 {
                    Some(**fixed_section_size)
                } else {
                    None
                }
            }
            // Struct, HashMap, Option, ByRef — not fixed-size;
            // their extent depends on offset tables, variant discriminants, or
            // dynamic content.  Callers that need the full range for these types
            // must resolve via offset tables or child fields instead.
            _ => None,
        }
    }

    /// Unwrap Option/ByRef wrappers to get the effective value type.
    pub fn unwrap_type(&self, type_idx: u32) -> u32 {
        let t = self.type_reader(type_idx);
        match t {
            TypeDefReader::Option { inner_type } => **inner_type,
            TypeDefReader::ByRef { inner_type } => **inner_type,
            _ => type_idx,
        }
    }

    /// Check if a type index wraps an Option.
    pub fn is_option_type(&self, type_idx: u32) -> bool {
        matches!(self.type_reader(type_idx), TypeDefReader::Option { .. })
    }

}
