use crate::traits::{fnv1a, LayoutHash, MmapRead, MmapWrite};
use std::ops::Range;

/// String storage: raw bytes (no length prefix — length from range bounds).
/// Reader type is &str — zero-copy view into mmap.

impl LayoutHash for String {
    const HASH: u64 = fnv1a(b"String");
}

#[cfg(not(feature = "safe-strings"))]
fn validate_utf8(slice: &[u8]) -> &str {
    unsafe { std::str::from_utf8_unchecked(slice) }
}

#[cfg(feature = "safe-strings")]
fn validate_utf8(slice: &[u8]) -> &str {
    match std::str::from_utf8(slice) {
        Ok(s) => s,
        Err(e) => {
            let valid_up_to = e.valid_up_to();
            if cfg!(debug_assertions) {
                eprintln!(
                    "warning: mmapstruct: invalid UTF-8 at offset {}, truncating",
                    valid_up_to
                );
            }
            // SAFETY: valid_up_to is guaranteed to be a valid UTF-8 boundary
            unsafe { std::str::from_utf8_unchecked(&slice[..valid_up_to]) }
        }
    }
}

impl MmapRead for String {
    type Reader<'a> = &'a str;

    fn read<'a>(data: &'a [u8], range: Range<usize>) -> Self::Reader<'a> {
        let slice = &data[range];
        if slice.is_empty() {
            return "";
        }
        let s = validate_utf8(slice);
        crate::fixup_str(s)
    }
}

impl MmapWrite for String {
    fn write<'data>(&'data self, ctx: &mut crate::WriteContext<'data>) {
        ctx.write_raw(self.as_bytes());
    }
}

impl MmapWrite for &str {
    fn write<'data>(&'data self, ctx: &mut crate::WriteContext<'data>) {
        ctx.write_raw(self.as_bytes());
    }
}

impl crate::traits::ReaderOwned for String {
    fn into_owned(reader: Self::Reader<'_>) -> Self {
        reader.to_string()
    }
}
