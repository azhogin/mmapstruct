use crate::WriteContext;
use bytemuck::{Pod, Zeroable};
use std::ops::Range;

pub const HEADER_SIZE: usize = 24;
pub const MMAP_MAGIC: u32 = u32::from_le_bytes(*b"MMAP");
pub const HEADER_VER_1: u16 = 1;
pub const HEADER_VER_2: u16 = 2;
pub const HEADER_FLAG_HASH_PRESENT: u16 = 1 << 0;
pub const HEADER_FLAG_HAS_SCHEMA: u16 = 1 << 1;
pub const HEADER_FLAG_PAYLOAD_ALIGNED: u16 = 1 << 2;

/// 24-byte file header.
///
/// v1 Layout:
/// ```text
/// [magic: u32, hdr_ver=1: u16, flags: u16, layout_hash: u64, crc64: u64]
/// ```
///
/// v2 Layout (when `flags & HEADER_FLAG_HAS_SCHEMA`):
/// ```text
/// [magic: u32, hdr_ver=2: u16, flags: u16, layout_hash: u64, crc64: u64]
/// [schema_size: u32le][schema bytes (padded to 8)]
/// [optional alignment padding when PAYLOAD_ALIGNED is set]
/// [payload...]
/// ```
/// CRC covers the payload only (for backward compatibility).
///
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct FileHeader {
    pub magic: u32,
    pub hdr_ver: u16,
    pub flags: u16,
    pub layout_hash: u64,
    pub crc64: u64,
}

impl FileHeader {
    pub fn new<T: LayoutHash>() -> Self {
        Self {
            magic: MMAP_MAGIC,
            hdr_ver: HEADER_VER_1,
            flags: HEADER_FLAG_HASH_PRESENT,
            layout_hash: T::HASH,
            crc64: 0,
        }
    }

    pub fn with_schema<T: LayoutHash>() -> Self {
        Self {
            magic: MMAP_MAGIC,
            hdr_ver: HEADER_VER_2,
            flags: HEADER_FLAG_HASH_PRESENT | HEADER_FLAG_HAS_SCHEMA,
            layout_hash: T::HASH,
            crc64: 0,
        }
    }

    pub fn with_schema_aligned<T: LayoutHash>() -> Self {
        Self {
            magic: MMAP_MAGIC,
            hdr_ver: HEADER_VER_2,
            flags: HEADER_FLAG_HASH_PRESENT | HEADER_FLAG_HAS_SCHEMA | HEADER_FLAG_PAYLOAD_ALIGNED,
            layout_hash: T::HASH,
            crc64: 0,
        }
    }

    /// Total size of the extended header (base header + schema prefix + alignment).
    /// Without schema, returns `HEADER_SIZE`.
    /// With schema, returns `HEADER_SIZE + 4 + padded_schema_size`, potentially
    /// plus alignment padding when `HEADER_FLAG_PAYLOAD_ALIGNED` is set.
    pub fn total_header_size(&self, schema_bytes: &[u8]) -> usize {
        if self.flags & HEADER_FLAG_HAS_SCHEMA != 0 {
            let padded = (schema_bytes.len() + 7) & !7;
            let h = HEADER_SIZE + 4 + padded;
            if self.flags & HEADER_FLAG_PAYLOAD_ALIGNED != 0 {
                let align = 8usize;
                h + (align - (h % align)) % align
            } else {
                h
            }
        } else {
            HEADER_SIZE
        }
    }
}

// ---------------------------------------------------------------------------
// CRC-64-ECMA
// ---------------------------------------------------------------------------

const CRC64_POLY: u64 = 0xC96C5795D7870F42;

