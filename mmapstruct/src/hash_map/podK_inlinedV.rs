use super::common::{
    align_up, align_write, iter_offset_ranges, read_count, read_offset, reserve_offset_table,
    sort_pairs, write_offset_table,
};
use crate::traits::{FileRange, MmapPod, MmapRead, MmapWrite, ReaderOwned};
use core::mem::size_of;
use std::collections::HashMap;
use std::marker::PhantomData;

/// Layout: `[count: u64, [K; count], val_offsets: [u32; count + 1], vals_data...]`
///
/// After the inline values section there may be postponed data appended
/// (e.g. byref fields inside inlined structs). Whether postponed data
/// is present depends on where the enclosing scope finalizes it —
/// the topmost element or a scope boundary flushes postponed data into
/// the buffer after all inline content.
pub struct HashMapReader<'a, K, V> {
    data: &'a [u8],
    count: usize,
    keys: &'a [K],
    val_offsets_start: usize,
    vals_data_start: usize,
    _phantom: PhantomData<(K, V)>,
}

impl<'a, K: MmapPod, V: MmapRead + 'a> HashMapReader<'a, K, V> {
    /// Create a new reader for the podK-inlinedV HashMap layout.
    ///
    /// Range covers the entire HashMap, including any postponed data
    /// appended after the inline values section.
    ///
    /// # Panics
    ///
    /// Without `strict-validation`, panics on out-of-bounds slice access.
    /// With `strict-validation`, explicit bounds and overflow assertions
    /// provide clearer errors.
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        let count = read_count(data, &range);

        #[cfg(feature = "strict-validation")]
        let keys_size = count
            .checked_mul(core::mem::size_of::<K>())
            .expect("podK_inlinedV: overflow computing keys_size");
        #[cfg(not(feature = "strict-validation"))]
        let keys_size = count * core::mem::size_of::<K>();

        let key_align = core::mem::align_of::<K>();
        let keys_start = align_up(range.start + size_of::<u64>(), key_align);

        #[cfg(feature = "strict-validation")]
        {
            let keys_end = keys_start + keys_size;
            assert!(
                keys_end <= data.len(),
                "podK_inlinedV: keys range {}..{} exceeds data len {}",
                keys_start,
                keys_end,
                data.len()
            );
        }

        let keys_bytes = &data[keys_start..keys_start + keys_size];
        let keys: &[K] = crate::bytemuck::cast_slice(keys_bytes);

        let ot_align = core::mem::align_of::<u32>();
        let val_offsets_start = align_up(keys_start + keys_size, ot_align);
        let raw_vals_data_start = val_offsets_start + (count + 1) * 4;
        let val_align = core::mem::align_of::<V>();
        let vals_data_start = align_up(raw_vals_data_start, val_align);

        #[cfg(feature = "strict-validation")]
        {
            let total_vals = if count > 0 {
                read_offset(data, val_offsets_start, count) as usize
            } else {
                0
            };
            let vals_end = vals_data_start + total_vals;
            assert!(
                vals_end <= data.len(),
                "podK_inlinedV: vals range {}..{} exceeds data len {}",
                vals_data_start,
                vals_end,
                data.len()
            );
        }

        Self {
            data,
            count,
            keys,
            val_offsets_start,
            vals_data_start,
            _phantom: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn get(&self, key: &K) -> Option<V::Reader<'a>>
    where
        K: Ord,
    {
        let idx = self.keys.binary_search(key).ok()?;
        let start_off = read_offset(self.data, self.val_offsets_start, idx);
        let end_off = read_offset(self.data, self.val_offsets_start, idx + 1);
        #[cfg(feature = "strict-validation")]
        {
            assert!(
                start_off <= end_off,
                "podK_inlinedV::get({}): corrupted offsets: {} > {}",
                idx,
                start_off,
                end_off
            );
            let end = self.vals_data_start + end_off as usize;
            assert!(
                end <= self.data.len(),
                "podK_inlinedV::get({}): value range up to {} exceeds data len {}",
                idx,
                end,
                self.data.len()
            );
        }
        let start = self.vals_data_start + start_off as usize;
        let end = self.vals_data_start + end_off as usize;
        Some(V::read(self.data, start..end))
    }

    pub fn contains_key(&self, key: &K) -> bool
    where
        K: Ord,
    {
        self.keys.binary_search(key).is_ok()
    }

    pub fn keys(&self) -> &'a [K] {
        self.keys
    }

    pub fn iter(&'a self) -> impl Iterator<Item = (&'a K, V::Reader<'a>)> {
        self.keys.iter().zip(
            iter_offset_ranges(
                self.data,
                self.val_offsets_start,
                self.vals_data_start,
                self.count,
            )
            .map(move |(start, end)| V::read(self.data, start..end)),
        )
    }
}

