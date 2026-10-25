//! In-place patching of existing mmapstruct files via a mutable mmap.
//!
//! A [`PatchSession`] opens a headered file read-write, validates its
//! identity (`layout_hash`), and exposes typed patch operations over the
//! mapped payload. Data written through the session is visible to readers
//! that map the same file afterwards (after [`PatchSession::flush`]).
//!
//! Operations:
//! - [`PatchSession::patch_pod`] — overwrite a fixed-section scalar or an
//!   element of a `Vec<Pod>` at a payload offset.
//! - [`PatchSession::offset_of`] — locate the payload offset of any
//!   reference obtained from the typed reader view (pod fields, pod vec
//!   slices, string slices all borrow directly from the mmap).
//! - [`PatchSession::append_pod_vec`] — append elements to a `Vec<Pod>`
//!   block that occupies the tail of the payload (crash-consistent
//!   protocol: bytes → offset-table sentinel → count switch).
//!
//! Policies:
//! - CRC-protected files are rejected: the whole-payload CRC64 cannot be
//!   updated by a partial patch, so patching silently would leave the file
//!   flagged corrupt.
//! - The header `layout_hash` is mandatory and (for `open_typed*`) checked
//!   against `T::HASH` before any write is possible.
//! - Indexed (`Vec<NonFixedSize>`) and byref containers are **not**
//!   appendable at this stage: their slot tables are pinned in front of the
//!   values, so growing a table requires shifting live data — that breaks
//!   concurrent readers and has no crash-consistent protocol without
//!   reserved slot slack (planned as the reserve stage).

