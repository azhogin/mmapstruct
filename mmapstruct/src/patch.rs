//! In-place patching of existing mmapstruct files via a mutable mmap.
//!
//! A [`PatchSession`] opens a headered file read-write, validates its
//! identity (`layout_hash`), and exposes typed patch operations over the
//! mapped payload. Data written through the session is visible to readers
//! that map the same file afterwards (after [`PatchSession::flush`]).
//!
//! Stage-1 operations (no structural file changes):
//! - [`PatchSession::patch_pod`] — overwrite a fixed-section scalar or an
//!   element of a `Vec<Pod>` at a payload offset.
//! - [`PatchSession::offset_of`] — locate the payload offset of any
//!   reference obtained from the typed reader view (pod fields, pod vec
//!   slices, string slices all borrow directly from the mmap).
//!
//! Policies:
//! - CRC-protected files are rejected: the whole-payload CRC64 cannot be
//!   updated by a partial patch, so patching silently would leave the file
//!   flagged corrupt.
//! - The header `layout_hash` is mandatory and (for `open_typed*`) checked
//!   against `T::HASH` before any write is possible.
//! - Structural changes (appending to containers) are out of scope for this
//!   stage and arrive in later patching stages.

use crate::file_reader::parse_header;
use crate::traits::{LayoutHash, MmapPod, MmapRead};
use memmap2::MmapMut;
use std::fs::File;
use std::io;
use std::ops::Range;
use std::path::Path;

fn with_path(path: &Path, err: io::Error) -> io::Error {
    io::Error::new(err.kind(), format!("{}: {err}", path.display()))
}

fn patch_err(path: &Path, msg: impl AsRef<str>) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{}: {}", path.display(), msg.as_ref()),
    )
}

/// Where the mmapstruct header lives for a given data file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderSource {
    /// Standard file: 24-byte header (optionally with embedded schema) before payload.
    Embedded,
    /// Split layout: header in sidecar `<path>.mmshdr`, payload is the whole file.
    Sidecar,
}

/// Read-write session over an existing mmapstruct file.
pub struct PatchSession {
    mmap: MmapMut,
    /// Byte range of the payload within `mmap` (after any header).
    payload_range: Range<usize>,
    /// Authoritative `layout_hash` from the header source.
    auth_hash: u64,
}

impl std::fmt::Debug for PatchSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatchSession")
            .field("payload_len", &self.payload_range.len())
            .field("layout_hash", &format_args!("0x{:016x}", self.auth_hash))
            .finish()
    }
}

impl PatchSession {
    fn open_impl(
        path: &Path,
        source: HeaderSource,
        expected_hash: Option<u64>,
    ) -> io::Result<Self> {
        let (mmap, payload_range, auth_hash) = match source {
            HeaderSource::Embedded => {
                let file = File::options()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(|e| with_path(path, e))?;
                let mmap = unsafe { MmapMut::map_mut(&file) }.map_err(|e| with_path(path, e))?;
                let data: &[u8] = &mmap;
                let (payload, _schema, hash, crc) = parse_header(data);
                let auth_hash = validate_header(path, hash, crc)?;
                let payload_start = payload.as_ptr() as usize - mmap.as_ptr() as usize;
                let payload_range = payload_start..payload_start + payload.len();
                (mmap, payload_range, auth_hash)
            }
            HeaderSource::Sidecar => {
                let mut sidecar_path = path.to_path_buf();
                sidecar_path.set_extension("mmshdr");
                let sidecar =
                    std::fs::read(&sidecar_path).map_err(|e| with_path(&sidecar_path, e))?;
                let (_payload, _schema, hash, crc) = parse_header(&sidecar);
                let auth_hash = validate_header(&sidecar_path, hash, crc)?;
                let file = File::options()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(|e| with_path(path, e))?;
                let mmap = unsafe { MmapMut::map_mut(&file) }.map_err(|e| with_path(path, e))?;
                let payload_range = 0..mmap.len();
                (mmap, payload_range, auth_hash)
            }
        };
        if let Some(expected) = expected_hash {
            if auth_hash != expected {
                return Err(patch_err(
                    path,
                    format!(
                        "layout_hash mismatch: header has 0x{auth_hash:016x}, struct expects 0x{expected:016x}. The file was written with a different struct layout."
                    ),
                ));
            }
        }
        Ok(Self {
            mmap,
            payload_range,
            auth_hash,
        })
    }

