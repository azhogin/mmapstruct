use crate::traits::{mmap_read_pod, FileRange, MmapRead, ReaderOrd};
use std::collections::HashMap;

#[inline]
pub fn read_count(data: &[u8], range: &FileRange) -> usize {
    let hdr = std::mem::size_of::<u64>();
    if range.len() >= hdr {
        unsafe { mmap_read_pod::<u64>(&data[range.start..range.start + hdr]) as usize }
    } else {
        0
    }
}

#[inline(always)]
pub fn read_offset(data: &[u8], ot_start: usize, index: usize) -> u32 {
    let pos = ot_start + index * 4;
    unsafe { mmap_read_pod::<u32>(&data[pos..pos + 4]) }
}

#[inline(always)]
pub(crate) const fn align_up(offset: usize, align: usize) -> usize {
    (offset + align - 1) & !(align - 1)
}

pub fn binary_search_reader<'a, K: MmapRead + ReaderOrd + 'a>(
    data: &'a [u8],
    key_offsets_start: usize,
    keys_area_start: usize,
    key: &K,
    count: usize,
) -> Option<usize> {
    let mut lo = 0isize;
    let mut hi = count as isize - 1;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        let start = keys_area_start + read_offset(data, key_offsets_start, mid as usize) as usize;
        let end = keys_area_start + read_offset(data, key_offsets_start, mid as usize + 1) as usize;
        let r = K::read(data, start..end);
        match K::cmp_reader(&r, key) {
            std::cmp::Ordering::Less => lo = mid + 1,
            std::cmp::Ordering::Greater => hi = mid - 1,
            std::cmp::Ordering::Equal => return Some(mid as usize),
        }
    }
    None
}

pub fn iter_offset_ranges<'a>(
    data: &'a [u8],
    ot_start: usize,
    area_start: usize,
    count: usize,
) -> impl Iterator<Item = (usize, usize)> + 'a {
    (0..count).map(move |i| {
        let start = area_start + read_offset(data, ot_start, i) as usize;
        let end = area_start + read_offset(data, ot_start, i + 1) as usize;
        (start, end)
    })
}

pub fn sort_pairs<K: Ord, V>(map: &HashMap<K, V>) -> Vec<(&K, &V)> {
    let mut pairs: Vec<(&K, &V)> = map.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
}

pub fn reserve_offset_table(ctx: &mut crate::WriteContext<'_>, count: usize) -> usize {
    let start = ctx.current_offset();
    ctx.write_padding((count + 1) * 4);
    start
}

pub fn write_offset_table(ctx: &mut crate::WriteContext<'_>, ot_start: usize, offsets: &[u32]) {
    for (i, off) in offsets.iter().enumerate() {
        ctx.write_u32_le_at(*off, ot_start + i * 4);
    }
}

pub fn align_write(ctx: &mut crate::WriteContext<'_>, align: usize) {
    let raw = ctx.current_offset();
    let aligned = align_up(raw, align);
    ctx.write_padding(aligned - raw);
}
