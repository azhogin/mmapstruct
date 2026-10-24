use crate::helpers::TypeDefReaderExt;
use crate::schema::{FieldDef, TypeDefReader};
use crate::value::SchemaValue;
use mmapstruct::indexed_array::IndexedArrayReader;
use mmapstruct::traits::{mmap_read_pod, ByRefRange};

/// Return value of [`RuntimeSchemaCtx::field_get_with_info`].
pub struct FieldInfo {
    pub val: SchemaValue,
    pub is_byref: bool,
}

impl<'a> super::RuntimeSchemaCtx<'a> {
    /// Navigate from `root` into a named field.
    ///
    /// For struct/variant fields: look up `FieldDef` by name, then compute the
    /// `SchemaValue` from its `offset_in_fixed` and `type_index`.
    ///
    /// For enum types: first resolve which variant is active, then search the
    /// variant's fields.
    ///
    /// Returns `None` if the field is not found or the type cannot contain fields.
    pub fn field_get(&self, root: &SchemaValue, name: &str) -> Option<SchemaValue> {
        self.field_index_and_value(root, name).map(|(_, v)| v)
    }

    /// Like [`field_get`](Self::field_get) but additionally reports whether the
    /// field's declared type is `ByRef<T>` (after unwrapping `root`'s type for
    /// enum variants).
    pub fn field_get_with_info(&self, root: &SchemaValue, name: &str) -> Option<FieldInfo> {
        let (idx, val) = self.field_index_and_value(root, name)?;
        let is_byref = self.is_field_byref(root, idx);
        Some(FieldInfo { val, is_byref })
    }

