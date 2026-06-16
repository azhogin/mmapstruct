use crate::traits::ByRefRange;
use std::ops::Range;

/// Resolve a ByRefRange field: read the 8-byte ByRefRange, then read T from it.
pub fn read_by_ref<'a, T: crate::traits::MmapRead>(
    data: &'a [u8],
    base: usize,
    byref_range: Range<usize>,
) -> T::Reader<'a> {
    let byref: ByRefRange = *bytemuck::from_bytes(&data[byref_range]);
    let range = resolve_byref_range(base, byref.begin as usize, byref.end as usize);
    T::read(data, range)
}

#[cfg(not(feature = "strict-validation"))]
fn resolve_byref_range(base: usize, begin: usize, end: usize) -> Range<usize> {
    base + begin..base + end
}

#[cfg(feature = "strict-validation")]
fn resolve_byref_range(base: usize, begin: usize, end: usize) -> Range<usize> {
    let begin = base.checked_add(begin)
        .expect("read_by_ref: begin overflow");
    let end = base.checked_add(end)
        .expect("read_by_ref: end overflow");
    assert!(begin <= end, "read_by_ref: begin {} > end {}", begin, end);
    begin..end
}