    /// Open a standard headered file for patching (identity validated, type
    /// checked lazily by [`PatchSession::read`]).
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_impl(path.as_ref(), HeaderSource::Embedded, None)
    }

    /// Open a standard headered file for patching, rejecting it unless the
    /// header hash matches `T::HASH`.
    pub fn open_typed<T: LayoutHash>(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_impl(path.as_ref(), HeaderSource::Embedded, Some(T::HASH))
    }

    /// Open a split-layout file (payload at `path`, header in `<path>.mmshdr`)
    /// for patching.
    pub fn open_with_sidecar(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_impl(path.as_ref(), HeaderSource::Sidecar, None)
    }

    /// Open a split-layout file for patching, rejecting it unless the sidecar
    /// hash matches `T::HASH`.
    pub fn open_typed_with_sidecar<T: LayoutHash>(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_impl(path.as_ref(), HeaderSource::Sidecar, Some(T::HASH))
    }

    /// Read the payload as type `T` (hash-verified against the header).
    ///
    /// The returned reader borrows the immutable view of the mapped payload;
    /// references extracted from it can be located with
    /// [`PatchSession::offset_of`] and patched.
    pub fn read<T: MmapRead + LayoutHash>(&self) -> io::Result<T::Reader<'_>> {
        if self.auth_hash != T::HASH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "layout_hash mismatch: header has 0x{:016x}, struct expects 0x{:016x}. The file was written with a different struct layout.",
                    self.auth_hash,
                    T::HASH
                ),
            ));
        }
        let data = self.payload();
        Ok(T::read(data, 0..data.len()))
    }

    /// Payload bytes (after stripping the header, hash not re-checked).
    pub fn payload(&self) -> &[u8] {
        &self.mmap[self.payload_range.clone()]
    }

    /// Mutable payload bytes. Expert-level: callers must only write bytes
    /// whose layout is understood (e.g. ranges located through the typed
    /// reader view). Prefer [`PatchSession::patch_pod`] for typed writes.
    pub fn payload_mut(&mut self) -> &mut [u8] {
        &mut self.mmap[self.payload_range.clone()]
    }

    /// Payload-relative byte offset of a reference obtained from the typed
    /// reader view (`&PodField`, `&[PodElem]`, `&str`, ...).
    ///
    /// Errors when the reference does not lie inside the mapped payload.
    pub fn offset_of<T: ?Sized>(&self, value: &T) -> io::Result<usize> {
        let ptr = (value as *const T) as *const u8 as usize;
        let base = (self.mmap.as_ptr() as usize) + self.payload_range.start;
        let end = base + self.payload_range.len();
        if ptr < base || ptr > end {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reference lies outside the file payload range",
            ));
        }
        Ok(ptr - base)
    }

    /// Overwrite the pod value at payload-relative `offset` with `value`.
    ///
    /// The write is bounds-checked and requires natural alignment of `T`
    /// (`offset % align_of::<T>() == 0`), which every offset produced from
    /// the typed reader view satisfies. For aligned values of `size <= 8`
    /// the write is a single word store and cannot be observed torn.
    pub fn patch_pod<T: MmapPod>(&mut self, offset: usize, value: &T) -> io::Result<()> {
        let size = std::mem::size_of::<T>();
        if size == 0 {
            return Ok(());
        }
        let align = std::mem::align_of::<T>();
        if !offset.is_multiple_of(align) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("patch offset {offset} is not aligned to align_of::<T>() == {align}"),
            ));
        }
        let end = offset
            .checked_add(size)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "patch offset overflow"))?;
        if end > self.payload_range.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "patch range [{offset}, {end}) exceeds payload length {}",
                    self.payload_range.len()
                ),
            ));
        }
        let bytes = bytemuck::bytes_of(value);
        self.payload_mut()[offset..end].copy_from_slice(bytes);
        Ok(())
    }

    /// Flush dirty mmap pages to the backing file.
    pub fn flush(&mut self) -> io::Result<()> {
        self.mmap.flush()
    }
}

fn validate_header(path: &Path, hash: Option<u64>, crc: Option<u64>) -> io::Result<u64> {
    let hash = hash.ok_or_else(|| {
        patch_err(
            path,
            "file has no layout_hash (raw or v0 format); patching requires an identity hash",
        )
    })?;
    if crc.is_some() {
        return Err(patch_err(
            path,
            "file is CRC-protected; in-place patching is not supported for CRC-covered files",
        ));
    }
    Ok(hash)
}
