use crate::schema::TypeDefReader;
use crate::value::SchemaValue;

impl<'a> super::RuntimeSchemaCtx<'a> {
    /// For an enum root value, determine which variant is active.
    /// Returns `(variant_type_idx, data_offset_from_root)`.
    ///
    /// The discriminant size comes from the schema's `disc_size` (the derive
    /// always emits it: 4 for data-carrying enums, `repr(Int)` size for pod
    /// enums), with the old value-size heuristic as a fallback for schemata
    /// written before `disc_size` was populated.
    ///
    /// Variant data starts after the discriminant at an **absolute** 8-byte
    /// boundary, mirroring the writer's `write_u32_le(disc)` + `ctx.align_to(8)`
    /// (see `enum_handler::handle_enum`).
    pub fn enum_variant(&self, root: &SchemaValue) -> Option<(u32, u64)> {
        let type_idx = self.unwrap_type(root.type_idx);
        let t = self.type_reader(type_idx);
        if let TypeDefReader::Enum {
            name: _,
            variants,
            variant_type_indices,
            disc_size,
        } = t
        {
            let begin = root.mmap_begin as usize;
            let end = root.mmap_end as usize;
            let ds = **disc_size;
            let disc_byte_size = match ds {
                1 | 2 | 4 | 8 => ds as usize,
                _ => {
                    let val_size = end.saturating_sub(begin);
                    if val_size >= 4 {
                        4
                    } else {
                        val_size
                    }
                }
            };
            if disc_byte_size == 0 || begin + disc_byte_size > end {
                return None;
            }
            let disc = match disc_byte_size {
                1 => self.read_u8(root) as u64,
                2 => self.read_u16(root) as u64,
                4 => self.read_u32(root) as u64,
                8 => self.read_u64(root),
                _ => return None,
            } as u32;
            let variant_idx = disc;
            if (variant_idx as usize) < variants.len() {
                let variant_type_idx = *variant_type_indices.get(variant_idx as usize)?;
                let data_off = ((begin + disc_byte_size + 7) & !7usize) as u64 - root.mmap_begin;
                return Some((variant_type_idx, data_off));
            }
        }
        None
    }
}