use crate::file_reader::parse_header;
use crate::traits::{LayoutHash, MmapLayout, MmapPod, MmapRead};
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
    file: File,
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
        let (file, mmap, payload_range, auth_hash) = match source {
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
                (file, mmap, payload_range, auth_hash)
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
                (file, mmap, payload_range, auth_hash)
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
            file,
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

    /// Extend the backing file by `add` bytes and re-map.
    ///
    /// The payload range always ends at EOF for both header sources, so the
    /// new bytes are appended payload bytes (zero-filled until written).
    fn grow(&mut self, add: usize) -> io::Result<()> {
        if add == 0 {
            return Ok(());
        }
        self.mmap.flush()?;
        let new_len =
            self.mmap.len().checked_add(add).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "file length overflow")
            })?;
        self.file.set_len(new_len as u64)?;
        self.mmap = unsafe { MmapMut::map_mut(&self.file) }?;
        self.payload_range.end =
            self.payload_range.start + (self.mmap.len() - self.payload_range.start);
        Ok(())
    }

    /// Single aligned u64 overwrite (8-byte store; used for the offset-table
    /// sentinel, which is always 8-byte aligned).
    fn patch_u64_at(&mut self, pos: usize, value: u64) -> io::Result<()> {
        let end = pos
            .checked_add(8)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "patch offset overflow"))?;
        if end > self.payload_range.len() || !pos.is_multiple_of(8) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("u64 patch target [{pos}, {end}) is out of bounds or misaligned"),
            ));
        }
        self.payload_mut()[pos..end].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }

    /// Update a count field (u64) that acts as the visibility switch.
    ///
    /// Written as two 4-byte halves, low dword first: a count only grows on
    /// append, so while the high dwords are equal a torn read (between the
    /// two stores) yields either the old or the new count, never a mix.
    /// Counts crossing 2^32 are rejected as non-atomic.
    fn patch_count_u64(&mut self, pos: usize, old: u64, new: u64) -> io::Result<()> {
        if (old >> 32) != (new >> 32) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("count patch {old} -> {new} crosses the 2^32 boundary; not crash-atomic"),
            ));
        }
        let data = self.payload_mut();
        data[pos..pos + 4].copy_from_slice(&(new as u32).to_le_bytes());
        data[pos + 4..pos + 8].copy_from_slice(&((new >> 32) as u32).to_le_bytes());
        Ok(())
    }

    /// Validate that a container block `[block_start, block_end)` sits at the
    /// tail of the payload and matches the root layout `R`.
    ///
    /// Returns `(sentinel_pos, sentinel_value)` — the offset-table sentinel
    /// (total dynamic-region size) that must be patched alongside the count.
    fn tail_block_check<R: MmapLayout>(
        &self,
        block_start: usize,
        block_end: usize,
    ) -> io::Result<(usize, u64)> {
        let payload_len = self.payload_range.len();
        let hs = R::HEADER_SIZE;
        if R::INLINE_DYN_COUNT == 0 || hs < 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "root layout has no dynamic region (INLINE_DYN_COUNT == 0 or HEADER_SIZE < 8)",
            ));
        }
        if block_start < hs || !block_start.is_multiple_of(4) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "block start {block_start} is outside the dynamic region [{hs}, {payload_len}) or not 4-byte aligned"
                ),
            ));
        }
        if block_end != payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "container is not at the payload tail (block ends at {block_end}, payload ends at {payload_len}); tail-append requires the container to be the last dynamic value of the root struct, with no byref data after it"
                ),
            ));
        }
        let sentinel_pos = hs - 8;
        let sentinel = u64::from_le_bytes(
            self.payload()[sentinel_pos..sentinel_pos + 8]
                .try_into()
                .unwrap(),
        );
        let expected = (payload_len - hs) as u64;
        if sentinel != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "offset-table sentinel is {sentinel:#x}, expected {expected:#x} (total dynamic size); the file does not match the root layout"
                ),
            ));
        }
        Ok((sentinel_pos, sentinel))
    }

    /// Append pod elements to a `Vec<T>` (pod-element) block that occupies
    /// the tail of the payload.
    ///
    /// `elems_off` is the payload-relative offset of the elements region:
    /// `session.offset_of(&reader.vec[0])` for non-empty vecs, or
    /// `session.payload().len()` for an empty one (the count field is then
    /// the last 8 bytes of the payload).
    ///
    /// `R` is the root struct type the file was written with. The block must
    /// be the LAST inline-dynamic value of that root, with no byref data
    /// after it (the tail check enforces this). Other dynamic fields may
    /// precede the block freely.
    ///
    /// Crash-consistency: the element bytes and the offset-table sentinel are
    /// written first; the count is the visibility switch (see
    /// [`Self::patch_count_u64`]). A crash before the count update leaves the
    /// extra bytes orphaned but the file valid with the old element count.
    ///
    /// Note: the typed reader view must be dropped before calling (it borrows
    /// the session immutably).
    pub fn append_pod_vec<R: MmapLayout, T: MmapPod>(
        &mut self,
        elems_off: usize,
        add: &[T],
    ) -> io::Result<()> {
        let elem_size = std::mem::size_of::<T>();
        if elem_size == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zero-sized elements are not supported",
            ));
        }
        let payload_len = self.payload_range.len();
        if elems_off < 8 || elems_off > payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("elems_off {elems_off} is outside the payload range [8, {payload_len}]"),
            ));
        }
        let block_start = elems_off - 8;
        let count = u64::from_le_bytes(
            self.payload()[block_start..block_start + 8]
                .try_into()
                .unwrap(),
        ) as usize;
        let block_end = elems_off
            .checked_add(count.checked_mul(elem_size).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "element region size overflow")
            })?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "block end overflow"))?;
        let (sentinel_pos, sentinel) = self.tail_block_check::<R>(block_start, block_end)?;
        if !elems_off.is_multiple_of(std::mem::align_of::<T>()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "elements region offset {elems_off} is not aligned to align_of::<T>() == {}",
                    std::mem::align_of::<T>()
                ),
            ));
        }
        if add.is_empty() {
            return Ok(());
        }
        let add_bytes = std::mem::size_of_val(add);
        self.grow(add_bytes)?;
        {
            let p = self.payload_mut();
            let start = p.len() - add_bytes;
            p[start..].copy_from_slice(bytemuck::cast_slice(add));
        }
        self.patch_u64_at(sentinel_pos, sentinel + add_bytes as u64)?;
        self.patch_count_u64(block_start, count as u64, (count + add.len()) as u64)?;
        Ok(())
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
