use crate::traits::{MmapRead, MmapWrite, ReaderOwned};
use std::ops::Range;

/// Layout for `Option<T>`: `[present: u8, data_if_present...]`
impl<T: MmapRead> MmapRead for Option<T> {
    type Reader<'a>
        = Option<T::Reader<'a>>
    where
        T: 'a;

    fn read<'a>(data: &'a [u8], range: Range<usize>) -> Self::Reader<'a> {
        if range.is_empty() {
            return None;
        }
        if range.start >= data.len() {
            // Corrupt range: panic with context instead of a raw index panic.
            panic!(
                "Option read: present byte at {} exceeds the buffer (len {})",
                range.start,
                data.len()
            );
        }
        if data[range.start] == 0 {
            None
        } else {
            let val_start = (range.start + 1 + std::mem::align_of::<T>() - 1)
                & !(std::mem::align_of::<T>() - 1);
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

/// Convert an `Option` of a reader view into an owned `Option` (elements of
/// `Vec<Option<T>>` and friends). The present byte is already resolved by
/// [`MmapRead::read`].
impl<T: ReaderOwned> ReaderOwned for Option<T> {
    fn into_owned(reader: Self::Reader<'_>) -> Self {
        reader.map(T::into_owned)
    }
}
