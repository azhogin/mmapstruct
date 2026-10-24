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

/// Wrap an io error so its `Display` includes the file path (std io errors
/// omit the path, e.g. `No such file or directory (os error 2)`).  The
/// `ErrorKind` is preserved, so `err.kind()` matching keeps working.
fn with_path(path: &Path, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{}: {err}", path.display()))
}

/// Parsed mmapstruct header: `(payload, optional_embedded_schema_range,
/// layout_hash, crc64)`.  The schema range is `(start, end)` within `data`.
pub type ParsedHeader<'a> = (&'a [u8], Option<(usize, usize)>, Option<u64>, Option<u64>);

/// Parse a mmapstruct header (v0/v1/v2) from `data`.
pub fn parse_header(data: &[u8]) -> ParsedHeader<'_> {
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

/// Reader that owns a memory-mapped file and provides typed readers.
pub struct MmapReader {
    mmap: Mmap,
    embedded_schema: Option<Vec<u8>>,
    /// Byte range of the payload within `mmap` (after any header).
    payload_range: Range<usize>,
    /// Authoritative `layout_hash` from the header source (embedded or sidecar).
    auth_hash: Option<u64>,
    /// Authoritative `crc64` from the header source (None when zero/disabled).
    auth_crc: Option<u64>,
}

impl MmapReader {
    fn new_internal(
        mmap: Mmap,
        payload_range: Range<usize>,
        auth_hash: Option<u64>,
        auth_crc: Option<u64>,
        embedded_schema: Option<Vec<u8>>,
    ) -> Self {
        Self {
            mmap,
            embedded_schema,
            payload_range,
            auth_hash,
            auth_crc,
        }
    }

    /// Memory-map a file at `path` (standard embedded-header file).
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|e| with_path(path, e))?;
        let mmap = unsafe { Mmap::map(&file).map_err(|e| with_path(path, e))? };
        let (payload, schema, hash, crc) = parse_header(&mmap);
        let payload_start = payload.as_ptr() as usize - mmap.as_ptr() as usize;
        let payload_range = payload_start..payload_start + payload.len();
        let embedded_schema = schema.map(|(s, e)| mmap[s..e].to_vec());
        Ok(Self::new_internal(
            mmap,
            payload_range,
            hash,
            crc,
            embedded_schema,
        ))
    }

    /// Open a headerless payload file whose mmapstruct header/schema lives in a
    /// sidecar `<path>.mmshdr`. The data file is treated as pure payload
    /// (offset 0), so it is directly O_DIRECT-aligned.
    pub fn from_file_with_sidecar(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let mut sidecar_path = path.to_path_buf();
        sidecar_path.set_extension("mmshdr");
        let sidecar = std::fs::read(&sidecar_path).map_err(|e| with_path(&sidecar_path, e))?;
        let (_sp, schema, hash, crc) = parse_header(&sidecar);
        let embedded_schema = schema.map(|(s, e)| sidecar[s..e].to_vec());
        let file = File::open(path).map_err(|e| with_path(path, e))?;
        let mmap = unsafe { Mmap::map(&file).map_err(|e| with_path(path, e))? };
        let payload_range = 0..mmap.len();
        Ok(Self::new_internal(
            mmap,
            payload_range,
            hash,
            crc,
            embedded_schema,
        ))
    }

    /// Open a headerless payload file RAW: the whole file is the payload,
    /// and the layout-hash/crc checks are skipped entirely (like the
    /// O_DIRECT expert-cache reads — the caller guarantees the layout).
    pub fn from_file_raw(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|e| with_path(path, e))?;
        let mmap = unsafe { Mmap::map(&file).map_err(|e| with_path(path, e))? };
        let payload_range = 0..mmap.len();
        Ok(Self::new_internal(mmap, payload_range, None, None, None))
    }

    /// Wrap an existing `Mmap`.
    pub fn new(mmap: Mmap) -> Self {
        let (payload, schema, hash, crc) = parse_header(&mmap);
        let payload_start = payload.as_ptr() as usize - mmap.as_ptr() as usize;
        let payload_range = payload_start..payload_start + payload.len();
        let embedded_schema = schema.map(|(s, e)| mmap[s..e].to_vec());
        Self::new_internal(mmap, payload_range, hash, crc, embedded_schema)
    }

    /// Read the file as type `T`, skipping the header and verifying the
    /// authoritative `layout_hash` and `crc64` (from the embedded header or the
    /// sidecar, whichever was used to open the file).
    pub fn read<T: MmapRead + LayoutHash>(&self) -> T::Reader<'_> {
        let data = &self.mmap[self.payload_range.clone()];
        if let Some(actual) = self.auth_hash {
            let expected = T::HASH;
            assert_eq!(
                actual, expected,
                "layout_hash mismatch: header has 0x{:016x}, struct expects 0x{:016x}. The file was written with a different struct layout.",
                actual, expected
            );
        }
        if let Some(crc_value) = self.auth_crc {
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
        &self.mmap[self.payload_range.clone()]
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
    /// Byte range of the payload within `buf` (after any header).
    payload_range: Range<usize>,
    /// Authoritative `layout_hash` from the header source (embedded or sidecar).
    auth_hash: Option<u64>,
    /// Authoritative `crc64` from the header source (None when zero/disabled).
    auth_crc: Option<u64>,
}

impl BufReader {
    fn new_internal(
        buf: Vec<u8>,
        payload_range: Range<usize>,
        auth_hash: Option<u64>,
        auth_crc: Option<u64>,
        embedded_schema: Option<Vec<u8>>,
    ) -> Self {
        Self {
            buf,
            embedded_schema,
            payload_range,
            auth_hash,
            auth_crc,
        }
    }