impl<
        'a,
        K: MmapPod + Ord + std::hash::Hash + Eq,
        V: MmapRead + ReaderOwned + std::hash::Hash + Eq + 'a,
    > From<HashMapReader<'a, K, V>> for std::collections::HashMap<K, V>
{
    fn from(r: HashMapReader<'a, K, V>) -> Self {
        r.iter()
            .map(|(k, v)| (*k, ReaderOwned::into_owned(v)))
            .collect()
    }
}

pub fn write_hash_map<'data, K: MmapPod + Ord + 'data, V: MmapWrite + 'data>(
    map: &'data HashMap<K, V>,
    ctx: &mut crate::WriteContext<'data>,
) {
    let pairs = sort_pairs(map);
    let count = pairs.len() as u64;
    ctx.write_u64_le(count);

    let key_align = core::mem::align_of::<K>();
    align_write(ctx, key_align);

    for (k, _) in &pairs {
        ctx.write_pod(*k);
    }

    let ot_align = core::mem::align_of::<u32>();
    align_write(ctx, ot_align);
    let val_offsets_start = reserve_offset_table(ctx, count as usize);

    let val_align = core::mem::align_of::<V>();
    let vals_data_start = {
        align_write(ctx, val_align);
        ctx.current_offset()
    };
    let mut val_offsets: Vec<u32> = Vec::with_capacity(count as usize);
    for (_, v) in &pairs {
        align_write(ctx, val_align);
        val_offsets.push((ctx.current_offset() - vals_data_start) as u32);
        v.write(ctx);
    }
    val_offsets.push((ctx.current_offset() - vals_data_start) as u32);

    write_offset_table(ctx, val_offsets_start, &val_offsets);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WriteContext;

    fn write_and_read<'a, K: MmapPod + Ord + 'static, V: MmapRead + MmapWrite + 'static>(
        map: &HashMap<K, V>,
        buf: &'a mut Vec<u8>,
    ) -> HashMapReader<'a, K, V> {
        let mut ctx = WriteContext::new();
        write_hash_map(map, &mut ctx);
        *buf = ctx.finish();
        let range = FileRange {
            start: 0,
            end: buf.len(),
        };
        HashMapReader::new(buf, range)
    }

    #[test]
    fn roundtrip_podK_string() {
        let map = HashMap::from([
            (10u32, "alpha".to_string()),
            (20u32, "beta".to_string()),
            (5u32, "gamma".to_string()),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert_eq!(reader.len(), 3);
        assert_eq!(reader.get(&5).unwrap(), "gamma");
        assert_eq!(reader.get(&10).unwrap(), "alpha");
        assert_eq!(reader.get(&20).unwrap(), "beta");
        assert!(reader.get(&99).is_none());
    }

    #[test]
    fn empty_map() {
        let map: HashMap<u32, String> = HashMap::new();
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert!(reader.is_empty());
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn keys_in_order() {
        let map = HashMap::from([
            (3u32, "c".to_string()),
            (1u32, "a".to_string()),
            (2u32, "b".to_string()),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        let collected = reader.keys();
        assert_eq!(collected, vec![1u32, 2, 3]);
    }
}
