use crate::traits::{MmapRead, FileRange};
use crate::WriteContext;
use std::marker::PhantomData;

pub struct FixedArrayReader<'a, T> {
    data: &'a [u8],
    base: usize,
    count: usize,
    stride: usize,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead> FixedArrayReader<'a, T> {
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        if range.len() < 16 {
            return Self { data, base: range.start, count: 0, stride: 0, _phantom: PhantomData };
        }
        let count = u64::from_le_bytes(data[range.start..range.start + 8].try_into().unwrap()) as usize;
        let total_size = u64::from_le_bytes(data[range.start + 8..range.start + 16].try_into().unwrap()) as usize;
        let base = range.start + 16;
        let stride = if count > 0 { total_size / count } else { 0 };
        #[cfg(feature = "strict-validation")]
        if count > 0 {
            assert_eq!(total_size % count, 0,
                "FixedArrayReader: total_size {} not divisible by count {}", total_size, count);
            assert!(count * stride <= total_size,
                "FixedArrayReader: count {} * stride {} > total_size {}",
                count, stride, total_size);
        }
        Self { data, base, count, stride, _phantom: PhantomData }
    }

    pub fn len(&self) -> usize { self.count }
    pub fn is_empty(&self) -> bool { self.count == 0 }

    pub fn get(&self, index: usize) -> Option<T::Reader<'a>> {
        if index >= self.count { return None; }
        let start = self.base + index * self.stride;
        let end = start + self.stride;
        #[cfg(feature = "strict-validation")]
        {
            assert!(end <= self.data.len(),
                "FixedArrayReader::get({}): end {} > data len {}",
                index, end, self.data.len());
        }
        Some(T::read(self.data, start..end))
    }

    pub fn iter(&self) -> FixedArrayIter<'a, T> {
        FixedArrayIter { data: self.data, base: self.base, stride: self.stride, pos: 0, count: self.count, _phantom: PhantomData }
    }
}

pub struct FixedArrayIter<'a, T> {
    data: &'a [u8],
    base: usize,
    stride: usize,
    pos: usize,
    count: usize,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> Iterator for FixedArrayIter<'a, T> {
    type Item = T::Reader<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.count { return None; }
        let start = self.base + self.pos * self.stride;
        let end = start + self.stride;
        self.pos += 1;
        Some(T::read(self.data, start..end))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.count.saturating_sub(self.pos);
        (rem, Some(rem))
    }
}

impl<'a, T: MmapRead + 'a> IntoIterator for &FixedArrayReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = FixedArrayIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

impl<'a, T: MmapRead + 'a> IntoIterator for FixedArrayReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = FixedArrayIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter { self.iter() }
}

impl<'a, T: MmapRead + 'a> From<FixedArrayReader<'a, T>> for Vec<T>
where
    <T as MmapRead>::Reader<'a>: Into<T>,
{
    fn from(r: FixedArrayReader<'a, T>) -> Self {
        r.iter().map(|e| e.into()).collect()
    }
}

pub fn write_fixed_array<'data, T: crate::traits::MmapWrite>(items: &'data [T], ctx: &mut WriteContext<'data>) {
    let before = ctx.current_offset();
    ctx.write_u64_le(items.len() as u64);
    ctx.write_padding(8); // reserve space for total data size
    let data_start = ctx.current_offset();
    for elem in items {
        elem.write(ctx);
    }
    let total_size = ctx.current_offset() - data_start;
    ctx.write_u64_le_at(total_size as u64, before + 8);
}
