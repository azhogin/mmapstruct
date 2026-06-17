//! Zero-copy Schema-based DynContext — navigate mmapstruct data using a
//! serialized `Schema` (from `mmapstruct-schema`) without needing
//! `#[derive(MmapStruct)]` at compile time.
//!
//! The schema is read with `Schema::Reader<'_>` — **no** `String`/`Vec` copies.
//! All navigation is zero-copy over both `data` and `schema_bytes`.

use crate::{
    ArrayLayoutDefReader, FieldDef, FieldDefReader, Schema, SchemaReader, SchemaTypeDefReader,
    VariantDefReader,
};
use mmapstruct::indexed_array::IndexedArrayReader;
use mmapstruct::traits::{mmap_read_pod, ByRefRange, MmapRead};

/// Lightweight value descriptor for schema-based navigation.
/// Analogous to `mmapstruct::schema::Value` but carries a `type_idx` into
/// the schema's type table instead of a `&'static DynValueType`.
#[derive(Clone, Copy, Debug)]
pub struct SchemaValue {
    pub offset: u64,
    pub limit: u64,
    pub type_idx: u32,
}

impl SchemaValue {
    pub fn new(offset: u64, limit: u64, type_idx: u32) -> Self {
        Self {
            offset,
            limit,
            type_idx,
        }
    }
}

// ---------------------------------------------------------------------------
// SchemaDynContext
// ---------------------------------------------------------------------------

/// Zero-copy context for reading mmapstruct data using a serialized schema.
///
/// Stores a `Schema::Reader<'_>` (borrowed from `schema_bytes`) and eagerly
/// resolves all type entries from the schema's type table into
/// `SchemaTypeDefReader` enums (still zero-copy — just discriminant + field
/// readers pointing into `schema_bytes`).
pub struct SchemaDynContext<'a> {
    data: &'a [u8],
    schema: SchemaReader<'a>,
    /// Eagerly resolved type readers. Small table (usually < 100 entries).
    types: Vec<SchemaTypeDefReader<'a>>,
}

