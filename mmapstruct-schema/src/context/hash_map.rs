use crate::helpers::align_up;
use crate::value::{HashMapDef, SchemaValue};
use crate::schema::TypeDefReader;
use mmapstruct::traits::mmap_read_pod;

impl<'a> super::RuntimeSchemaCtx<'a> {
    /// Number of entries in a HashMap value.
    pub fn hashmap_len(&self, val: &SchemaValue) -> usize {
        let off = val.offset as usize;
        if off + 8 > self.data.len() {
            return 0;
        }
        unsafe { mmap_read_pod::<u64>(&self.data[off..off + 8]) as usize }
    }

    /// Get the i-th entry of a HashMap value. Returns `(key, value)`.
    pub fn hashmap_get_index(&self, val: &SchemaValue, index: usize) -> Option<(SchemaValue, SchemaValue)> {
        let (def, key_type, val_type) = self.hash_map_def(val.type_idx)?;
        let count = self.hashmap_len(val);
        if index >= count {
            return None;
        }
        let base = val.offset as usize;
        let ot_byte_size = (count + 1) * 4;
        let key_align = self.type_align(key_type) as usize;
        let val_align = self.type_align(val_type) as usize;

        let (rel_key_off, rel_key_limit, rel_val_off, rel_val_limit) = match (def.key_is_pod, def.val_is_pod) {
            (true, true) => {
                let key_size = self.type_size(key_type)? as usize;
                let keys_start = align_up(8, key_align);
                let vals_start = align_up(keys_start + count * key_size, val_align);
                let val_size = self.type_size(val_type)? as usize;
                let ko = keys_start + index * key_size;
                let ke = keys_start + (index + 1) * key_size;
                let vo = vals_start + index * val_size;
                let ve = vals_start + (index + 1) * val_size;
                (ko, ke, vo, ve)
            }
            (true, false) => {
                let key_size = self.type_size(key_type)? as usize;
                let keys_start = align_up(8, key_align);
                let keys_end = keys_start + count * key_size;
                let val_ot_start = align_up(keys_end, 4);
                let raw_vals_data_start = val_ot_start + ot_byte_size;
                let vals_data_start = align_up(raw_vals_data_start, val_align);
                let ko = keys_start + index * key_size;
                let ke = keys_start + (index + 1) * key_size;
                let vs_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + val_ot_start + index * 4..]) } as u64;
                let ve_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + val_ot_start + (index + 1) * 4..]) } as u64;
                return Some((
                    SchemaValue::new(base as u64 + ko as u64, base as u64 + ke as u64, key_type),
                    SchemaValue::new(base as u64 + vals_data_start as u64 + vs_rel, base as u64 + vals_data_start as u64 + ve_rel, val_type),
                ));
            }
            (false, true) => {
                let key_ot_start = 8;
                let keys_data_start = align_up(key_ot_start + ot_byte_size, key_align);
                let ks_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + key_ot_start + index * 4..]) } as u64;
                let ke_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + key_ot_start + (index + 1) * 4..]) } as u64;
                let ko = (keys_data_start as u64 + ks_rel) as usize;
                let ke = (keys_data_start as u64 + ke_rel) as usize;
                let keys_total = unsafe { mmap_read_pod::<u32>(&self.data[base + key_ot_start + count * 4..]) } as u64;
                let aligned_keys = align_up(keys_data_start + keys_total as usize, val_align);
                let val_size = self.type_size(val_type)? as usize;
                let vo = aligned_keys + index * val_size;
                let ve = aligned_keys + (index + 1) * val_size;
                return Some((
                    SchemaValue::new(base as u64 + ko as u64, base as u64 + ke as u64, key_type),
                    SchemaValue::new(base as u64 + vo as u64, base as u64 + ve as u64, val_type),
                ));
            }
            (false, false) => {
                let key_ot_start = 8;
                let val_ot_start = key_ot_start + ot_byte_size;
                let keys_data_start = align_up(val_ot_start + ot_byte_size, key_align);
                let ks_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + key_ot_start + index * 4..]) } as u64;
                let ke_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + key_ot_start + (index + 1) * 4..]) } as u64;
                let ko = (keys_data_start as u64 + ks_rel) as usize;
                let ke = (keys_data_start as u64 + ke_rel) as usize;
                let vs_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + val_ot_start + index * 4..]) } as u64;
                let ve_rel = unsafe { mmap_read_pod::<u32>(&self.data[base + val_ot_start + (index + 1) * 4..]) } as u64;
                let keys_total = unsafe { mmap_read_pod::<u32>(&self.data[base + key_ot_start + count * 4..]) } as u64;
                let aligned_keys = align_up(keys_data_start + keys_total as usize, val_align);
                let vals_data_start = aligned_keys;
                let vo = vals_data_start as u64 + vs_rel;
                let ve = vals_data_start as u64 + ve_rel;
                return Some((
                    SchemaValue::new(base as u64 + ko as u64, base as u64 + ke as u64, key_type),
                    SchemaValue::new(base as u64 + vo as u64, base as u64 + ve as u64, val_type),
                ));
            }
        };

        Some((
            SchemaValue::new(base as u64 + rel_key_off as u64, base as u64 + rel_key_limit as u64, key_type),
            SchemaValue::new(base as u64 + rel_val_off as u64, base as u64 + rel_val_limit as u64, val_type),
        ))
    }

    /// Get the HashMap's internal structure for iteration.
    pub fn hash_map_def(&self, type_idx: u32) -> Option<(HashMapDef, u32, u32)> {
        let t = self.type_reader(type_idx);
        if let TypeDefReader::HashMap {
            key_type,
            val_type,
            key_is_pod,
            val_is_pod,
        } = t
        {
            Some((
                HashMapDef {
                    key_is_pod: **key_is_pod != 0,
                    val_is_pod: **val_is_pod != 0,
                },
                **key_type,
                **val_type,
            ))
        } else {
            None
        }
    }
}
