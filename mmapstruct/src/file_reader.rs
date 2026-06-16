use std::fs::File;
use std::io;
use std::path::Path;
use std::ops::Range;
use memmap2::Mmap;
use crate::traits::{MmapRead, LayoutHash, FileHeader, HEADER_SIZE, HEADER_VER_1, HEADER_FLAG_HASH_PRESENT, MMAP_MAGIC, mmap_read_pod, compute_crc64};

/// Check for a mmapstruct header and skip it.
/// Returns the payload slice (everything after the header).
/// If a layout_hash is present in a v1 header, writes it to `out_hash` (caller may verify).
/// If a CRC is present in a v1 header, writes it to `out_crc` (caller may verify).
/// v0 (old 8-byte) headers are detected via hdr_ver == 0 and skipped without hash/CRC check.
fn strip_header_raw<'a>(data: &'a [u8], out_hash: &mut Option<u64>, out_crc: &mut Option<u64>) -> &'a [u8] {
    if data.len() >= HEADER_SIZE {
        let header: FileHeader = unsafe { mmap_read_pod(&data[..HEADER_SIZE]) };
        if header.magic == MMAP_MAGIC {
            match header.hdr_ver {
                HEADER_VER_1 => {
                    if header.flags & HEADER_FLAG_HASH_PRESENT != 0 {
                        *out_hash = Some(header.layout_hash);
                    }
                    if header.crc64 != 0 {
                        *out_crc = Some(header.crc64);
                    }
                    return &data[HEADER_SIZE..];
                }
                0 => {
                    // Old 8-byte format: skip 8 bytes
                    return &data[8..];
                }
                _ => {} // Unknown version, treat as no header
            }
        }
    }
    data
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
}

impl MmapReader {
    /// Memory-map a file at `path`.
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path.as_ref())?;
        let mmap = unsafe { Mmap::map(&file)? };
        Ok(Self { mmap })
    }

    /// Wrap an existing `Mmap`.
    pub fn new(mmap: Mmap) -> Self {
        Self { mmap }
    }

    /// Read the file as type `T`, auto‑skipping the header if present.
    /// Verifies layout_hash when the header contains one.
    /// Verifies CRC64 when the header has a non-zero CRC.
    pub fn read<T: MmapRead + LayoutHash>(&self) -> T::Reader<'_> {
        let (data, crc) = strip_header_checked::<T>(&self.mmap);
        if let Some(crc_value) = crc {
            if crc_value != 0 {
                let actual = compute_crc64(data);
                assert_eq!(actual, crc_value,
                    "CRC64 mismatch: file may be corrupt. Expected 0x{:016x}, got 0x{:016x}",
                    crc_value, actual);
            }
        }
        let range: Range<usize> = 0..data.len();
        T::read(data, range)
    }

    /// Return the payload bytes (after stripping the header, no hash check).
    pub fn payload(&self) -> &[u8] {
        strip_header(&self.mmap)
    }

    /// Return the underlying `Mmap`.
    pub fn into_inner(self) -> Mmap {
        self.mmap
    }
}

/// Reader that owns a byte buffer (read from file via `std::fs::read`).
pub struct BufReader {
    buf: Vec<u8>,
}

impl BufReader {
    /// Read file at `path` into memory.
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let buf = std::fs::read(path.as_ref())?;
        Ok(Self { buf })
    }

    /// Wrap an existing `Vec<u8>`.
    pub fn new(buf: Vec<u8>) -> Self {
        Self { buf }
    }

    /// Read the buffer as type `T`, auto‑skipping the header if present.
    /// Verifies layout_hash when the header contains one.
    /// Verifies CRC64 when the header has a non-zero CRC.
    pub fn read<T: MmapRead + LayoutHash>(&self) -> T::Reader<'_> {
        let (data, crc) = strip_header_checked::<T>(&self.buf);
        if let Some(crc_value) = crc {
            if crc_value != 0 {
                let actual = compute_crc64(data);
                assert_eq!(actual, crc_value,
                    "CRC64 mismatch: file may be corrupt. Expected 0x{:016x}, got 0x{:016x}",
                    crc_value, actual);
            }
        }
        let range: Range<usize> = 0..data.len();
        T::read(data, range)
    }

    /// Return the payload bytes (after stripping the header, no hash check).
    pub fn payload(&self) -> &[u8] {
        strip_header(&self.buf)
    }

    /// Return the underlying buffer.
    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }
}
