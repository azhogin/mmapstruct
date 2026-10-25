use crate::helpers::TypeDefReaderExt;
use crate::schema::{ArrayLayoutDefReader, TypeDefReader};
use crate::value::SchemaValue;
use mmapstruct::traits::mmap_read_pod;

impl<'a> super::RuntimeSchemaCtx<'a> {
    /// Get the number of elements in an array value.
    pub fn array_len(&self, val: &SchemaValue) -> u64 {
        let t = self.type_reader(val.type_idx);
        if let TypeDefReader::Array {
            elem_type: _,
            layout,
        } = t
        {
            let arr_range = val.mmap_begin as usize..val.mmap_end as usize;
            match layout {
                ArrayLayoutDefReader::Pod { stride } => {
                    let sz = **stride as usize;
                    if sz == 0 {
                        return 0;
                    }
                    if arr_range.len() < 8 {
                        return 0;
                    }
                    // Pod arrays now carry an 8-byte element-count prefix
                    // (flagged reserved blocks keep the count in the same
                    // word, masked below).

                    (unsafe {
                        mmap_read_pod::<u64>(&self.data[arr_range.start..arr_range.start + 8])
                    } as u64)
                        & !mmapstruct::traits::BLOCK_FLAG_RESERVED
                }
                ArrayLayoutDefReader::FixedPod { count, .. } => **count,
                ArrayLayoutDefReader::Indexed => {
                    if arr_range.len() < 8 {
                        return 0;
                    }

                    (unsafe {
                        mmap_read_pod::<u64>(&self.data[arr_range.start..arr_range.start + 8])
                    } as u64)
                        & !mmapstruct::traits::BLOCK_FLAG_RESERVED
                }
                ArrayLayoutDefReader::Fixed { .. } => {
                    if arr_range.len() < 8 {
                        return 0;
                    }

                    (unsafe {
                        mmap_read_pod::<u64>(&self.data[arr_range.start..arr_range.start + 8])
                    } as u64)
                }
                ArrayLayoutDefReader::List => {
                    if arr_range.len() < 24 {
                        return 0;
                    }

                    (unsafe {
                        mmap_read_pod::<u64>(&self.data[arr_range.start + 16..arr_range.start + 24])
                    } as u64)
                }
            }
        } else {
            0
        }
    }

