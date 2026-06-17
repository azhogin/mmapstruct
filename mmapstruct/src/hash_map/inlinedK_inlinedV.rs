use super::common::{
    align_up, align_write, binary_search_reader, iter_offset_ranges, read_count, read_offset,
    reserve_offset_table, sort_pairs, write_offset_table,
};
use crate::traits::{FileRange, MmapRead, MmapWrite, ReaderOrd, ReaderOwned};
use std::collections::HashMap;
use std::marker::PhantomData;

/// Layout:
/// `[count: u64, key_offsets: [u32; count + 1], val_offsets: [u32; count + 1],
///  keys_data..., align_padding, vals_data...]`
pub struct HashMapReader<'a, K, V> {
    data: &'a [u8],
    count: usize,
    key_offsets_start: usize,
    keys_area_start: usize,
    val_offsets_start: usize,
    vals_area_start: usize,
    _phantom: PhantomData<(K, V)>,
}

impl<'a, K: MmapRead + 'a, V: MmapRead + 'a> HashMapReader<'a, K, V> {
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        let count = read_count(data, &range);

        let key_offsets_start = range.start + std::mem::size_of::<u64>();
        let ot_byte_size = (count + 1) * 4;
        let val_offsets_start = key_offsets_start + ot_byte_size;

        let key_align = core::mem::align_of::<K>();
        let raw_keys = val_offsets_start + ot_byte_size;
        let keys_area_start = align_up(raw_keys, key_align);

        let total_keys = if count > 0 {
            read_offset(data, key_offsets_start, count) as usize
        } else {
            0
        };

        let val_align = core::mem::align_of::<V>();
        let keys_end = keys_area_start + total_keys;
        let vals_area_start = align_up(keys_end, val_align);

        #[cfg(feature = "strict-validation")]
        {
            let total_vals = if count > 0 {
                read_offset(data, val_offsets_start, count) as usize
            } else {
                0
            };
            assert!(
                vals_area_start + total_vals <= data.len(),
                "inlinedK_inlinedV: total range exceeds data len"
            );
        }

        Self {
            data,
            count,
            key_offsets_start,
            keys_area_start,
            val_offsets_start,
            vals_area_start,
            _phantom: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn read_stored_val(&self, index: usize) -> V::Reader<'a> {
        let start =
            self.vals_area_start + read_offset(self.data, self.val_offsets_start, index) as usize;
        let end = self.vals_area_start
            + read_offset(self.data, self.val_offsets_start, index + 1) as usize;
        V::read(self.data, start..end)
    }

    pub fn get(&self, key: &K) -> Option<V::Reader<'a>>
    where
        K: ReaderOrd,
    {
        let idx = binary_search_reader(
            self.data,
            self.key_offsets_start,
            self.keys_area_start,
            key,
            self.count,
        )?;
        Some(self.read_stored_val(idx))
    }

    pub fn contains_key(&self, key: &K) -> bool
    where
        K: ReaderOrd,
    {
        self.get(key).is_some()
    }

    pub fn keys(&'a self) -> impl Iterator<Item = K::Reader<'a>> {
        iter_offset_ranges(
            self.data,
            self.key_offsets_start,
            self.keys_area_start,
            self.count,
        )
        .map(move |(start, end)| K::read(self.data, start..end))
    }

    pub fn values(&'a self) -> impl Iterator<Item = V::Reader<'a>> {
        iter_offset_ranges(
            self.data,
            self.val_offsets_start,
            self.vals_area_start,
            self.count,
        )
        .map(move |(start, end)| V::read(self.data, start..end))
    }

    pub fn iter(&'a self) -> impl Iterator<Item = (K::Reader<'a>, V::Reader<'a>)> {
        let keys = iter_offset_ranges(
            self.data,
            self.key_offsets_start,
            self.keys_area_start,
            self.count,
        )
        .map(move |(start, end)| K::read(self.data, start..end));
        let vals = iter_offset_ranges(
            self.data,
            self.val_offsets_start,
            self.vals_area_start,
            self.count,
        )
        .map(move |(start, end)| V::read(self.data, start..end));
        keys.zip(vals)
    }
}

impl<
        'a,
        K: MmapRead + ReaderOwned + Ord + std::hash::Hash + Eq + 'a,
        V: MmapRead + ReaderOwned + std::hash::Hash + Eq + 'a,
    > From<HashMapReader<'a, K, V>> for std::collections::HashMap<K, V>
{
    fn from(r: HashMapReader<'a, K, V>) -> Self {
        r.iter()
            .map(|(k, v)| (ReaderOwned::into_owned(k), ReaderOwned::into_owned(v)))
            .collect()
    }
}

pub fn write_hash_map<'data, K: MmapWrite + Ord + 'data, V: MmapWrite + 'data>(
    map: &'data HashMap<K, V>,
    ctx: &mut crate::WriteContext<'data>,
) {
    let pairs = sort_pairs(map);
    let count = pairs.len() as u64;
    ctx.write_u64_le(count);

    let key_offsets_start = reserve_offset_table(ctx, count as usize);
    let val_offsets_start = reserve_offset_table(ctx, count as usize);

    let key_align = core::mem::align_of::<K>();
    let keys_data_start = {
        align_write(ctx, key_align);
        ctx.current_offset()
    };

    let mut key_offsets: Vec<u32> = Vec::with_capacity(count as usize);
    for (k, _) in &pairs {
        align_write(ctx, key_align);
        key_offsets.push((ctx.current_offset() - keys_data_start) as u32);
        k.write(ctx);
    }
    key_offsets.push((ctx.current_offset() - keys_data_start) as u32);

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

    write_offset_table(ctx, key_offsets_start, &key_offsets);
    write_offset_table(ctx, val_offsets_start, &val_offsets);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WriteContext;

    fn write_and_read<'a, K, V>(
        map: &HashMap<K, V>,
        buf: &'a mut Vec<u8>,
    ) -> HashMapReader<'a, K, V>
    where
        K: MmapRead + MmapWrite + Ord + ReaderOrd + 'static,
        V: MmapRead + MmapWrite + 'static,
    {
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
    fn roundtrip_string_keys_string_vals() {
        let map = HashMap::from([
            ("gamma".to_string(), "GAMMA".to_string()),
            ("alpha".to_string(), "ALPHA".to_string()),
            ("beta".to_string(), "BETA".to_string()),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert_eq!(reader.len(), 3);
        assert_eq!(reader.get(&"alpha".to_string()).unwrap(), "ALPHA");
        assert_eq!(reader.get(&"beta".to_string()).unwrap(), "BETA");
        assert_eq!(reader.get(&"gamma".to_string()).unwrap(), "GAMMA");
        assert!(reader.get(&"delta".to_string()).is_none());
    }

    #[test]
    fn empty_map() {
        let map: HashMap<String, String> = HashMap::new();
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert!(reader.is_empty());
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn iterate_keys_values() {
        let map = HashMap::from([
            ("b".to_string(), "2".to_string()),
            ("a".to_string(), "1".to_string()),
            ("c".to_string(), "3".to_string()),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);

        let keys: Vec<String> = reader.keys().map(|s| s.to_string()).collect();
        assert_eq!(keys, vec!["a", "b", "c"]);

        let vals: Vec<String> = reader.values().map(|s| s.to_string()).collect();
        assert_eq!(vals, vec!["1", "2", "3"]);

        let pairs: Vec<(String, String)> = reader
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("a".to_string(), "1".to_string()),
                ("b".to_string(), "2".to_string()),
                ("c".to_string(), "3".to_string()),
            ]
        );
    }
}
