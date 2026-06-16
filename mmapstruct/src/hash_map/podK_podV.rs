use crate::traits::{FileRange, MmapPod};
use std::collections::HashMap;
use std::marker::PhantomData;
use super::common::{read_count, sort_pairs, align_up, align_write};
use core::mem::size_of;

/// Layout: `[count: u64, [K; count], [V; count]]`
///
/// Both K and V are pod types, so no postponed data section follows —
/// the layout is entirely self-contained within the range.
pub struct HashMapReader<'a, K, V> {
    count: usize,
    keys: &'a [K],
    vals: &'a [V],
    _phantom: PhantomData<(K, V)>,
}

impl<'a, K: MmapPod, V: MmapPod> HashMapReader<'a, K, V> {
    /// Create a new reader for the podK-podV HashMap layout.
    ///
    /// Layout: `[count: u64, [K; count], [V; count]]` with alignment padding
    /// between sections as needed. No postponed data section — both K and V
    /// are pod types and produce no deferred writes.
    ///
    /// # Panics
    ///
    /// Without `strict-validation`, panics on out-of-bounds slice access or
    /// arithmetic overflow (debug mode).
    /// With `strict-validation`, explicit bounds and overflow assertions
    /// provide clearer errors.
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        let count = read_count(data, &range);
        #[cfg(feature = "strict-validation")]
        let keys_size = count.checked_mul(core::mem::size_of::<K>())
            .expect("podK_podV: overflow computing keys_size");
        #[cfg(not(feature = "strict-validation"))]
        let keys_size = count * core::mem::size_of::<K>();
        #[cfg(feature = "strict-validation")]
        let vals_size = count.checked_mul(core::mem::size_of::<V>())
            .expect("podK_podV: overflow computing vals_size");
        #[cfg(not(feature = "strict-validation"))]
        let vals_size = count * core::mem::size_of::<V>();
        let key_align = core::mem::align_of::<K>();
        let keys_start = align_up(range.start + size_of::<u64>(), key_align);
        #[cfg(feature = "strict-validation")]
        {
            let keys_end = keys_start + keys_size;
            assert!(keys_end <= data.len(),
                "podK_podV: keys range {}..{} exceeds data len {}", keys_start, keys_end, data.len());
        }
        let keys_bytes = &data[keys_start..keys_start + keys_size];
        let keys: &[K] = crate::bytemuck::cast_slice(keys_bytes);
        let val_align = core::mem::align_of::<V>();
        let vals_start = align_up(keys_start + keys_size, val_align);
        #[cfg(feature = "strict-validation")]
        {
            let vals_end = vals_start + vals_size;
            assert!(vals_end <= data.len(),
                "podK_podV: vals range {}..{} exceeds data len {}", vals_start, vals_end, data.len());
        }
        let vals_bytes = &data[vals_start..vals_start + vals_size];
        let vals: &[V] = crate::bytemuck::cast_slice(vals_bytes);
        Self { count, keys, vals, _phantom: PhantomData }
    }

    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn get(&self, key: &K) -> Option<&'a V>
    where K: Ord,
    {
        self.keys.binary_search(key).ok().map(|i| &self.vals[i])
    }

    pub fn contains_key(&self, key: &K) -> bool
    where K: Ord,
    {
        self.keys.binary_search(key).is_ok()
    }

    pub fn keys(&self) -> &'a [K] { self.keys }
    pub fn values(&self) -> &'a [V] { self.vals }

    pub fn iter(&self) -> impl Iterator<Item = (&'a K, &'a V)> {
        self.keys.iter().zip(self.vals.iter())
    }
}

impl<'a, K: MmapPod + Ord + std::hash::Hash + Eq, V: MmapPod> From<HashMapReader<'a, K, V>> for std::collections::HashMap<K, V> {
    fn from(r: HashMapReader<'a, K, V>) -> Self {
        r.iter().map(|(k, v)| (*k, *v)).collect()
    }
}

pub fn write_hash_map<'data, K: MmapPod + Ord + 'data, V: MmapPod + 'data>(
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

    let val_align = core::mem::align_of::<V>();
    align_write(ctx, val_align);
    for (_, v) in &pairs {
        ctx.write_pod(*v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::FileRange;
    use crate::WriteContext;

    fn write_and_read<'a, K: MmapPod + Ord, V: MmapPod>(
        map: &HashMap<K, V>,
        buf: &'a mut Vec<u8>,
    ) -> HashMapReader<'a, K, V> {
        let mut ctx = WriteContext::new();
        write_hash_map(map, &mut ctx);
        *buf = ctx.finish();
        let range = FileRange { start: 0, end: buf.len() };
        HashMapReader::new(buf, range)
    }

    #[test]
    fn roundtrip_podK_podV() {
        let map = HashMap::from([
            (10u32, 100u64),
            (20u32, 200u64),
            (5u32, 50u64),
        ]);

        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert_eq!(reader.len(), 3);
        assert_eq!(*reader.get(&5).unwrap(), 50);
        assert_eq!(*reader.get(&10).unwrap(), 100);
        assert_eq!(*reader.get(&20).unwrap(), 200);
        assert!(reader.get(&99).is_none());
    }

    #[test]
    fn empty_map() {
        let map: HashMap<u32, u64> = HashMap::new();
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert!(reader.is_empty());
        assert_eq!(reader.len(), 0);
    }

    #[test]
    fn roundtrip_u8_keys() {
        let map = HashMap::from([
            (10u8, 100u64),
            (20u8, 200u64),
            (5u8, 50u64),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert_eq!(reader.len(), 3);
        assert_eq!(*reader.get(&5).unwrap(), 50);
        assert_eq!(*reader.get(&10).unwrap(), 100);
        assert_eq!(*reader.get(&20).unwrap(), 200);
        assert!(reader.get(&99).is_none());
    }

    #[test]
    fn roundtrip_u64_keys_u8_vals() {
        let map = HashMap::from([
            (100u64, 1u8),
            (200u64, 2u8),
            (50u64, 3u8),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        assert_eq!(reader.len(), 3);
        assert_eq!(*reader.get(&50).unwrap(), 3);
        assert_eq!(*reader.get(&100).unwrap(), 1);
        assert_eq!(*reader.get(&200).unwrap(), 2);
        assert!(reader.get(&99).is_none());
    }

    #[test]
    fn keys_in_order() {
        let map = HashMap::from([
            (3u32, 30u64),
            (1u32, 10u64),
            (2u32, 20u64),
        ]);
        let mut buf = Vec::new();
        let reader = write_and_read(&map, &mut buf);
        let collected = reader.keys();
        assert_eq!(collected, vec![1u32, 2, 3]);
    }
}