pub const fn build_crc64_table() -> [u64; 256] {
    let mut table = [0u64; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u64;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ CRC64_POLY;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

pub const CRC64_TABLE: [u64; 256] = build_crc64_table();

pub const fn compute_crc64(data: &[u8]) -> u64 {
    let mut crc = !0u64;
    let mut i = 0;
    while i < data.len() {
        let idx = ((crc ^ data[i] as u64) & 0xFF) as usize;
        crc = CRC64_TABLE[idx] ^ (crc >> 8);
        i += 1;
    }
    !crc
}

// ---------------------------------------------------------------------------
// FNV-1a hash helpers (const fn, no deps)
// ---------------------------------------------------------------------------

pub const fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}

pub const fn combine_hashes(hashes: &[u64]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut i = 0;
    while i < hashes.len() {
        h ^= hashes[i];
        h = h.wrapping_mul(0x100000001b3);
        i += 1;
    }
    h
}

/// Trait for layout identity: a structural hash of the binary format.
///
/// Every type that has a defined binary layout implements this trait.
/// The hash is computed entirely at compile-time (const evaluation).
/// Changed a field? The hash changes automatically → old files rejected.
pub trait LayoutHash {
    const HASH: u64;
}

/// Compile-time layout facts of a struct's mmap representation, emitted by
/// `#[derive(MmapStruct)]` for structs (not enums: enum variants have
/// per-variant layouts).
///
/// Lets patching tools locate the instance's inline-dyn offset table:
/// the table starts at `FIXED_SECTION_SIZE` and holds
/// `INLINE_DYN_COUNT + 1` u64 entries (the last one is the sentinel with
/// the total dynamic-region size), so the payload layout is
/// `[fixed: FIXED_SECTION_SIZE][offset table][dynamic values]` and the
/// sentinel lives at `HEADER_SIZE - 8`.
pub trait MmapLayout {
    /// Size of the fixed section of the instance.
    const FIXED_SECTION_SIZE: usize;
    /// Fixed section + offset table (including the sentinel entry).
    const HEADER_SIZE: usize;
    /// Number of inline-dyn fields.
    const INLINE_DYN_COUNT: usize;
}

// -- Primitives --
macro_rules! impl_layout_hash_primitive {
    ($t:ty, $name:literal) => {
        impl LayoutHash for $t {
            const HASH: u64 = fnv1a($name);
        }
    };
}

impl_layout_hash_primitive!(u8, b"u8");
impl_layout_hash_primitive!(u16, b"u16");
impl_layout_hash_primitive!(u32, b"u32");
impl_layout_hash_primitive!(u64, b"u64");
impl_layout_hash_primitive!(i8, b"i8");
impl_layout_hash_primitive!(i16, b"i16");
impl_layout_hash_primitive!(i32, b"i32");
impl_layout_hash_primitive!(i64, b"i64");
impl_layout_hash_primitive!(f32, b"f32");
impl_layout_hash_primitive!(f64, b"f64");
impl_layout_hash_primitive!(bool, b"bool");
impl_layout_hash_primitive!(usize, b"usize");
impl_layout_hash_primitive!(isize, b"isize");

// -- [T; N] fixed-size array (base hash, encoding added by parent struct's derive) --
impl<T: LayoutHash, const N: usize> LayoutHash for [T; N] {
    const HASH: u64 = combine_hashes(&[fnv1a(b"Array"), T::HASH, N as u64]);
}

// -- Vec<T> (base hash, encoding added by parent struct's derive) --
impl<T: LayoutHash> LayoutHash for Vec<T> {
    const HASH: u64 = combine_hashes(&[fnv1a(b"Vec"), T::HASH]);
}

// -- HashMap<K, V> (base hash, encoding added by parent struct's derive) --
impl<K: LayoutHash, V: LayoutHash> LayoutHash for std::collections::HashMap<K, V> {
    const HASH: u64 = combine_hashes(&[fnv1a(b"HashMap"), K::HASH, V::HASH]);
}

// -- Option<T> (base hash, encoding added by parent struct's derive) --
impl<T: LayoutHash> LayoutHash for Option<T> {
    const HASH: u64 = combine_hashes(&[fnv1a(b"Option"), T::HASH]);
}

// -- Encoding tags for field-level wrapping --
pub const ENC_BYREF: u64 = fnv1a(b"BYREF");
pub const ENC_INLINE: u64 = fnv1a(b"INLINE");
pub const ENC_POD_VEC: u64 = fnv1a(b"POD_VEC");
pub const ENC_FIXED_VEC: u64 = fnv1a(b"FIXED_VEC");
pub const ENC_BYREF_VEC: u64 = fnv1a(b"BYREF_VEC");
pub const ENC_HASHMAP: u64 = fnv1a(b"HASHMAP");
pub const ENC_FIXED_ARRAY: u64 = fnv1a(b"FIXED_ARRAY");
pub const ENC_POD_CUSTOM: u64 = fnv1a(b"POD_CUSTOM");
pub const ENC_LIST: u64 = fnv1a(b"LIST");

/// High bit of a container's count word marking a *reserved* (self-describing)
/// block: the word after the count holds the reserve amounts
/// (`u32 slots | u32 bytes` for indexed blocks, `u32 bytes` for pod-vec
/// blocks). Old (unflagged) blocks are unchanged and fully backward
/// compatible; element counts below 2^63 never collide with the flag.
pub const BLOCK_FLAG_RESERVED: u64 = 1u64 << 63;

// ---------------------------------------------------------------------------
// Pod I/O
// ---------------------------------------------------------------------------
///
/// Both paths use `core::ptr::read_unaligned` so there is no alignment
/// requirement.  With `strict-validation` an additional size check is added.
///
/// # Safety
///
/// `data` must contain at least `size_of::<T>()` readable bytes.  Without
/// the `strict-validation` feature the size is **not** asserted, so the
/// caller is responsible for the bound; a short slice causes an out-of-bounds
/// read.  `T` must be a `Pod` type, so any bit pattern in the range is valid.
#[inline(always)]
pub unsafe fn mmap_read_pod<T: Pod>(data: &[u8]) -> T {
    #[cfg(feature = "strict-validation")]
    {
        assert!(
            data.len() >= std::mem::size_of::<T>(),
            "mmap_read_pod: buffer too short: len {} < size {}",
            data.len(),
            std::mem::size_of::<T>()
        );
    }
    core::ptr::read_unaligned(data.as_ptr() as *const T)
}

/// A by-reference range [begin, end) stored inline in the fixed section.
/// Offsets are relative to the start of the containing structure.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct ByRefRange {
    pub begin: u32,
    pub end: u32,
}