    /// Read file at `path` into memory (standard embedded-header file).
    pub fn from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let buf = std::fs::read(path).map_err(|e| with_path(path, e))?;
        let (payload, schema, hash, crc) = parse_header(&buf);
        let payload_start = payload.as_ptr() as usize - buf.as_ptr() as usize;
        let payload_range = payload_start..payload_start + payload.len();
        let embedded_schema = schema.map(|(s, e)| buf[s..e].to_vec());
        Ok(Self::new_internal(
            buf,
            payload_range,
            hash,
            crc,
            embedded_schema,
        ))
    }

    /// Open a headerless payload file whose mmapstruct header/schema lives in a
    /// sidecar `<path>.mmshdr`. The data file is treated as pure payload
    /// (offset 0), so it is directly O_DIRECT-aligned.
    pub fn from_file_with_sidecar(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        let buf = std::fs::read(path).map_err(|e| with_path(path, e))?;
        let mut sidecar_path = path.to_path_buf();
        sidecar_path.set_extension("mmshdr");
        let sidecar = std::fs::read(&sidecar_path).map_err(|e| with_path(&sidecar_path, e))?;
        let (_sp, schema, hash, crc) = parse_header(&sidecar);
        let embedded_schema = schema.map(|(s, e)| sidecar[s..e].to_vec());
        let payload_range = 0..buf.len();
        Ok(Self::new_internal(
            buf,
            payload_range,
            hash,
            crc,
            embedded_schema,
        ))
    }

    /// Wrap an existing `Vec<u8>`.
    pub fn new(buf: Vec<u8>) -> Self {
        let (payload, schema, hash, crc) = parse_header(&buf);
        let payload_start = payload.as_ptr() as usize - buf.as_ptr() as usize;
        let payload_range = payload_start..payload_start + payload.len();
        let embedded_schema = schema.map(|(s, e)| buf[s..e].to_vec());
        Self::new_internal(buf, payload_range, hash, crc, embedded_schema)
    }

    /// Read the buffer as type `T`, skipping the header and verifying the
    /// authoritative `layout_hash` and `crc64` (from the embedded header or the
    /// sidecar, whichever was used to open the file).
    pub fn read<T: MmapRead + LayoutHash>(&self) -> T::Reader<'_> {
        let data = &self.buf[self.payload_range.clone()];
        if let Some(actual) = self.auth_hash {
            let expected = T::HASH;
            assert_eq!(
                actual, expected,
                "layout_hash mismatch: header has 0x{:016x}, struct expects 0x{:016x}. The file was written with a different struct layout.",
                actual, expected
            );
        }
        if let Some(crc_value) = self.auth_crc {
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
        &self.buf[self.payload_range.clone()]
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

// ---------------------------------------------------------------------------
// Layout-hash patching — in-place header fixup for pre-existing files
// ---------------------------------------------------------------------------

/// Stored `layout_hash` from a headered file's 24-byte header.
///
/// Returns `Ok(None)` for raw payloads, the old 8-byte v0 format, unknown
/// versions, or headers without the hash flag.
pub fn stored_layout_hash(path: &Path) -> io::Result<Option<u64>> {
    use std::io::Read;
    let mut f = File::open(path).map_err(|e| with_path(path, e))?;
    let mut hdr = [0u8; HEADER_SIZE];
    let n = f.read(&mut hdr).map_err(|e| with_path(path, e))?;
    if n < HEADER_SIZE {
        return Ok(None);
    }
    let header: FileHeader = unsafe { mmap_read_pod(&hdr) };
    if header.magic != MMAP_MAGIC {
        return Ok(None);
    }
    match header.hdr_ver {
        HEADER_VER_1 | HEADER_VER_2 => {
            if header.flags & HEADER_FLAG_HASH_PRESENT != 0 {
                Ok(Some(header.layout_hash))
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

/// Overwrite the `layout_hash` field of a headered file in place
/// (8 bytes at offset 8). The payload and CRC are untouched.
///
/// Errors for files that are too small, lack the MMAP magic, or carry no
/// hash flag (raw / v0).
pub fn write_layout_hash(path: &Path, new_hash: u64) -> io::Result<()> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut f = File::options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| with_path(path, e))?;
    let mut hdr = [0u8; HEADER_SIZE];
    f.read_exact(&mut hdr).map_err(|e| with_path(path, e))?;
    let header: FileHeader = unsafe { mmap_read_pod(&hdr) };
    if header.magic != MMAP_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: not an MMAP-headered file", path.display()),
        ));
    }
    if !matches!(header.hdr_ver, HEADER_VER_1 | HEADER_VER_2)
        || header.flags & HEADER_FLAG_HASH_PRESENT == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "{}: header has no layout_hash slot (v0/raw)",
                path.display()
            ),
        ));
    }
    hdr[8..16].copy_from_slice(&new_hash.to_le_bytes());
    f.seek(SeekFrom::Start(0)).map_err(|e| with_path(path, e))?;
    f.write_all(&hdr).map_err(|e| with_path(path, e))?;
    f.sync_all().map_err(|e| with_path(path, e))
}

/// Recompute [`LayoutHash::HASH`] for `T` and write it into the file header.
///
/// One-shot migration tool for files written before a change to the hash
/// composition formula (binary layouts unchanged). O(1) per file regardless
/// of file size.
pub fn patch_layout_hash<T: LayoutHash>(path: &Path) -> io::Result<()> {
    write_layout_hash(path, T::HASH)
}
