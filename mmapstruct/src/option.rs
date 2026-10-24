use crate::traits::{MmapRead, MmapWrite};
use std::ops::Range;

/// Layout for `Option<T>`: `[present: u8, data_if_present...]`
impl<T: MmapRead> MmapRead for Option<T> {
    type Reader<'a>
        = Option<T::Reader<'a>>
    where
        T: 'a;

    fn read<'a>(data: &'a [u8], range: Range<usize>) -> Self::Reader<'a> {
        if range.is_empty() || data[range.start] == 0 {
            None
        } else {
            let val_start = (range.start + 1 + std::mem::align_of::<T>() - 1) & !(std::mem::align_of::<T>() - 1);
            Some(T::read(data, val_start..range.end))
        }
    }
}

impl<T: MmapWrite> MmapWrite for Option<T> {
    fn write<'data>(&'data self, ctx: &mut crate::WriteContext<'data>) {
        match self {
            Some(value) => {
                ctx.write_u8(1u8);
                ctx.align_to(std::mem::align_of::<T>());
                value.write(ctx);
            }
            None => {
                ctx.write_u8(0u8);
            }
        }
    }
}