    /// Get the i-th element of an array value.
    ///
    /// Out-of-range indices return `None` (consistent with the static
    /// `SchemaCtx::array_get`).
    pub fn array_get(&self, val: &SchemaValue, index: u64) -> Option<SchemaValue> {
        let t = self.type_reader(val.type_idx);
        if let TypeDefReader::Array { elem_type, layout } = t {
            // Bounds-check against the actual element count first.
            if index >= self.array_len(val) {
                return None;
            }
            let arr_range = val.mmap_begin as usize..val.mmap_end as usize;
            let (elem_off, elem_limit) = match layout {
                ArrayLayoutDefReader::Pod { stride } => {
                    let sz = **stride as usize;
                    // Skip the 8-byte element-count prefix; reserved (flagged)
                    // blocks add a reserve word after the count.
                    let raw = unsafe {
                        mmap_read_pod::<u64>(&self.data[arr_range.start..arr_range.start + 8])
                    };
                    let flagged = raw & mmapstruct::traits::BLOCK_FLAG_RESERVED != 0;
                    let off = arr_range.start + if flagged { 16 } else { 8 } + index as usize * sz;
                    let limit = off + sz;
                    (off as u64, limit as u64)
                }
                ArrayLayoutDefReader::FixedPod { stride, .. } => {
                    let sz = **stride as usize;
                    let off = arr_range.start + index as usize * sz;
                    let limit = off + sz;
                    (off as u64, limit as u64)
                }
                ArrayLayoutDefReader::Indexed => {
                    let count_off = arr_range.start;
                    let raw = unsafe { mmap_read_pod::<u64>(&self.data[count_off..count_off + 8]) };
                    let flagged = raw & mmapstruct::traits::BLOCK_FLAG_RESERVED != 0;
                    let count = (raw & !mmapstruct::traits::BLOCK_FLAG_RESERVED) as usize;
                    if index as usize >= count {
                        return None;
                    }
                    if flagged {
                        // Reserved block: block-relative offsets, spare slots
                        // and a data gap; implicit end = span end - gap.
                        let resv = unsafe {
                            mmap_read_pod::<u64>(&self.data[count_off + 8..count_off + 16])
                        };
                        let reserved_bytes = (resv >> 32) as usize;
                        let off_rel = (unsafe {
                            mmap_read_pod::<u64>(
                                &self.data[count_off + 16 + index as usize * 8
                                    ..count_off + 16 + (index as usize + 1) * 8],
                            )
                        }) as usize;
                        let elem_off = count_off + off_rel;
                        let limit_rel = if index as usize + 1 < count {
                            (unsafe {
                                mmap_read_pod::<u64>(
                                    &self.data[count_off + 16 + (index as usize + 1) * 8
                                        ..count_off + 16 + (index as usize + 2) * 8],
                                )
                            }) as usize
                        } else {
                            arr_range.len() - reserved_bytes
                        };
                        let elem_limit = count_off + limit_rel;
                        (elem_off as u64, elem_limit as u64)
                    } else {
                        let off_table = count_off + 8;
                        let values_start = off_table + count * 8;
                        let elem_off_rel = unsafe {
                            mmap_read_pod::<u64>(
                                &self.data[off_table + index as usize * 8
                                    ..off_table + (index as usize + 1) * 8],
                            )
                        } as usize;
                        let elem_next_rel = if index as usize + 1 < count {
                            (unsafe {
                                mmap_read_pod::<u64>(
                                    &self.data[off_table + (index as usize + 1) * 8
                                        ..off_table + (index as usize + 2) * 8],
                                )
                            }) as usize
                        } else {
                            arr_range.len() - (values_start - count_off)
                        };
                        let elem_off = values_start + elem_off_rel;
                        let elem_limit = values_start + elem_next_rel;
                        (elem_off as u64, elem_limit as u64)
                    }
                }
                ArrayLayoutDefReader::Fixed { stride } => {
                    let sz = **stride as usize;
                    let off = arr_range.start + 16 + index as usize * sz;
                    let limit = off + sz;
                    (off as u64, limit as u64)
                }
                ArrayLayoutDefReader::List => {
                    // Walk the node chain from head (O(index)); node offsets
                    // are relative to the list block start.
                    let block = arr_range.start;
                    let mut cur = unsafe { mmap_read_pod::<u64>(&self.data[block..block + 8]) };
                    for _ in 0..index {
                        if cur == 0 {
                            return None;
                        }
                        cur = unsafe {
                            mmap_read_pod::<u64>(
                                &self.data[block + cur as usize..block + cur as usize + 8],
                            )
                        };
                    }
                    if cur == 0 {
                        return None;
                    }
                    let node = block + cur as usize;
                    let len =
                        (unsafe { mmap_read_pod::<u32>(&self.data[node + 8..node + 12]) }) as u64;
                    (node as u64 + 16, node as u64 + 16 + len)
                }
            };
            // Option<T> elements: unwrap the present byte + alignment
            // (None -> empty range), mirroring field_get's Option handling
            // in mixed_struct.rs. Applies to every layout: Option is always
            // a dynamic element, but the unwrap is cheap and uniform.
            let elem_type_idx = **elem_type;
            if self.type_reader(elem_type_idx).is_option_kind() {
                if elem_off >= elem_limit
                    || elem_off as usize >= self.data.len()
                    || self.data[elem_off as usize] == 0
                {
                    return Some(SchemaValue {
                        mmap_begin: elem_off,
                        mmap_end: elem_off,
                        type_idx: elem_type_idx,
                    });
                }
                let inner_type = self.unwrap_type(elem_type_idx);
                let inner_align = self.type_align(inner_type);
                let inner_start = (elem_off + 1 + inner_align - 1) & !(inner_align - 1);
                if inner_start > elem_limit {
                    // Corrupt storage: treat as None rather than a bogus range.
                    return Some(SchemaValue {
                        mmap_begin: elem_off,
                        mmap_end: elem_off,
                        type_idx: elem_type_idx,
                    });
                }
                return Some(SchemaValue {
                    mmap_begin: inner_start,
                    mmap_end: elem_limit,
                    type_idx: inner_type,
                });
            }
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
