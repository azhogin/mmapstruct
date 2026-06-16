use crate::traits::{MmapRead, ByRefRange, FileRange, mmap_read_pod};
use crate::WriteContext;
use std::marker::PhantomData;
use std::ops::Range;

/// Reader for a `Vec<T>` with `#[mmap(byref)]` — elements stored as
/// `ByRefRange` placeholders in inline values, actual data in postponed section.
///
/// Layout in inline values:
/// ```text
/// [count: u64, total_size: u64, ranges: [ByRefRange; count]]
/// ```
/// Each `ByRefRange` is resolved relative to the struct's base offset.
pub struct ByRefArrayReader<'a, T> {
    data: &'a [u8],
    /// Struct base offset — used to resolve `ByRefRange` into absolute positions.
    struct_base: usize,
    count: usize,
    /// Raw bytes of the `[ByRefRange; count]` array.
    ranges: &'a [u8],
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> ByRefArrayReader<'a, T> {
    pub fn new(data: &'a [u8], range: FileRange, struct_base: usize) -> Self {
        if range.len() < 16 {
            return Self { data, struct_base, count: 0, ranges: &[], _phantom: PhantomData };
        }
        let count = u64::from_le_bytes(data[range.start..range.start + 8].try_into().unwrap()) as usize;
        #[allow(unused_variables)]
        let total_size = u64::from_le_bytes(data[range.start + 8..range.start + 16].try_into().unwrap()) as usize;
        #[cfg(feature = "strict-validation")]
        {
            assert_eq!(total_size, count * 8,
                "ByRefArrayReader: total_size {} != count {} * 8", total_size, count);
            assert!(range.start + 16 + count * 8 <= data.len(),
                "ByRefArrayReader: ranges end {} > data len {}", range.start + 16 + count * 8, data.len());
        }
        let ranges = &data[range.start + 16..range.start + 16 + count * 8];
        Self { data, struct_base, count, ranges, _phantom: PhantomData }
    }

    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn get(&self, index: usize) -> Option<T::Reader<'a>> {
        if index >= self.count { return None; }
        let pos = index * 8;
        let br: ByRefRange = unsafe { mmap_read_pod(&self.ranges[pos..pos + 8]) };
        let range: Range<usize> = self.struct_base + br.begin as usize..self.struct_base + br.end as usize;
        #[cfg(feature = "strict-validation")]
        {
            assert!(range.end <= self.data.len(),
                "ByRefArrayReader::get({}): end {} > data len {}", index, range.end, self.data.len());
        }
        Some(T::read(self.data, range))
    }

    pub fn iter(&self) -> ByRefArrayIter<'a, T> {
        ByRefArrayIter {
            data: self.data,
            struct_base: self.struct_base,
            ranges: self.ranges,
            count: self.count,
            pos: 0,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: MmapRead + 'a> IntoIterator for &ByRefArrayReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = ByRefArrayIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

impl<'a, T: MmapRead + 'a> IntoIterator for ByRefArrayReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = ByRefArrayIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

impl<'a, T: MmapRead + 'a> From<ByRefArrayReader<'a, T>> for Vec<T>
where
    <T as MmapRead>::Reader<'a>: Into<T>,
{
    fn from(r: ByRefArrayReader<'a, T>) -> Self {
        r.iter().map(|e| e.into()).collect()
    }
}

pub struct ByRefArrayIter<'a, T: MmapRead> {
    data: &'a [u8],
    struct_base: usize,
    ranges: &'a [u8],
    count: usize,
    pos: usize,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> Iterator for ByRefArrayIter<'a, T> {
    type Item = T::Reader<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.count { return None; }
        let br: ByRefRange = unsafe { mmap_read_pod(&self.ranges[self.pos * 8..self.pos * 8 + 8]) };
        let range: Range<usize> = self.struct_base + br.begin as usize..self.struct_base + br.end as usize;
        self.pos += 1;
        Some(T::read(self.data, range))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.count.saturating_sub(self.pos);
        (rem, Some(rem))
    }
}

/// Write a `Vec<T>` with `#[mmap(byref)]` — count + total_size + [ByRefRange; N] in inline values,
/// actual data in postponed section.
pub fn write_byref_array<'data, T: crate::traits::MmapWrite>(
    items: &'data [T],
    ctx: &mut WriteContext<'data>,
    base: usize,
    dedup: bool,
) {
    ctx.write_u64_le(items.len() as u64);
    let total_size_offset = ctx.current_offset();
    ctx.write_u64_le(0); // placeholder for total_size
    for item in items {
        ctx.write_byref(item, base, dedup);
    }
    // total_size = count * 8 (each ByRefRange is 8 bytes)
    ctx.write_u64_le_at((items.len() * 8) as u64, total_size_offset);
}