impl<'a> SchemaDynContext<'a> {
    /// Build a context from raw data bytes and the schema bytes that
    /// represent a serialized `Schema`.
    pub fn new(data: &'a [u8], schema_bytes: &'a [u8]) -> Self {
        let schema = <Schema as MmapRead>::read(schema_bytes, 0..schema_bytes.len());
        let count = schema.types.len();
        let mut types = Vec::with_capacity(count);
        for i in 0..count {
            types.push(schema.types.get(i).expect("Schema type index out of bounds"));
        }
        Self {
            data,
            schema,
            types,
        }
    }

    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Borrow the underyling schema reader (e.g. for debugging).
    pub fn schema_reader(&self) -> &SchemaReader<'a> {
        &self.schema
    }

    // ------------------------------------------------------------------
    // Scalar reads (same as DynContext — just read from offset)
    // ------------------------------------------------------------------

    pub fn read_u32(&self, v: &SchemaValue) -> u32 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<u32>(&self.data[off..off + 4]) }
    }

    pub fn read_u64(&self, v: &SchemaValue) -> u64 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<u64>(&self.data[off..off + 8]) }
    }

    pub fn read_i32(&self, v: &SchemaValue) -> i32 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<i32>(&self.data[off..off + 4]) }
    }

    pub fn read_i64(&self, v: &SchemaValue) -> i64 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<i64>(&self.data[off..off + 8]) }
    }

    pub fn read_f32(&self, v: &SchemaValue) -> f32 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<f32>(&self.data[off..off + 4]) }
    }

    pub fn read_f64(&self, v: &SchemaValue) -> f64 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<f64>(&self.data[off..off + 8]) }
    }

    pub fn read_bool(&self, v: &SchemaValue) -> bool {
        self.data[v.offset as usize] != 0
    }

    pub fn read_u16(&self, v: &SchemaValue) -> u16 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<u16>(&self.data[off..off + 2]) }
    }

    pub fn read_i16(&self, v: &SchemaValue) -> i16 {
        let off = v.offset as usize;
        unsafe { mmap_read_pod::<i16>(&self.data[off..off + 2]) }
    }

    pub fn read_u8(&self, v: &SchemaValue) -> u8 {
        self.data[v.offset as usize]
    }

    pub fn read_i8(&self, v: &SchemaValue) -> i8 {
        self.data[v.offset as usize] as i8
    }

    pub fn read_usize(&self, v: &SchemaValue) -> usize {
        self.read_u64(v) as usize
    }

    pub fn read_isize(&self, v: &SchemaValue) -> isize {
        self.read_i64(v) as isize
    }

    pub fn read_str_opt(&self, v: &SchemaValue) -> Option<&'a str> {
        if matches!(
            self.type_reader(v.type_idx),
            SchemaTypeDefReader::Str { .. }
        ) {
            Some(self.read_str(v))
        } else {
            None
        }
    }

    pub fn read_str(&self, v: &SchemaValue) -> &'a str {
        let raw = &self.data[v.offset as usize..v.limit as usize];
        // Trim trailing null bytes (inline strings are null-padded)
        let trimmed = if let Some(null_pos) = raw.iter().position(|&b| b == 0) {
            &raw[..null_pos]
        } else {
            raw
        };
        unsafe { std::str::from_utf8_unchecked(trimmed) }
    }

    /// Read a numeric value as `i64` (widening conversion).
    pub fn read_as_i64(&self, v: &SchemaValue) -> Option<i64> {
        match &self.types[v.type_idx as usize] {
            SchemaTypeDefReader::U32 => Some(self.read_u32(v) as i64),
            SchemaTypeDefReader::U64 => Some(self.read_u64(v) as i64),
            SchemaTypeDefReader::I32 => Some(self.read_i32(v) as i64),
            SchemaTypeDefReader::I64 => Some(self.read_i64(v)),
            SchemaTypeDefReader::F32 => Some(self.read_f32(v) as i64),
            SchemaTypeDefReader::F64 => Some(self.read_f64(v) as i64),
            SchemaTypeDefReader::U16 => Some(self.read_u16(v) as i64),
            SchemaTypeDefReader::I16 => Some(self.read_i16(v) as i64),
            SchemaTypeDefReader::U8 => Some(self.read_u8(v) as i64),
            SchemaTypeDefReader::I8 => Some(self.read_i8(v) as i64),
            SchemaTypeDefReader::Usize => Some(self.read_usize(v) as i64),
            SchemaTypeDefReader::Isize => Some(self.read_isize(v) as i64),
            _ => None,
        }
    }

    /// Read a numeric value as `f64` (widening conversion).
    pub fn read_as_f64(&self, v: &SchemaValue) -> Option<f64> {
        match &self.types[v.type_idx as usize] {
            SchemaTypeDefReader::U32 => Some(self.read_u32(v) as f64),
            SchemaTypeDefReader::U64 => Some(self.read_u64(v) as f64),
            SchemaTypeDefReader::I32 => Some(self.read_i32(v) as f64),
            SchemaTypeDefReader::I64 => Some(self.read_i64(v) as f64),
            SchemaTypeDefReader::F32 => Some(self.read_f32(v) as f64),
            SchemaTypeDefReader::F64 => Some(self.read_f64(v)),
            SchemaTypeDefReader::U16 => Some(self.read_u16(v) as f64),
            SchemaTypeDefReader::I16 => Some(self.read_i16(v) as f64),
            SchemaTypeDefReader::U8 => Some(self.read_u8(v) as f64),
            SchemaTypeDefReader::I8 => Some(self.read_i8(v) as f64),
            SchemaTypeDefReader::Usize => Some(self.read_usize(v) as f64),
            SchemaTypeDefReader::Isize => Some(self.read_isize(v) as f64),
            _ => None,
        }
    }

    // ------------------------------------------------------------------
    // Type helpers
    // ------------------------------------------------------------------

    pub fn type_reader(&self, type_idx: u32) -> &SchemaTypeDefReader<'a> {
        &self.types[type_idx as usize]
    }

    pub fn is_array_type(&self, val: &SchemaValue) -> bool {
        matches!(
            self.type_reader(val.type_idx),
            SchemaTypeDefReader::Array { .. }
        )
    }

    pub fn is_str_type(&self, val: &SchemaValue) -> bool {
        matches!(
            self.type_reader(val.type_idx),
            SchemaTypeDefReader::Str { .. }
        )
    }

    pub(crate) fn type_align(&self, type_idx: u32) -> u64 {
        let t = self.type_reader(type_idx);
        match t {
            SchemaTypeDefReader::U8 | SchemaTypeDefReader::I8 | SchemaTypeDefReader::Bool => 1,
            SchemaTypeDefReader::U16 | SchemaTypeDefReader::I16 => 2,
            SchemaTypeDefReader::U32 | SchemaTypeDefReader::I32 | SchemaTypeDefReader::F32 => 4,
            SchemaTypeDefReader::U64
            | SchemaTypeDefReader::I64
            | SchemaTypeDefReader::F64
            | SchemaTypeDefReader::Usize
            | SchemaTypeDefReader::Isize => 8,
            SchemaTypeDefReader::PodStruct { size } => {
                let s = **size as usize;
                if s == 0 {
                    1
                } else {
                    s.min(8).next_power_of_two() as u64
                }
            }
            _ => 8,
        }
    }

    fn type_size(&self, type_idx: u32) -> Option<u64> {
        let t = self.type_reader(type_idx);
        match t {
            SchemaTypeDefReader::U8 | SchemaTypeDefReader::I8 | SchemaTypeDefReader::Bool => {
                Some(1)
            }
            SchemaTypeDefReader::U16 | SchemaTypeDefReader::I16 => Some(2),
            SchemaTypeDefReader::U32 | SchemaTypeDefReader::I32 | SchemaTypeDefReader::F32 => {
                Some(4)
            }
            SchemaTypeDefReader::U64
            | SchemaTypeDefReader::I64
            | SchemaTypeDefReader::F64
            | SchemaTypeDefReader::Usize
            | SchemaTypeDefReader::Isize => Some(8),
            SchemaTypeDefReader::PodStruct { size } => Some(**size),
            SchemaTypeDefReader::Str { def } => {
                def.inline_size.map(|s| *s as u64)
            }
            _ => None,
        }
    }

    /// Unwrap Option/ByRef wrappers to get the effective value type.
    fn unwrap_type(&self, type_idx: u32) -> u32 {
        let t = self.type_reader(type_idx);
        if let SchemaTypeDefReader::Option { def } = t {
            def.inner_type
        } else if let SchemaTypeDefReader::ByRef { def } = t {
            def.inner_type
        } else {
            type_idx
        }
    }

    fn is_option_type(&self, type_idx: u32) -> bool {
        matches!(self.type_reader(type_idx), SchemaTypeDefReader::Option { .. })
    }

    /// maximum type alignment across all fields of a struct/variant.
    fn max_field_align(&self, fields: &IndexedArrayReader<'a, FieldDef>) -> u64 {
        let mut max_align = 4u64;
        for f in fields {
            let fa = self.type_align(*f.type_index);
            if fa > max_align {
                max_align = fa;
            }
        }
        max_align.max(8)
    }

    /// Variant data offset: past discriminant, aligned to `max_align.max(8)`.
    fn variant_data_off(&self, val: &SchemaValue, variant: &VariantDefReader<'a>) -> u64 {
        let max_align = self.max_field_align(&variant.fields);
        align_up(val.offset as usize + 4, max_align as usize) as u64
    }

    // ------------------------------------------------------------------
    // Struct / enum field navigation
    // ------------------------------------------------------------------

    /// Navigate into a struct or enum field by name.
    pub fn field_get(&self, val: &SchemaValue, name: &str) -> Option<SchemaValue> {
        let t = self.type_reader(val.type_idx);
        match t {
            SchemaTypeDefReader::Struct { def } => {
                self.resolve_field_in_def(&def.fields, *def.fixed_section_size, *def.header_size, *def.inline_dyn_count as usize, name, val.offset)
            }
        SchemaTypeDefReader::Enum { def } => {
                let disc = u32::from_le_bytes(
                    self.data[val.offset as usize..val.offset as usize + 4]
                        .try_into()
                        .ok()?,
                ) as usize;
                let variant = def.variants.get(disc)?;
                let data_off = self.variant_data_off(val, &variant);
                self.resolve_field_in_def(
                    &variant.fields,
                    *variant.fixed_section_size,
                    *variant.header_size,
                    *variant.inline_dyn_count as usize,
                    name,
                    data_off,
                )
            }
            _ => None,
        }
    }

    /// Resolve an enum value's active variant + data offset.
    pub fn enum_variant(&self, val: &SchemaValue) -> Option<(VariantDefReader<'a>, u64)> {
        let t = self.type_reader(val.type_idx);
        let def = match t {
            SchemaTypeDefReader::Enum { def } => def,
            _ => return None,
        };
        let disc = u32::from_le_bytes(
            self.data[val.offset as usize..val.offset as usize + 4]
                .try_into()
                .ok()?,
        ) as usize;
        let variant = def.variants.get(disc)?;
        let data_off = self.variant_data_off(val, &variant);
        Some((variant, data_off))
    }

    fn resolve_field_in_def(
        &self,
        fields: &IndexedArrayReader<'a, FieldDef>,
        fixed_section_size: u64,
        header_size: u64,
        inline_dyn_count: usize,
        name: &str,
        base: u64,
    ) -> Option<SchemaValue> {
        let mut idx = 0usize;
        for field in fields {
            if field.name == name {
                return self.resolve_field_value(
                    base,
                    fixed_section_size,
                    header_size,
                    inline_dyn_count,
                    &field,
                    idx,
                );
            }
            idx += 1;
        }
        None
    }

    fn resolve_field_value(
        &self,
        base: u64,
        fixed_section_size: u64,
        header_size: u64,
        inline_dyn_count: usize,
        field: &FieldDefReader<'a>,
        _field_idx: usize,
    ) -> Option<SchemaValue> {
        // The effective type index — unwrap Option/ByRef wrappers to get
        // the actual value type (same logic as DynContext).
        let effective_type = self.unwrap_type(*field.type_index);

        if let Some(dyn_idx) = field.inline_dyn_index {
            // Inline-dynamic field — resolve via offset table
            let dyn_idx = *dyn_idx as usize;
            let ot_offset = base + fixed_section_size;
            let values_offset = base + header_size;

            let ot_start = ot_offset as usize;
            let off_i: u64 =
                unsafe { mmap_read_pod::<u64>(&self.data[ot_start + dyn_idx * 8..][..8]) };
            let off_next: u64 = if dyn_idx + 1 < inline_dyn_count {
                unsafe {
                    mmap_read_pod::<u64>(&self.data[ot_start + (dyn_idx + 1) * 8..][..8])
                }
            } else {
                unsafe {
                    mmap_read_pod::<u64>(
                        &self.data[ot_start + inline_dyn_count * 8..][..8],
                    )
                }
            };

            let data_off = values_offset + off_i;
            let data_limit = values_offset + off_next;

            if self.is_option_type(*field.type_index) {
                // Option inline-dynamic: skip u8 flag + alignment padding
                if self.data.get(data_off as usize).copied() == Some(0) {
                    // None → empty range signals absence
                    Some(SchemaValue::new(data_off, data_off, effective_type))
                } else {
                    let inner_align = self.type_align(effective_type) as usize;
                    let adjusted = align_up(data_off as usize + 1, inner_align) as u64;
                    Some(SchemaValue::new(adjusted, data_limit, effective_type))
                }
            } else {
                Some(SchemaValue::new(data_off, data_limit, effective_type))
            }
        } else if *field.is_byref != 0 {
            // By-ref field — read ByRefRange placeholder
            let off = base + *field.offset_in_fixed;
            let br: ByRefRange =
                unsafe { mmap_read_pod(&self.data[off as usize..off as usize + 8]) };
            if br.begin == 0 && br.end == 0 {
                // Sentinel {0,0} = None for optional byref fields
                return Some(SchemaValue::new(off, off, effective_type));
            }
            let data_off = base + br.begin as u64;
            let data_limit = base + br.end as u64;
            Some(SchemaValue::new(data_off, data_limit, effective_type))
        } else {
            // Pod field or inline string — fixed offset
            let fixed_off = base + *field.offset_in_fixed;
            let limit = if let Some(sz) = self.type_size(*field.type_index) {
                fixed_off + sz
            } else {
                // fallback: use 8 bytes (typical pod fields)
                fixed_off + 8
            };
            Some(SchemaValue::new(fixed_off, limit, *field.type_index))
        }
    }

    // ------------------------------------------------------------------
    // Array access
    // ------------------------------------------------------------------

    pub fn array_len(&self, val: &SchemaValue) -> usize {
        let t = self.type_reader(val.type_idx);
        match t {
            SchemaTypeDefReader::Array { def } => match &def.layout {
                ArrayLayoutDefReader::Pod { stride } => {
                    (val.limit - val.offset) as usize / **stride as usize
                }
                ArrayLayoutDefReader::Indexed | ArrayLayoutDefReader::Fixed { .. } => {
                    let count_off = val.offset as usize;
                    (unsafe { mmap_read_pod::<u64>(&self.data[count_off..count_off + 8]) })
                        as usize
                }
            },
            _ => 0,
        }
    }

    pub fn array_get(&self, val: &SchemaValue, index: usize) -> Option<SchemaValue> {
        let t = self.type_reader(val.type_idx);
        let def = match t {
            SchemaTypeDefReader::Array { def } => def,
            _ => return None,
        };
        let count = self.array_len(val);
        if index >= count {
            return None;
        }

        let elem_type_idx = *def.elem_type;
        let (elem_off, elem_limit) = match &def.layout {
            ArrayLayoutDefReader::Pod { stride } => {
                let off = val.offset + index as u64 * **stride;
                (off, off + **stride)
            }
            ArrayLayoutDefReader::Indexed => {
                let base = val.offset as usize;
                let count_off = base;
                let ot_off = count_off + 8;
                let elem_rel: u64 =
                    unsafe { mmap_read_pod::<u64>(&self.data[ot_off + index * 8..][..8]) };
                let values_base = ot_off + count * 8;
                let off = values_base as u64 + elem_rel;
                let next_rel: u64 = if index + 1 < count {
                    unsafe {
                        mmap_read_pod::<u64>(&self.data[ot_off + (index + 1) * 8..][..8])
                    }
                } else {
                    unsafe {
                        mmap_read_pod::<u64>(&self.data[ot_off + count * 8..][..8])
                    }
                };
                let limit = values_base as u64 + next_rel;
                (off, limit)
            }
            ArrayLayoutDefReader::Fixed { stride } => {
                // Fixed: [count: u64, total_size: u64, elements...]
                let off = val.offset + 16 + index as u64 * **stride;
                (off, off + **stride)
            }
        };

        Some(SchemaValue::new(elem_off, elem_limit, elem_type_idx))
    }

    // ------------------------------------------------------------------
    // HashMap access
    // ------------------------------------------------------------------

    pub fn hashmap_len(&self, val: &SchemaValue) -> usize {
        let count_off = val.offset as usize;
        (unsafe { mmap_read_pod::<u64>(&self.data[count_off..count_off + 8]) }) as usize
    }

    pub fn hashmap_get_index(
        &self,
        val: &SchemaValue,
        index: usize,
    ) -> Option<(SchemaValue, SchemaValue)> {
        let t = self.type_reader(val.type_idx);
        let schema = match t {
            SchemaTypeDefReader::HashMap { def } => def,
            _ => return None,
        };
        let count = self.hashmap_len(val);
        if index >= count {
            return None;
        }

        let base = val.offset as usize;
        let key_type_idx = schema.key_type;
        let val_type_idx = schema.val_type;
        let key_is_pod = schema.key_is_pod != 0;
        let val_is_pod = schema.val_is_pod != 0;
        let key_align = self.type_align(key_type_idx) as usize;
        let val_align = self.type_align(val_type_idx) as usize;

        let (key_off, key_limit, val_off, val_limit) =
            match (key_is_pod, val_is_pod) {
                (true, true) => {
                    // podK_podV: [count: u64, [K; count], [V; count]]
                    let key_size = self.type_size(key_type_idx).unwrap_or(8) as usize;
                    let keys_start = align_up(base + 8, key_align);
                    let keys_end = keys_start + count * key_size;
                    let vals_start = align_up(keys_end, val_align);
                    let val_size = self.type_size(val_type_idx).unwrap_or(8) as usize;
                    let ko = keys_start + index * key_size;
                    let vo = vals_start + index * val_size;
                    (ko, ko + key_size, vo, vo + val_size)
                }
                (true, false) => {
                    // podK_inlinedV: [count: u64, keys: [K; count], vals: [offset: u64; count+1], val_data...]
                    let key_size = self.type_size(key_type_idx).unwrap_or(8) as usize;
                    let keys_start = align_up(base + 8, key_align);
                    let keys_end = keys_start + count * key_size;
                    let ot_start = align_up(keys_end, 4);
                    let val_data_start = ot_start + (count + 1) * 4;
                    let ko = keys_start + index * key_size;
                    let off_i = unsafe {
                        mmap_read_pod::<u32>(&self.data[ot_start + index * 4..][..4])
                            as usize
                    };
                    let off_next = unsafe {
                        mmap_read_pod::<u32>(&self.data[ot_start + (index + 1) * 4..][..4])
                            as usize
                    };
                    let vo = val_data_start + off_i;
                    let vl = val_data_start + off_next;
                    (ko, ko + key_size, vo, vl)
                }
                (false, true) => {
                    // inlinedK_podV: symmetric
                    let ot_start = align_up(base + 8, 4);
                    let key_data_start = ot_start + (count + 1) * 4;
                    // resolve key range
                    let off_i = unsafe {
                        mmap_read_pod::<u32>(&self.data[ot_start + index * 4..][..4])
                            as usize
                    };
                    let off_next = unsafe {
                        mmap_read_pod::<u32>(&self.data[ot_start + (index + 1) * 4..][..4])
                            as usize
                    };
                    let ko = key_data_start + off_i;
                    let kl = key_data_start + off_next;

                    let val_size = self.type_size(val_type_idx).unwrap_or(8) as usize;
                    // Layout: [count: u64, ot: [u32; count+1], keys..., vals...]
                    let key_data_end = key_data_start + off_next;
                    let vals_start = align_up(key_data_end, val_align);
                    let vo = vals_start + index * val_size;
                    (ko, kl, vo, vo + val_size)
                }
                (false, false) => {
                    // inlinedK_inlinedV: [count: u64, key_ot: [u32; count+1], val_ot: [u32; count+1], key_data..., val_data...]
                    let key_ot_start = align_up(base + 8, 4);
                    let val_ot_start = key_ot_start + (count + 1) * 4;
                    let key_data_start = val_ot_start + (count + 1) * 4;
                    let val_data_start = key_data_start + unsafe {
                        mmap_read_pod::<u32>(
                            &self.data[key_ot_start + count * 4..][..4],
                        ) as usize
                    };

                    // key range
                    let k_off_i = unsafe {
                        mmap_read_pod::<u32>(&self.data[key_ot_start + index * 4..][..4])
                            as usize
                    };
                    let k_off_next = unsafe {
                        mmap_read_pod::<u32>(
                            &self.data[key_ot_start + (index + 1) * 4..][..4],
                        ) as usize
                    };
                    let ko = key_data_start + k_off_i;
                    let kl = key_data_start + k_off_next;

                    // val range
                    let v_off_i = unsafe {
                        mmap_read_pod::<u32>(&self.data[val_ot_start + index * 4..][..4])
                            as usize
                    };
                    let v_off_next = unsafe {
                        mmap_read_pod::<u32>(
                            &self.data[val_ot_start + (index + 1) * 4..][..4],
                        ) as usize
                    };
                    let vo = val_data_start + v_off_i;
                    let vl = val_data_start + v_off_next;

                    (ko, kl, vo, vl)
                }
            };

        let key_val = SchemaValue::new(key_off as u64, key_limit as u64, key_type_idx);
        let val_val = SchemaValue::new(val_off as u64, val_limit as u64, val_type_idx);
        Some((key_val, val_val))
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// An evaluated result — either a buffer-backed value or a computed scalar.
#[derive(Clone, Copy, Debug)]
pub enum SchemaEvalResult {
    Buf(SchemaValue),
    U64(u64),
    I64(i64),
    F64(f64),
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

/// Render a `SchemaValue` as a JSON-like string with indentation.
pub fn format_schema_value(ctx: &SchemaDynContext<'_>, val: &SchemaValue) -> String {
    let mut out = String::new();
    write_schema_val(ctx, val, &mut out, 0);
    out
}

fn write_schema_val(
    ctx: &SchemaDynContext<'_>,
    val: &SchemaValue,
    out: &mut String,
    depth: usize,
) {
    let t = ctx.type_reader(val.type_idx);
    match t {
        SchemaTypeDefReader::U32 => out.push_str(&ctx.read_u32(val).to_string()),
        SchemaTypeDefReader::I32 => out.push_str(&ctx.read_i32(val).to_string()),
        SchemaTypeDefReader::U64 => out.push_str(&ctx.read_u64(val).to_string()),
        SchemaTypeDefReader::I64 => out.push_str(&ctx.read_i64(val).to_string()),
        SchemaTypeDefReader::F32 => out.push_str(&ctx.read_f32(val).to_string()),
        SchemaTypeDefReader::F64 => out.push_str(&ctx.read_f64(val).to_string()),
        SchemaTypeDefReader::Bool => {
            out.push_str(if ctx.read_bool(val) { "true" } else { "false" });
        }
        SchemaTypeDefReader::U16 => out.push_str(&ctx.read_u16(val).to_string()),
        SchemaTypeDefReader::I16 => out.push_str(&ctx.read_i16(val).to_string()),
        SchemaTypeDefReader::U8 => out.push_str(&ctx.read_u8(val).to_string()),
        SchemaTypeDefReader::I8 => out.push_str(&ctx.read_i8(val).to_string()),
        SchemaTypeDefReader::Usize => out.push_str(&ctx.read_usize(val).to_string()),
        SchemaTypeDefReader::Isize => out.push_str(&ctx.read_isize(val).to_string()),
        SchemaTypeDefReader::Str { .. } => {
            out.push('"');
            out.push_str(ctx.read_str(val));
            out.push('"');
        }
        SchemaTypeDefReader::PodStruct { .. } => {
            out.push_str("[pod]");
        }
        SchemaTypeDefReader::Struct { def } => {
            out.push_str("{\n");
            let fields: Vec<_> = (&def.fields).into_iter().collect();
            for (i, field) in fields.iter().enumerate() {
                indent(out, depth + 1);
                out.push('"');
                out.push_str(field.name);
                out.push_str("\": ");
                if let Some(fv) = ctx.field_get(val, field.name) {
                    write_schema_val(ctx, &fv, out, depth + 1);
                } else {
                    out.push_str("null");
                }
                if i + 1 < fields.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push('}');
        }
        SchemaTypeDefReader::Array { .. } => {
            out.push_str("[\n");
            let len = ctx.array_len(val);
            for i in 0..len {
                indent(out, depth + 1);
                if let Some(elem) = ctx.array_get(val, i) {
                    write_schema_val(ctx, &elem, out, depth + 1);
                } else {
                    out.push_str("null");
                }
                if i + 1 < len {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, depth);
            out.push(']');
        }
        // Option: show inner value if Some, "null" if None
        SchemaTypeDefReader::Option { def } => {
            let inner_type = def.inner_type;
            // Read the u8 flag at val.offset
            if ctx.data.get(val.offset as usize).copied() == Some(0) {
                out.push_str("null");
            } else {
                let inner_align = ctx.type_align(inner_type) as usize;
                let adjusted = align_up(val.offset as usize + 1, inner_align) as u64;
                let inner_val = SchemaValue::new(adjusted, val.limit, inner_type);
                write_schema_val(ctx, &inner_val, out, depth);
            }
        }
        SchemaTypeDefReader::ByRef { def } => {
            let inner_type = def.inner_type;
            let inner_val = SchemaValue::new(val.offset, val.limit, inner_type);
            write_schema_val(ctx, &inner_val, out, depth);
        }
        SchemaTypeDefReader::Enum { def: _ } => {
            if let Some((variant, _data_off)) = ctx.enum_variant(val) {
                out.push_str(variant.name);
                let fields: Vec<_> = variant.fields.iter().collect();
                if !fields.is_empty() {
                    out.push_str(" {\n");
                    for (i, field) in fields.iter().enumerate() {
                        indent(out, depth + 1);
                        out.push('"');
                        out.push_str(field.name);
                        out.push_str("\": ");
                        if let Some(fv) = ctx.field_get(val, field.name) {
                            write_schema_val(ctx, &fv, out, depth + 1);
                        } else {
                            out.push_str("null");
                        }
                        if i + 1 < fields.len() {
                            out.push(',');
                        }
                        out.push('\n');
                    }
                    indent(out, depth);
                    out.push('}');
                }
            } else {
                out.push_str("<invalid enum>");
            }
        }
        SchemaTypeDefReader::HashMap { .. } => {
            out.push_str("[\n");
            let len = ctx.hashmap_len(val);
            for i in 0..len {
                if let Some((k, v)) = ctx.hashmap_get_index(val, i) {
                    indent(out, depth + 1);
                    out.push_str("[\n");
                    write_schema_val(ctx, &k, out, depth + 2);
                    out.push_str(",\n");
                    write_schema_val(ctx, &v, out, depth + 2);
                    out.push('\n');
                    indent(out, depth + 1);
                    out.push(']');
                    if i + 1 < len {
                        out.push(',');
                    }
                    out.push('\n');
                }
            }
            indent(out, depth);
            out.push(']');
        }
    }
}

// ---------------------------------------------------------------------------
// Simple path-based query (no filter/aggregation)
// ---------------------------------------------------------------------------

/// Navigate a `SchemaValue` by a simple path string like `/field1[0]/field2`.
///
/// Supports:
/// - `/field_name` — struct/enum field access
/// - `array_field[0]` — indexed array access
///
/// Returns `Err` if the path is malformed or a segment can't be resolved.
/// Returns a single `SchemaEvalResult::Buf` (not a Vec — use `query_from_schema`
/// for multiple results).
pub fn navigate_path<'a>(
    ctx: &SchemaDynContext<'a>,
    root: &SchemaValue,
    path: &str,
) -> Result<SchemaEvalResult, String> {
    let path = path.strip_prefix('/').unwrap_or(path);
    if path.is_empty() {
        return Ok(SchemaEvalResult::Buf(*root));
    }
    let mut current = *root;
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if let Some((field_name, idx)) = segment.split_once('[') {
            // array access: field_name[idx]
            let idx_str = idx.trim_end_matches(']');
            let index: usize = idx_str
                .parse()
                .map_err(|_| format!("invalid array index `{}`", idx_str))?;
            // First resolve the array field
            let arr = ctx
                .field_get(&current, field_name)
                .ok_or_else(|| format!("no such field `{}`", field_name))?;
            // Then index into it
            let elem = ctx
                .array_get(&arr, index)
                .ok_or_else(|| format!("index {} out of bounds", index))?;
            current = elem;
        } else {
            let fv = ctx
                .field_get(&current, segment)
                .ok_or_else(|| format!("no such field `{}`", segment))?;
            current = fv;
        }
    }
    Ok(SchemaEvalResult::Buf(current))
}

/// Run a simple path query and return results.
///
/// For simple path navigation use `navigate_path`. For full query
/// language (filters, aggregations) use `mmapview` with typed access.
pub fn query_from_schema<'a>(
    q: &str,
    ctx: &SchemaDynContext<'a>,
    root: &SchemaValue,
) -> Result<Vec<SchemaEvalResult>, String> {
    // Currently supports simple paths; future: full query language
    navigate_path(ctx, root, q).map(|r| vec![r])
}

#[inline(always)]
fn align_up(offset: usize, align: usize) -> usize {
    (offset + align - 1) & !(align - 1)
}
