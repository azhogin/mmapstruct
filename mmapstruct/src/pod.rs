use std::ops::Range;
use crate::traits::{MmapPod, MmapRead, MmapWrite};

impl MmapPod for u8 {}
impl MmapPod for u16 {}
impl MmapPod for u32 {}
impl MmapPod for u64 {}
impl MmapPod for i8 {}
impl MmapPod for i16 {}
impl MmapPod for i32 {}
impl MmapPod for i64 {}
impl MmapPod for f32 {}
impl MmapPod for f64 {}
impl MmapPod for usize {}
impl MmapPod for isize {}

// bool is not Pod — explicit impls
impl MmapRead for bool {
    type Reader<'a> = bool;
    fn read<'a>(data: &'a [u8], range: Range<usize>) -> Self::Reader<'a> {
        data[range.start] != 0
    }
}
impl MmapWrite for bool {
    fn write<'data>(&'data self, ctx: &mut crate::WriteContext<'data>) {
        ctx.write_u8(if *self { 1u8 } else { 0u8 });
    }
}
