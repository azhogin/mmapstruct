use crate::traits::{
    compute_crc64, mmap_read_pod, FileHeader, LayoutHash, MmapRead, HEADER_FLAG_HASH_PRESENT,
    HEADER_FLAG_HAS_SCHEMA, HEADER_FLAG_PAYLOAD_ALIGNED, HEADER_SIZE, HEADER_VER_1, HEADER_VER_2,
    MMAP_MAGIC,
};
use memmap2::Mmap;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;

fn read_u32_le(data: &[u8]) -> u32 {
    u32::from_le_bytes([data[0], data[1], data[2], data[3]])
}

/// Parse a mmapstruct header (v0/v1/v2) from `data`.
/// Returns `(payload, optional_embedded_schema_range, layout_hash, crc64)`.
/// The schema range is `(start, end)` within `data`.
pub fn parse_header<'a>(data: &'a [u8]) -> (&'a [u8], Option<(usize, usize)>, Option<u64>, Option<u64>) {
    if data.len() < HEADER_SIZE {
        return (data, None, None, None);
    }
    let header: FileHeader = unsafe { mmap_read_pod(&data[..HEADER_SIZE]) };
    if header.magic != MMAP_MAGIC {
        return (data, None, None, None);
    }
    match header.hdr_ver {
        HEADER_VER_1 => {
            let hash = if header.flags & HEADER_FLAG_HASH_PRESENT != 0 {
                Some(header.layout_hash)
            } else {
                None
            };
            let crc = if header.crc64 != 0 {
                Some(header.crc64)
            } else {
                None
            };
            (&data[HEADER_SIZE..], None, hash, crc)
        }
        HEADER_VER_2 => {
            let hash = if header.flags & HEADER_FLAG_HASH_PRESENT != 0 {
                Some(header.layout_hash)
            } else {
                None
            };
            let crc = if header.crc64 != 0 {
                Some(header.crc64)
            } else {
                None
            };
            if header.flags & HEADER_FLAG_HAS_SCHEMA != 0 {
                let pos = HEADER_SIZE;
                let schema_size = read_u32_le(&data[pos..pos + 4]) as usize;
                let padded = (schema_size + 7) & !7;
                let schema = (pos + 4, pos + 4 + schema_size);
                let mut payload_start = pos + 4 + padded;
                if header.flags & HEADER_FLAG_PAYLOAD_ALIGNED != 0 {
                    let align = 8usize;
                    payload_start += (align - (payload_start % align)) % align;
                }
                (&data[payload_start..], Some(schema), hash, crc)
            } else {
                (&data[HEADER_SIZE..], None, hash, crc)
            }
        }
        0 => {
            // Old 8-byte format: skip 8 bytes
            (&data[8..], None, None, None)
        }
        _ => {
            // Unknown version, treat as no header
            (data, None, None, None)
        }
    }
}

fn strip_header_raw<'a>(
    data: &'a [u8],
    out_hash: &mut Option<u64>,
    out_crc: &mut Option<u64>,
) -> &'a [u8] {
    let (payload, _schema, hash, crc) = parse_header(data);
    *out_hash = hash;
    *out_crc = crc;
    payload
}

fn strip_header<'a>(data: &'a [u8]) -> &'a [u8] {
    strip_header_raw(data, &mut None, &mut None)
}

fn strip_header_checked<'a, T: LayoutHash>(data: &'a [u8]) -> (&'a [u8], Option<u64>) {
    let mut hash = None;
    let mut crc = None;
    let payload = strip_header_raw(data, &mut hash, &mut crc);
    if let Some(actual) = hash {
        let expected = T::HASH;
        assert_eq!(actual, expected,
            "layout_hash mismatch: header has 0x{:016x}, struct expects 0x{:016x}. The file was written with a different struct layout.", actual, expected);
    }
    (payload, crc)
}

