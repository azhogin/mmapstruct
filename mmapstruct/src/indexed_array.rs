use crate::traits::{FileRange, MmapRead, ReaderOwned, BLOCK_FLAG_RESERVED};
use crate::write_context::Reserve;
use std::marker::PhantomData;

#[derive(Clone)]
pub struct IndexedArrayReader<'a, T> {
    data: &'a [u8],
    #[allow(dead_code)]
    range: FileRange,
    count: usize,
    /// Base for the stored offsets: block start for reserved (flagged)
    /// blocks (offsets are block-relative there), values start for plain
    /// blocks (offsets are values-relative).
    base: usize,
    offset_bytes: &'a [u8],
    /// Byte size of the values region relative to `base`.
    total_values_size: usize,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> IndexedArrayReader<'a, T> {
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        let raw = u64::from_le_bytes(data[range.start..range.start + 8].try_into().unwrap());
        let flagged = raw & BLOCK_FLAG_RESERVED != 0;
        let count = (raw & !BLOCK_FLAG_RESERVED) as usize;
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
        let slots_pos = range.start + if flagged { 16 } else { 8 };
        let offset_bytes = &data[slots_pos..slots_pos + count * 8];
        let (base, total_values_size) = if flagged {
            let resv =
                u64::from_le_bytes(data[range.start + 8..range.start + 16].try_into().unwrap());
            let reserved_bytes = (resv >> 32) as usize;
            // Reserved blocks store block-relative offsets; the implicit end
            // of the values region is the span end minus the (possibly
            // already partially consumed) data gap.
            (range.start, range.len() - reserved_bytes)
        } else {
            let offsets_offset = range.start + 8;
            let values_offset = offsets_offset + count * 8;
            (values_offset, range.end - values_offset)
        };

        Self {
            data,
            range,
            count,
            base,
            offset_bytes,
            total_values_size,
            _phantom: PhantomData,
        }
    }

    pub fn len(&self) -> usize {
        self.count
    }
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// File-relative range of the whole block
    /// (`[count u64][offset slots][values]`).
    ///
    /// `start` is the position of the count field — the anchor for
    /// append-style patching of a block that occupies the payload tail.
    pub fn block_range(&self) -> FileRange {
        self.range
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
        let start = self.base + self.offset(index);
        let end = self.base + self.offset(index + 1);
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
            values_start: self.base,
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

impl<'a, T: MmapRead + ReaderOwned + 'a> From<IndexedArrayReader<'a, T>> for Vec<T> {
    fn from(r: IndexedArrayReader<'a, T>) -> Self {
        r.iter()
            .map(<T as crate::traits::ReaderOwned>::into_owned)
            .collect()
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

/// [`write_indexed_array`] with an optional reserve.
///
/// Reserved blocks are self-describing:
/// `[count | BLOCK_FLAG_RESERVED][reserve word][count+slots offset slots][values][gap]`
/// where the reserve word holds spare table slots in its low 32 bits and the
/// spare data bytes in its high 32 bits. Offsets in reserved blocks are
/// **block-relative** (the reader derives value positions from the block
/// start, so appending values into the gap never invalidates existing
/// slots), and the implicit end of the values region is the span end minus
/// the remaining gap bytes. A patch session appends in place: value bytes →
/// new slot → count switch → reserve-word update.
pub fn write_indexed_array_r<'data, T: crate::traits::MmapWrite>(
    items: &'data [T],
    ctx: &mut crate::WriteContext<'data>,
    reserve: Reserve,
) {
    if reserve.is_none() {
        write_indexed_array(items, ctx);
        return;
    }
    let count_pos = ctx.current_offset();
    ctx.write_u64_le((items.len() as u64) | BLOCK_FLAG_RESERVED);
    ctx.write_u64_le(((reserve.bytes << 32) | reserve.slots as u64) & !(1u64 << 63));
    let offsets_pos = ctx.current_offset();
    ctx.write_padding((items.len() + reserve.slots as usize) * 8);

    // First value starts after the (count + spare) slot table, block-relative.
    let mut current_offset: u64 =
        ((offsets_pos - count_pos) + (items.len() + reserve.slots as usize) * 8) as u64;
    for (i, elem) in items.iter().enumerate() {
        ctx.write_u64_le_at(current_offset, offsets_pos + i * 8);
        elem.write(ctx);
        ctx.align_to(8);
        // Block-relative offset (block starts at count_pos).
        current_offset = (ctx.current_offset() - count_pos) as u64;
    }
    // Data gap after the values: patchable space for future elements.
    if reserve.bytes > 0 {
        ctx.write_padding(reserve.bytes as usize);
    }
    ctx.write_u64_le_at((items.len() as u64) | BLOCK_FLAG_RESERVED, count_pos);
}
