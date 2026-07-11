use crate::schema::{ArrayLayoutDefReader, TypeDefReader};
use crate::value::SchemaValue;
use mmapstruct::traits::mmap_read_pod;

impl<'a> super::RuntimeSchemaCtx<'a> {
    /// Get the number of elements in an array value.
    pub fn array_len(&self, val: &SchemaValue) -> u64 {
        let t = self.type_reader(val.type_idx);
        if let TypeDefReader::Array { elem_type: _, layout } = t {
            let arr_range = val.mmap_begin as usize..val.mmap_end as usize;
            match layout {
                ArrayLayoutDefReader::Pod { stride } => {
                    let sz = **stride as usize;
                    if sz == 0 {
                        return 0;
                    }
                    (arr_range.len() / sz) as u64
                }
                ArrayLayoutDefReader::Indexed => {
                    if arr_range.len() < 8 {
                        return 0;
                    }
                    let count = unsafe { mmap_read_pod::<u64>(&self.data[arr_range.start..arr_range.start + 8]) } as u64;
                    count
                }
                ArrayLayoutDefReader::Fixed { stride } => {
                    let sz = **stride as usize;
                    if sz == 0 {
                        return 0;
                    }
                    (arr_range.len() / sz) as u64
                }
            }
        } else {
            0
        }
    }

    /// Get the i-th element of an array value.
    pub fn array_get(&self, val: &SchemaValue, index: u64) -> Option<SchemaValue> {
        let t = self.type_reader(val.type_idx);
        if let TypeDefReader::Array { elem_type, layout } = t {
            let arr_range = val.mmap_begin as usize..val.mmap_end as usize;
            let (elem_off, elem_limit) = match layout {
                ArrayLayoutDefReader::Pod { stride } => {
                    let sz = **stride as usize;
                    let off = arr_range.start + index as usize * sz;
                    let limit = off + sz;
                    (off as u64, limit as u64)
                }
                ArrayLayoutDefReader::Indexed => {
                    let count_off = arr_range.start;
                    let count = unsafe { mmap_read_pod::<u64>(&self.data[count_off..count_off + 8]) } as usize;
                    if index as usize >= count {
                        return None;
                    }
                    let off_table = count_off + 8;
                    let values_start = off_table + count * 8;
                    let elem_off_rel = unsafe { mmap_read_pod::<u64>(&self.data[off_table + index as usize * 8..off_table + (index as usize + 1) * 8]) } as usize;
                    let elem_next_rel = if index as usize + 1 < count {
                        (unsafe { mmap_read_pod::<u64>(&self.data[off_table + (index as usize + 1) * 8..off_table + (index as usize + 2) * 8]) }) as usize
                    } else {
                        arr_range.len() - (values_start - count_off)
                    };
                    let elem_off = values_start + elem_off_rel;
                    let elem_limit = values_start + elem_next_rel;
                    (elem_off as u64, elem_limit as u64)
                }
                ArrayLayoutDefReader::Fixed { stride } => {
                    let sz = **stride as usize;
                    let off = arr_range.start + index as usize * sz;
                    let limit = off + sz;
                    (off as u64, limit as u64)
                }
            };
            Some(SchemaValue {
                mmap_begin: elem_off,
                mmap_end: elem_limit,
                type_idx: **elem_type,
            })
        } else {
            None
        }
    }
}