impl ByRefRange {
    pub fn range(&self, base: usize) -> Range<usize> {
        resolve_byref_range(base, self.begin as usize, self.end as usize)
    }
}

#[cfg(not(feature = "strict-validation"))]
fn resolve_byref_range(base: usize, begin: usize, end: usize) -> Range<usize> {
    base + begin..base + end
}

#[cfg(feature = "strict-validation")]
fn resolve_byref_range(base: usize, begin: usize, end: usize) -> Range<usize> {
    let begin = base.checked_add(begin).expect("ByRefRange: begin overflow");
    let end = base.checked_add(end).expect("ByRefRange: end overflow");
    assert!(begin <= end, "ByRefRange: begin {} > end {}", begin, end);
    begin..end
}

/// Trait for types that can be read from a memory-mapped byte range.
pub trait MmapRead: Sized {
    type Reader<'a>: 'a
    where
        Self: 'a;

    fn read<'a>(data: &'a [u8], range: Range<usize>) -> Self::Reader<'a>;
}

/// Trait for types that can be written/encoded into a byte buffer.
pub trait MmapWrite: Sized {
    fn write<'data>(&'data self, ctx: &mut WriteContext<'data>);
}

/// A range within a mapped file, used by readers.
#[derive(Clone, Copy, Debug)]
pub struct FileRange {
    pub start: usize,
    pub end: usize,
}

impl FileRange {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    pub fn sub_range(&self, offset: usize, size: usize) -> Self {
        Self {
            start: self.start + offset,
            end: self.start + offset + size,
        }
    }
}

/// Marker trait for Pod types that can be read/written directly as bytes.
pub trait MmapPod: Pod + Copy + 'static {}

impl<T: MmapPod, const N: usize> MmapPod for [T; N] {}

impl<T: MmapPod> MmapRead for T {
    type Reader<'a> = &'a T;

    fn read<'a>(data: &'a [u8], range: Range<usize>) -> Self::Reader<'a> {
        bytemuck::from_bytes(&data[range])
    }
}

impl<T: MmapPod> MmapWrite for T {
    fn write<'data>(&'data self, ctx: &mut WriteContext<'data>) {
        ctx.write_raw(bytemuck::bytes_of(self));
    }
}

/// Compare a stored reader directly with a key, without materializing the key.
pub trait ReaderOrd: MmapRead {
    fn cmp_reader(reader: &Self::Reader<'_>, key: &Self) -> std::cmp::Ordering;
}

impl<T: MmapPod + Ord> ReaderOrd for T {
    fn cmp_reader(reader: &&T, key: &T) -> std::cmp::Ordering {
        (**reader).cmp(key)
    }
}

impl ReaderOrd for String {
    fn cmp_reader(reader: &&str, key: &String) -> std::cmp::Ordering {
        (*reader).cmp(key.as_str())
    }
}

impl<T: ReaderOrd> ReaderOrd for Option<T> {
    fn cmp_reader(
        reader: &Option<<T as MmapRead>::Reader<'_>>,
        key: &Option<T>,
    ) -> std::cmp::Ordering {
        match (reader, key) {
            (Some(r), Some(k)) => T::cmp_reader(r, k),
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
        }
    }
}

/// Convert a reader type back into the owned type.
/// Implemented for pod types (Copy) and String.
pub trait ReaderOwned: MmapRead {
    fn into_owned(reader: Self::Reader<'_>) -> Self;
}

impl<T: MmapPod> ReaderOwned for T {
    fn into_owned(reader: Self::Reader<'_>) -> Self {
        *reader
    }
}

impl From<Range<usize>> for FileRange {
    fn from(r: Range<usize>) -> Self {
        Self {
            start: r.start,
            end: r.end,
        }
    }
}