    /// Check whether the field at `field_idx` in `parent` is declared as `ByRef<T>`.
    /// For enum parents the active variant's field list is consulted.
    pub fn is_field_byref(&self, parent: &SchemaValue, field_idx: usize) -> bool {
        let type_idx = self.unwrap_type(parent.type_idx);
        let t = self.type_reader(type_idx);
        match t {
            TypeDefReader::Struct { fields, .. } => fields
                .get(field_idx)
                .is_some_and(|f| self.type_reader(*f.type_index).is_byref_kind()),
            TypeDefReader::Enum { .. } => {
                let Some((variant, _)) = self.enum_variant(parent) else {
                    return false;
                };
                let vt = self.type_reader(variant);
                if let TypeDefReader::Struct { fields, .. } = vt {
                    fields
                        .get(field_idx)
                        .is_some_and(|f| self.type_reader(*f.type_index).is_byref_kind())
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    /// Like `field_get`, but also returns the index of the matched field within
    /// the struct's (or active enum variant's) field list.
    ///
    /// The index is suitable for `PathShift::Field(idx)`.  For enum types the
    /// index is relative to the active variant's fields, matching the convention
    /// used by `Scanner::child_at`.
    pub fn field_index_and_value(
        &self,
        root: &SchemaValue,
        name: &str,
    ) -> Option<(usize, SchemaValue)> {
        let type_idx = self.unwrap_type(root.type_idx);
        let t = self.type_reader(type_idx);
        match t {
            TypeDefReader::Struct {
                name: _,
                fields,
                fixed_section_size: _,
                header_size: _,
                inline_dyn_count: _,
            } => self.resolve_field_value(root, fields, name),
            TypeDefReader::Enum {
                name: _,
                variants: _,
                variant_type_indices: _,
                disc_size: _,
            } => {
                let (variant, data_off) = self.enum_variant(root)?;
                let variant_type = self.type_reader(variant);
                if let TypeDefReader::Struct {
                    name: _,
                    fields,
                    fixed_section_size: _,
                    header_size: _,
                    inline_dyn_count: _,
                } = variant_type
                {
                    let shifted = SchemaValue {
                        mmap_begin: root.mmap_begin + data_off,
                        mmap_end: root.mmap_end,
                        type_idx: variant,
                    };
                    self.resolve_field_value(&shifted, fields, name)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Given a struct's field definitions, find the named field and compute its
    /// `SchemaValue` from `offset_in_fixed` and `type_index`.
    /// Returns `(field_index, SchemaValue)` — the index can be used as
    /// `PathShift::Field(idx)` for path-based navigation.
    pub(crate) fn resolve_field_value(
        &self,
        root: &SchemaValue,
        fields: &IndexedArrayReader<'a, FieldDef>,
        name: &str,
    ) -> Option<(usize, SchemaValue)> {
        let field_count = fields.len();
        for i in 0..field_count {
            let f = fields.get(i).unwrap();
            if f.name == name {
                let raw_type = self.type_reader(*f.type_index);

                // Option<T> — read inline-dynamic data, then unwrap the
                // Option byte (0=None, 1=Some) and skip alignment padding.
                if raw_type.is_option_kind() {
                    if let Some(dyn_idx) = f.inline_dyn_index {
                        let range = self.inline_dynamic_range(root, *dyn_idx)?;
                        if range.mmap_begin >= range.mmap_end {
                            return Some((
                                i,
                                SchemaValue {
                                    mmap_begin: range.mmap_begin,
                                    mmap_end: range.mmap_begin,
                                    type_idx: *f.type_index,
                                },
                            ));
                        }
                        let disc = self.data[range.mmap_begin as usize];
                        if disc == 0 {
                            return Some((
                                i,
                                SchemaValue {
                                    mmap_begin: range.mmap_begin,
                                    mmap_end: range.mmap_begin,
                                    type_idx: *f.type_index,
                                },
                            ));
                        }
                        // Some: skip past the u8 discriminant + alignment
                        let inner_type = self.unwrap_type(*f.type_index);
                        let inner_align = self.type_align(inner_type);
                        let inner_start =
                            (range.mmap_begin + 1 + inner_align - 1) & !(inner_align - 1);
                        return Some((
                            i,
                            SchemaValue {
                                mmap_begin: inner_start,
                                mmap_end: range.mmap_end,
                                type_idx: inner_type,
                            },
                        ));
                    }
                }

                // ByRef<T>: field stores a ByRefRange (begin, end) in the fixed
                // section; the actual data lives in the variable-data area.
                if raw_type.is_byref_kind() {
                    let off = *f.offset_in_fixed as usize;
                    let byref: ByRefRange = unsafe {
                        mmap_read_pod::<ByRefRange>(&self.data[root.mmap_begin as usize + off..])
                    };
                    let inner = match raw_type {
                        TypeDefReader::ByRef { inner_type } => **inner_type,
                        _ => unreachable!(),
                    };
                    return Some((
                        i,
                        SchemaValue {
                            mmap_begin: root.mmap_begin + byref.begin as u64,
                            mmap_end: root.mmap_begin + byref.end as u64,
                            type_idx: inner,
                        },
                    ));
                }

                // Inline-dynamic (string, array, vec, etc.): look up offset table
                if let Some(dyn_idx) = f.inline_dyn_index {
                    return Some((i, self.inline_dynamic_value(root, *dyn_idx, *f.type_index)?));
                }

                let field_type = self.unwrap_type(*f.type_index);
                let dyntype = self.type_reader(field_type);
                if dyntype.is_fixed_size() {
                    if let Some(sz) = self.type_size(*f.type_index) {
                        return Some((
                            i,
                            SchemaValue {
                                mmap_begin: root.mmap_begin + *f.offset_in_fixed,
                                mmap_end: root.mmap_begin + *f.offset_in_fixed + sz,
                                type_idx: field_type,
                            },
                        ));
                    }
                }
                // PodStruct fallback? Not expected.
                return None;
            }
        }
        None
    }

    /// Compute the raw data range for an inline-dynamic field (without
    /// interpreting Option encoding).
    fn inline_dynamic_range(&self, root: &SchemaValue, dyn_idx: u32) -> Option<SchemaValue> {
        let (fixed_section_size, header_size, inline_dyn_count) =
            match self.type_reader(root.type_idx) {
                TypeDefReader::Struct {
                    fixed_section_size,
                    header_size,
                    inline_dyn_count,
                    ..
                } => (**fixed_section_size, **header_size, **inline_dyn_count),
                _ => return None,
            };

        let ot_start = root.mmap_begin as usize + fixed_section_size as usize;
        let vs = root.mmap_begin as usize + header_size as usize;
        let dy = dyn_idx as usize;

        if ot_start + (dy + 1) * 8 > self.data.len() {
            return None;
        }
        let off0 = (unsafe {
            mmap_read_pod::<u64>(&self.data[ot_start + dy * 8..ot_start + (dy + 1) * 8])
        }) as usize;
        let off1 = if dy + 1 < inline_dyn_count as usize {
            (unsafe {
                mmap_read_pod::<u64>(&self.data[ot_start + (dy + 1) * 8..ot_start + (dy + 2) * 8])
            }) as usize
        } else {
            (unsafe {
                mmap_read_pod::<u64>(
                    &self.data[ot_start + inline_dyn_count as usize * 8
                        ..ot_start + (inline_dyn_count as usize + 1) * 8],
                )
            }) as usize
        };

        Some(SchemaValue {
            mmap_begin: (vs + off0) as u64,
            mmap_end: (vs + off1) as u64,
            type_idx: 0,
        })
    }

    /// Resolve an inline-dynamic field's value.
    ///
    /// The inline-dynamic encoding uses an offset table (u64 per entry) indexed
    /// by `dyn_idx`.  Entry N stores the relative offset of datum N from the
    /// variable-data area (`vs`).  The range for datum N is `[vs + off_N, vs + off_{N+1})`,
    /// with a sentinel entry at `inline_dyn_count` marking the end.
    fn inline_dynamic_value(
        &self,
        root: &SchemaValue,
        dyn_idx: u32,
        field_type: u32,
    ) -> Option<SchemaValue> {
        let (fixed_section_size, header_size, inline_dyn_count) =
            match self.type_reader(root.type_idx) {
                TypeDefReader::Struct {
                    fixed_section_size,
                    header_size,
                    inline_dyn_count,
                    ..
                } => (**fixed_section_size, **header_size, **inline_dyn_count),
                _ => return None,
            };

        let ot_start = root.mmap_begin as usize + fixed_section_size as usize;
        let vs = root.mmap_begin as usize + header_size as usize;
        let dy = dyn_idx as usize;

        if ot_start + (dy + 1) * 8 > self.data.len() {
            return None;
        }
        let off0 = (unsafe {
            mmap_read_pod::<u64>(&self.data[ot_start + dy * 8..ot_start + (dy + 1) * 8])
        }) as usize;
        let off1 = if dy + 1 < inline_dyn_count as usize {
            (unsafe {
                mmap_read_pod::<u64>(&self.data[ot_start + (dy + 1) * 8..ot_start + (dy + 2) * 8])
            }) as usize
        } else {
            (unsafe {
                mmap_read_pod::<u64>(
                    &self.data[ot_start + inline_dyn_count as usize * 8
                        ..ot_start + (inline_dyn_count as usize + 1) * 8],
                )
            }) as usize
        };

        Some(SchemaValue {
            mmap_begin: (vs + off0) as u64,
            mmap_end: (vs + off1) as u64,
            type_idx: field_type,
        })
    }
}