/// Reader that owns a memory-mapped file and provides typed readers.
pub struct MmapReader {
    mmap: Mmap,
    embedded_schema: Option<Vec<u8>>,
}

impl MmapReader {
    /// Memory-map a file at `path`.
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path.as_ref())?;
        let mmap = unsafe { Mmap::map(&file)? };
        let embedded_schema = match parse_header(&mmap).1 {
            Some((start, end)) => Some(mmap[start..end].to_vec()),
            None => None,
        };
        Ok(Self { mmap, embedded_schema })
    }

    /// Wrap an existing `Mmap`.
    pub fn new(mmap: Mmap) -> Self {
        let embedded_schema = match parse_header(&mmap).1 {
            Some((start, end)) => Some(mmap[start..end].to_vec()),
            None => None,
        };
        Self { mmap, embedded_schema }
    }

    /// Read the file as type `T`, auto‑skipping the header if present.
    /// Verifies layout_hash when the header contains one.
    /// Verifies CRC64 when the header has a non-zero CRC.
    pub fn read<T: MmapRead + LayoutHash>(&self) -> T::Reader<'_> {
        let (data, crc) = strip_header_checked::<T>(&self.mmap);
        if let Some(crc_value) = crc {
            if crc_value != 0 {
                let actual = compute_crc64(data);
                assert_eq!(
                    actual, crc_value,
                    "CRC64 mismatch: file may be corrupt. Expected 0x{:016x}, got 0x{:016x}",
                    crc_value, actual
                );
            }
        }
        let range: Range<usize> = 0..data.len();
        T::read(data, range)
    }

    /// Return the payload bytes (after stripping the header, no hash check).
    pub fn payload(&self) -> &[u8] {
        strip_header(&self.mmap)
    }

    /// Return the embedded schema bytes, if present.
    pub fn embedded_schema(&self) -> Option<&[u8]> {
        self.embedded_schema.as_deref()
    }

    /// Return the underlying `Mmap`.
    pub fn into_inner(self) -> Mmap {
        self.mmap
    }
}

/// Reader that owns a byte buffer (read from file via `std::fs::read`).
pub struct BufReader {
    buf: Vec<u8>,
    embedded_schema: Option<Vec<u8>>,
}

impl BufReader {
    /// Read file at `path` into memory.
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let buf = std::fs::read(path.as_ref())?;
        let embedded_schema = match parse_header(&buf).1 {
            Some((start, end)) => Some(buf[start..end].to_vec()),
            None => None,
        };
        Ok(Self { buf, embedded_schema })
    }

    /// Wrap an existing `Vec<u8>`.
    pub fn new(buf: Vec<u8>) -> Self {
        let embedded_schema = match parse_header(&buf).1 {
            Some((start, end)) => Some(buf[start..end].to_vec()),
            None => None,
        };
        Self { embedded_schema, buf }
    }

    /// Read the buffer as type `T`, auto‑skipping the header if present.
    /// Verifies layout_hash when the header contains one.
    /// Verifies CRC64 when the header has a non-zero CRC.
    pub fn read<T: MmapRead + LayoutHash>(&self) -> T::Reader<'_> {
        let (data, crc) = strip_header_checked::<T>(&self.buf);
        if let Some(crc_value) = crc {
            if crc_value != 0 {
                let actual = compute_crc64(data);
                assert_eq!(
                    actual, crc_value,
                    "CRC64 mismatch: file may be corrupt. Expected 0x{:016x}, got 0x{:016x}",
                    crc_value, actual
                );
            }
        }
        let range: Range<usize> = 0..data.len();
        T::read(data, range)
    }

    /// Return the payload bytes (after stripping the header, no hash check).
    pub fn payload(&self) -> &[u8] {
        strip_header(&self.buf)
    }

    /// Return the embedded schema bytes, if present.
    pub fn embedded_schema(&self) -> Option<&[u8]> {
        self.embedded_schema.as_deref()
    }

    /// Return the underlying buffer.
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}
