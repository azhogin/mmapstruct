use crate::traits::{MmapPod, MmapRead, MmapWrite, FileRange, ReaderOrd, ReaderOwned};
use std::collections::HashMap;
use std::marker::PhantomData;
use super::common::{read_count, sort_pairs, align_up, read_offset, align_write, reserve_offset_table, write_offset_table, binary_search_reader, iter_offset_ranges};
use core::mem::size_of;

/// Layout: `[count: u64, key_offsets: [u32; count + 1], keys_data..., [V; count]]`
pub struct HashMapReader<'a, K, V> {
    data: &'a [u8],
    count: usize,
    key_offsets_start: usize,
    keys_area_start: usize,
    vals: &'a [V],
    _phantom: PhantomData<K>,
}

impl<'a, K: MmapRead + 'a, V: MmapPod> HashMapReader<'a, K, V> {
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        let count = read_count(data, &range);

        let key_offsets_start = range.start + size_of::<u64>();
        let key_align = core::mem::align_of::<K>();
        let keys_area_start = align_up(key_offsets_start + (count + 1) * 4, key_align);

        let total_keys = if count > 0 {
            read_offset(data, key_offsets_start, count) as usize
        } else {
            0
        };

        #[cfg(feature = "strict-validation")]
        {
            let val_align = core::mem::align_of::<V>();
            let keys_end = keys_area_start + total_keys;
            let vals_start = align_up(keys_end, val_align);
            let vals_end = vals_start + count * core::mem::size_of::<V>();
            assert!(vals_end <= data.len(),
                "inlinedK_podV: total range {}..{} exceeds data len {}", keys_area_start, vals_end, data.len());
        }

        let val_align = core::mem::align_of::<V>();
        let keys_end = keys_area_start + total_keys;
        let vals_start = align_up(keys_end, val_align);
        let vals_bytes = &data[vals_start..vals_start + count * core::mem::size_of::<V>()];
        let vals: &[V] = crate::bytemuck::cast_slice(vals_bytes);

        Self { data, count, key_offsets_start, keys_area_start, vals, _phantom: PhantomData }
    }

    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn get(&self, key: &K) -> Option<&'a V>
    where
        K: ReaderOrd,
    {
        let idx = binary_search_reader(self.data, self.key_offsets_start, self.keys_area_start, key, self.count)?;
        Some(&self.vals[idx])
    }

    pub fn contains_key(&self, key: &K) -> bool
    where
        K: ReaderOrd,
    {
        self.get(key).is_some()
    }

    pub fn keys(&'a self) -> impl Iterator<Item = K::Reader<'a>> {
        iter_offset_ranges(self.data, self.key_offsets_start, self.keys_area_start, self.count)
            .map(move |(start, end)| K::read(self.data, start..end))
    }

    pub fn values(&self) -> &'a [V] { self.vals }

    pub fn iter(&'a self) -> impl Iterator<Item = (K::Reader<'a>, &'a V)> {
        self.keys().zip(self.vals.iter())
    }
}

impl<'a, K: MmapRead + ReaderOwned + Ord + std::hash::Hash + Eq + 'a, V: MmapPod + std::hash::Hash + Eq>
    From<HashMapReader<'a, K, V>> for std::collections::HashMap<K, V>
{
    fn from(r: HashMapReader<'a, K, V>) -> Self {
        r.iter().map(|(k, v)| (ReaderOwned::into_owned(k), *v)).collect()
    }
}

pub fn write_hash_map<'data, K: MmapWrite + Ord + 'data, V: MmapPod + 'data>(
    map: &'data HashMap<K, V>,
    ctx: &mut crate::WriteContext<'data>,
) {
    let pairs = sort_pairs(map);
    let count = pairs.len() as u64;
    ctx.write_u64_le(count);

    let key_offsets_start = reserve_offset_table(ctx, count as usize);

    let key_align = core::mem::align_of::<K>();
    let keys_area_start = {
        align_write(ctx, key_align);
        ctx.current_offset()
    };
    let mut key_offsets: Vec<u32> = Vec::with_capacity(count as usize);
    for (k, _) in &pairs {
        align_write(ctx, key_align);
        key_offsets.push((ctx.current_offset() - keys_area_start) as u32);
        k.write(ctx);
    }
    key_offsets.push((ctx.current_offset() - keys_area_start) as u32);

    let val_align = core::mem::align_of::<V>();
    align_write(ctx, val_align);

    for (_, v) in &pairs {
        ctx.write_pod(*v);
    }

    write_offset_table(ctx, key_offsets_start, &key_offsets);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WriteContext;

    fn write_and_read<'a, K: MmapRead + MmapWrite + Ord + ReaderOrd + 'static, V: MmapPod + 'static>(
        map: &HashMap<K, V>,
        buf: &'a mut Vec<u8>,
    ) -> HashMapReader<'a, K, V>
    {
        let mut ctx = WriteContext::new();
        write_hash_map(map, &mut ctx);
        *buf = ctx.finish();
        let range = FileRange { start: 0, end: buf.len() };
        HashMapReader::new(buf, range)
    }

    #[test]
    fn roundtrip_string_keys_u32_vals() {
        let map = HashMap::from([
            ("gamma".to_string(), 30u32),
            ("alpha".to_string(), 10u32),
            ("beta".to_string(), 20u32),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert_eq!(reader.len(), 3);
        assert_eq!(*reader.get(&"alpha".to_string()).unwrap(), 10);
        assert_eq!(*reader.get(&"beta".to_string()).unwrap(), 20);
        assert_eq!(*reader.get(&"gamma".to_string()).unwrap(), 30);
        assert!(reader.get(&"delta".to_string()).is_none());
    }

    #[test]
    fn empty_map() {
        let map: HashMap<String, u32> = HashMap::new();
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert!(reader.is_empty());
        assert_eq!(reader.len(), 0);
    }
}
