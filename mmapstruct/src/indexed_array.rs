use crate::traits::{FileRange, MmapRead};
use std::marker::PhantomData;

#[derive(Clone)]
pub struct IndexedArrayReader<'a, T> {
    data: &'a [u8],
    #[allow(dead_code)]
    range: FileRange,
    count: usize,
    offset_bytes: &'a [u8],
    total_values_size: usize,
    values_offset: usize,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> IndexedArrayReader<'a, T> {
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        let count = if range.len() < 8 {
            0
        } else {
            u64::from_le_bytes(data[range.start..range.start + 8].try_into().unwrap()) as usize
        };
        #[cfg(feature = "strict-validation")]
        {
            let max_count = range.len() / 8;
            assert!(
                count <= max_count,
                "IndexedArrayReader: count {} exceeds max possible {} for range len {}",
                count,
                max_count,
                range.len()
            );
        }
        let offsets_offset = range.start + 8;
        let values_offset = offsets_offset + count * 8;
        let offset_bytes = &data[offsets_offset..offsets_offset + count * 8];
        let total_values_size = range.end - values_offset;

        Self {
            data,
            range,
            count,
            offset_bytes,
            total_values_size,
            values_offset,
            _phantom: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn offset(&self, i: usize) -> usize {
        if i < self.count {
            let pos = i * 8;
            u64::from_le_bytes(self.offset_bytes[pos..pos + 8].try_into().unwrap()) as usize
        } else {
            self.total_values_size
        }
    }

    pub fn get(&self, index: usize) -> Option<T::Reader<'a>> {
        if index >= self.count {
            return None;
        }
        let start = self.values_offset + self.offset(index);
        let end = self.values_offset + self.offset(index + 1);
        #[cfg(feature = "strict-validation")]
        {
            assert!(
                start <= end,
                "IndexedArrayReader::get({}): start {} > end {}",
                index,
                start,
                end
            );
            assert!(
                end <= self.data.len(),
                "IndexedArrayReader::get({}): end {} > data len {}",
                index,
                end,
                self.data.len()
            );
        }
        Some(T::read(self.data, start..end))
    }

    pub fn iter(&self) -> IndexedArrayIter<'a, T> {
        IndexedArrayIter {
            data: self.data,
            values_start: self.values_offset,
            offset_bytes: self.offset_bytes,
            total_values_size: self.total_values_size,
            pos: 0,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: MmapRead + 'a> IntoIterator for &IndexedArrayReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = IndexedArrayIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: MmapRead + 'a> IntoIterator for IndexedArrayReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = IndexedArrayIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub struct IndexedArrayIter<'a, T: MmapRead> {
    data: &'a [u8],
    values_start: usize,
    offset_bytes: &'a [u8],
    total_values_size: usize,
    pos: usize,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> Iterator for IndexedArrayIter<'a, T> {
    type Item = T::Reader<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let n = self.offset_bytes.len() / 8;
        if self.pos >= n {
            return None;
        }
        let read_off = |i: usize| -> usize {
            if i < n {
                let pos = i * 8;
                u64::from_le_bytes(self.offset_bytes[pos..pos + 8].try_into().unwrap()) as usize
            } else {
                self.total_values_size
            }
        };
        let start = self.values_start + read_off(self.pos);
        let end = self.values_start + read_off(self.pos + 1);
        self.pos += 1;
        Some(T::read(self.data, start..end))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.offset_bytes.len() / 8;
        let rem = n.saturating_sub(self.pos);
        (rem, Some(rem))
    }
}

impl<'a, T: MmapRead + 'a> From<IndexedArrayReader<'a, T>> for Vec<T>
where
    <T as MmapRead>::Reader<'a>: Into<T>,
{
    fn from(r: IndexedArrayReader<'a, T>) -> Self {
        r.iter().map(|e| e.into()).collect()
    }
}

/// Write an indexed array (count + offsets + values).
pub fn write_indexed_array<'data, T: crate::traits::MmapWrite>(
    items: &'data [T],
    ctx: &mut crate::WriteContext<'data>,
) {
    let count_pos = ctx.current_offset();
    ctx.write_u64_le(0);
    let offsets_pos = ctx.current_offset();
    ctx.write_padding(items.len() * 8);

    let mut current_offset: u64 = 0;
    for (i, elem) in items.iter().enumerate() {
        ctx.write_u64_le_at(current_offset, offsets_pos + i * 8);
        elem.write(ctx);
        ctx.align_to(8);
        let values_start = offsets_pos + items.len() * 8;
        current_offset = (ctx.current_offset() - values_start) as u64;
    }
    ctx.write_u64_le_at(items.len() as u64, count_pos);
}
