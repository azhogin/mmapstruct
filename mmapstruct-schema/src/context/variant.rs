use crate::schema::TypeDefReader;
use crate::value::SchemaValue;

impl<'a> super::RuntimeSchemaCtx<'a> {
    /// For an enum root value, determine which variant is active.
    /// Returns `(variant_type_idx, data_offset_from_root)`.
    pub fn enum_variant(&self, root: &SchemaValue) -> Option<(u32, u64)> {
        let type_idx = self.unwrap_type(root.type_idx);
        let t = self.type_reader(type_idx);
        if let TypeDefReader::Enum {
            name: _,
            variants,
            variant_type_indices,
        } = t
        {
            let disc = self.read_u32(root);
            let variant_idx = disc;
            if (variant_idx as usize) < variants.len() {
                let variant_type_idx = *variant_type_indices.get(variant_idx as usize)?;
                let data_offset = {
                    let fa = self.type_align(root.type_idx);
                    let aligned = (4u64 + fa - 1) & !(fa - 1);
                    aligned
                };
                return Some((variant_type_idx, data_offset));
            }
        }
        None
    }
}
