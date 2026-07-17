use crate::schema::TypeDefReader;
use crate::value::SchemaValue;

impl<'a> super::RuntimeSchemaCtx<'a> {
    /// For an enum root value, determine which variant is active.
    /// Returns `(variant_type_idx, data_offset_from_root)`.
    /// Handles both data-carrying enums (4-byte discriminator) and
    /// pod enums (1/2/4/8-byte discriminator inferred from value range).
    pub fn enum_variant(&self, root: &SchemaValue) -> Option<(u32, u64)> {
        let type_idx = self.unwrap_type(root.type_idx);
        let t = self.type_reader(type_idx);
        if let TypeDefReader::Enum {
            name: _,
            variants,
            variant_type_indices,
        } = t
        {
            let val_size = root.mmap_end - root.mmap_begin;
            let disc_byte_size = if val_size >= 4 { 4 } else { val_size };
            let disc = match disc_byte_size {
                1 => self.read_u8(root) as u64,
                2 => self.read_u16(root) as u64,
                4 => self.read_u32(root) as u64,
                8 => self.read_u64(root) as u64,
                _ => return None,
            } as u32;
            let variant_idx = disc;
            if (variant_idx as usize) < variants.len() {
                let variant_type_idx = *variant_type_indices.get(variant_idx as usize)?;
                let data_offset = {
                    let fa = self.type_align(root.type_idx);
                    let aligned = (disc_byte_size as u64 + fa - 1) & !(fa - 1);
                    aligned
                };
                return Some((variant_type_idx, data_offset));
            }
        }
        None
    }
}
