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
use crate::list::{LIST_BLOCK_SIZE, LIST_NODE_HEADER};
use crate::traits::{ByRefRange, LayoutHash, MmapLayout, MmapPod, MmapRead, MmapWrite};
use crate::write_context::WriteContext;
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

    /// Update a count word (u64) that acts as the visibility switch.
    ///
    /// `old_word` is the full current word (including a possible
    /// [`BLOCK_FLAG_RESERVED`] bit — it is preserved), `new_count` is the new
    /// element count without the flag. Written as two 4-byte halves, low dword
    /// first: a count only grows on append, so while the count high dwords are
    /// equal a torn read (between the two stores) yields either the old or the
    /// new count, never a mix. Counts crossing 2^32 are rejected as
    /// non-atomic.
    fn patch_count_u64(&mut self, pos: usize, old_word: u64, new_count: u64) -> io::Result<()> {
        let old_count = old_word & !crate::traits::BLOCK_FLAG_RESERVED;
        if (old_count >> 32) != (new_count >> 32) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("count patch {old_count} -> {new_count} crosses the 2^32 boundary; not crash-atomic"),
            ));
        }
        let flag_hi = if old_word & crate::traits::BLOCK_FLAG_RESERVED != 0 {
            0x8000_0000u32
        } else {
            0
        };
        let high = ((new_count >> 32) as u32) | flag_hi;
        let data = self.payload_mut();
        data[pos..pos + 4].copy_from_slice(&(new_count as u32).to_le_bytes());
        data[pos + 4..pos + 8].copy_from_slice(&high.to_le_bytes());
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
        if block_end != payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "container is not at the payload tail (block ends at {block_end}, payload ends at {payload_len}); tail-append requires the container to be the last dynamic value of the root struct, with no byref data after it"
                ),
            ));
        }
        self.layout_check::<R>(block_start)
    }

    /// Validate that a container block belongs to the root layout `R`:
    /// dynamic-region placement, 4-byte alignment, and the offset-table
    /// sentinel matching the payload size.
    ///
    /// Returns `(sentinel_pos, sentinel_value)` — the offset-table sentinel
    /// (total dynamic-region size) that must be patched when a span grows.
    fn layout_check<R: MmapLayout>(&self, block_start: usize) -> io::Result<(usize, u64)> {
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
        let sentinel_pos = hs - 8;
        let sentinel = u64::from_le_bytes(
            self.payload()[sentinel_pos..sentinel_pos + 8]
                .try_into()
                .unwrap(),
        );
        let expected = (payload_len - hs) as u64;
        // The sentinel counts only the inline dynamic values; byref (postponed)
        // data lives after them, so `sentinel <= expected` for valid files
        // (equality exactly when no byref data exists).
        if sentinel > expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "offset-table sentinel is {sentinel:#x}, larger than the payload allows ({expected:#x}); the file does not match the root layout"
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
        // Reserved (flagged) pod-vec blocks carry [count|FLAG][resv] before
        // the elements (16-byte header), plain blocks only [count] (8 bytes).
        // Disambiguate: the flagged probe must have the flag bit set AND the
        // candidate block start must be an offset-table entry of the root
        // (interior span positions never are, so the pair cannot falsely
        // trigger on pod data that happens to have bit 63 set).
        let (block_start, raw) = if elems_off >= 16 {
            let probe = u64::from_le_bytes(
                self.payload()[elems_off - 16..elems_off - 8]
                    .try_into()
                    .unwrap(),
            );
            if probe & crate::traits::BLOCK_FLAG_RESERVED != 0
                && self.span_end_for_block::<R>(elems_off - 16).is_ok()
            {
                (elems_off - 16, probe)
            } else {
                (
                    elems_off - 8,
                    u64::from_le_bytes(
                        self.payload()[elems_off - 8..elems_off].try_into().unwrap(),
                    ),
                )
            }
        } else {
            (
                elems_off - 8,
                u64::from_le_bytes(self.payload()[elems_off - 8..elems_off].try_into().unwrap()),
            )
        };
        let flagged = raw & crate::traits::BLOCK_FLAG_RESERVED != 0;
        let count = (raw & !crate::traits::BLOCK_FLAG_RESERVED) as usize;
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
        if flagged {
            // Reserved block: append in place into the data gap. The gap sits
            // right after the current elements; the field's span (and thus
            // the offset-table sentinel) does not change.
            self.layout_check::<R>(block_start)?;
            let resv = u64::from_le_bytes(
                self.payload()[block_start + 8..block_start + 16]
                    .try_into()
                    .unwrap(),
            );
            let gap_bytes = (resv >> 32) as usize;
            if add_bytes > gap_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "pod-vec data reserve exhausted: gap holds {gap_bytes} bytes, need {add_bytes}; rewrite the file with a larger reserve"
                    ),
                ));
            }
            let gap_start = elems_off + count * elem_size;
            {
                let p = self.payload_mut();
                p[gap_start..gap_start + add_bytes].copy_from_slice(bytemuck::cast_slice(add));
            }
            // Count switch first (elements are already in place), then the
            // reserve-word bookkeeping.
            self.patch_count_u64(block_start, raw, (count + add.len()) as u64)?;
            self.patch_u64_at(
                block_start + 8,
                (((gap_bytes - add_bytes) as u64) << 32) | (resv & 0xFFFF_FFFF),
            )?;
            return Ok(());
        }
        let block_end = elems_off
            .checked_add(count.checked_mul(elem_size).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "element region size overflow")
            })?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "block end overflow"))?;
        let (sentinel_pos, sentinel) = self.tail_block_check::<R>(block_start, block_end)?;
        self.grow(add_bytes)?;
        {
            let p = self.payload_mut();
            let start = p.len() - add_bytes;
            p[start..].copy_from_slice(bytemuck::cast_slice(add));
        }
        self.patch_u64_at(sentinel_pos, sentinel + add_bytes as u64)?;
        self.patch_count_u64(block_start, raw, (count + add.len()) as u64)?;
        Ok(())
    }

    /// Locate the span end (exclusive) of the dynamic field whose block sits
    /// at `block_off`, by scanning the root offset table of `R`.
    ///
    /// For the last dynamic field this is the payload end; otherwise the next
    /// field's block start.
    fn span_end_for_block<R: MmapLayout>(&self, block_off: usize) -> io::Result<usize> {
        let hs = R::HEADER_SIZE;
        if block_off < hs {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "block offset is before the dynamic region",
            ));
        }
        let fs = R::FIXED_SECTION_SIZE;
        let n = R::INLINE_DYN_COUNT;
        let data = self.payload();
        let target = (block_off - hs) as u64;
        for i in 0..n {
            let entry = u64::from_le_bytes(data[fs + i * 8..fs + i * 8 + 8].try_into().unwrap());
            if entry == target {
                return Ok(if i + 1 < n {
                    (u64::from_le_bytes(
                        data[fs + (i + 1) * 8..fs + (i + 1) * 8 + 8]
                            .try_into()
                            .unwrap(),
                    ) as usize)
                        + hs
                } else {
                    self.payload_range.len()
                });
            }
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "block is not referenced by the root offset table",
        ))
    }

    /// Append one element to a reserved `Vec<T>` (non-fixed-size elements)
    /// block anywhere in the file.
    ///
    /// Requires the block to be written with a reserve
    /// (`ReservePlan`/`write_to_buf_with`): spare table slots make the slot
    /// table extensible and the trailing data gap holds the new value bytes.
    /// Fully in place — the file does not grow and the offset-table sentinel
    /// is unchanged. The tail position is NOT required.
    ///
    /// Crash-consistency: value bytes and the new slot are written before the
    /// count switch; between the count switch and the reserve-word update the
    /// new element reads as zero-length (memory-safe), and a crash there
    /// self-heals on the next append (the gap position is derived from the
    /// stale reserve word).
    pub fn append_indexed<R: MmapLayout, T: MmapWrite>(
        &mut self,
        block_off: usize,
        value: &T,
    ) -> io::Result<()> {
        let payload_len = self.payload_range.len();
        if !block_off.is_multiple_of(8) || block_off + 16 > payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("indexed block offset {block_off} is misaligned or too close to the payload end ({payload_len})"),
            ));
        }
        let data = self.payload();
        let raw = u64::from_le_bytes(data[block_off..block_off + 8].try_into().unwrap());
        if raw & crate::traits::BLOCK_FLAG_RESERVED == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "indexed block has no reserve; without spare slots a crash-safe append is impossible — rewrite the file with a ReservePlan (write_to_buf_with)",
            ));
        }
        let count = (raw & !crate::traits::BLOCK_FLAG_RESERVED) as usize;
        let resv = u64::from_le_bytes(data[block_off + 8..block_off + 16].try_into().unwrap());
        let spare_slots = (resv & 0xFFFF_FFFF) as usize;
        let gap_bytes = (resv >> 32) as usize;
        self.layout_check::<R>(block_off)?;
        if spare_slots == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "indexed reserve exhausted: no spare table slots left; rewrite the file with a larger reserve",
            ));
        }
        let span_end = self.span_end_for_block::<R>(block_off)?;
        let mut ctx = WriteContext::new();
        value.write(&mut ctx);
        let vbytes = ctx.finish();
        let vlen = vbytes.len();
        // Land the value at its natural alignment inside the gap (the gap
        // start itself is aligned at build time; later appends re-align).
        // The alignment padding is charged to the reserve together with the
        // value bytes, so the next append lands aligned again. For align-1
        // elements (String, Vec<u8>) the content is exact; for larger
        // alignments trailing pad bytes sit beyond the instance fields and
        // never affect reads.
        let align = std::mem::align_of::<T>().max(1);
        let raw_start = span_end - gap_bytes;
        let gap_start = if align > 1 {
            (raw_start + align - 1) & !(align - 1)
        } else {
            raw_start
        };
        let consumed = (gap_start - raw_start) + vlen;
        if consumed > gap_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "indexed data reserve exhausted: gap holds {gap_bytes} bytes, need {consumed}; rewrite the file with a larger reserve"
                ),
            ));
        }
        // Sanity: the gap must live after the slot table and the values.
        let min_gap_start = block_off + 16 + (count + spare_slots) * 8;
        if gap_start < min_gap_start {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "indexed block layout is inconsistent: gap start {gap_start} < {min_gap_start}"
                ),
            ));
        }
        {
            let p = self.payload_mut();
            p[gap_start..gap_start + vlen].copy_from_slice(&vbytes);
            let slot_pos = block_off + 16 + count * 8;
            let slot_value = (gap_start - block_off) as u64;
            p[slot_pos..slot_pos + 8].copy_from_slice(&slot_value.to_le_bytes());
        }
        // Count is the visibility switch; the reserve word bookkeeping
        // (one slot consumed, the data gap shrunk) follows.
        self.patch_count_u64(block_off, raw, (count + 1) as u64)?;
        self.patch_u64_at(
            block_off + 8,
            (((gap_bytes - consumed) as u64) << 32) | ((spare_slots - 1) as u64),
        )?;
        Ok(())
    }

    /// Append one element to a reserved `Vec<T>` (`#[mmap(byref)]`) block
    /// anywhere in the file.
    ///
    /// Requires the block to be written with a byref reserve
    /// ([`crate::byref_array::write_byref_array_r`] — root-level non-dedup
    /// field + a `ReservePlan` entry). Reserved block layout:
    /// `[count | FLAG][total_size][count ranges][spare slots][reserve word]`
    /// where `total_size` holds the original slot-table size
    /// `(count + spare) * 8` (the fixed anchor for the reserve word
    /// position) and the ranges are payload-relative (base 0).
    ///
    /// The data gap sits in the postponed section right after the field's
    /// last value; the new value lands there (alignment-aware, padding
    /// charged to the reserve), a new `ByRefRange` is linked from the first
    /// spare slot, and the count is the visibility switch.
    ///
    /// Crash-consistency: value bytes and the new range are written before
    /// the count switch — readers with the old count never see them, readers
    /// with the new count see fully written data, so there is no torn window
    /// beyond the count atomicity itself. The reserve word is patched last
    /// (bookkeeping for the next append only); a crash before it self-heals
    /// because the gap position is derived from the stale word.
    pub fn append_byref_vec<R: MmapLayout, T: MmapWrite>(
        &mut self,
        block_off: usize,
        value: &T,
    ) -> io::Result<()> {
        let payload_len = self.payload_range.len();
        if !block_off.is_multiple_of(8) || block_off + 16 > payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "byref-vec block offset {block_off} is misaligned or too close to the payload end ({payload_len})"
                ),
            ));
        }
        let data = self.payload();
        let raw = u64::from_le_bytes(data[block_off..block_off + 8].try_into().unwrap());
        if raw & crate::traits::BLOCK_FLAG_RESERVED == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "byref-vec block has no reserve; rewrite the file with a ReservePlan (root-level non-dedup byref field)",
            ));
        }
        let count = (raw & !crate::traits::BLOCK_FLAG_RESERVED) as usize;
        let total_size =
            u64::from_le_bytes(data[block_off + 8..block_off + 16].try_into().unwrap()) as usize;
        if !total_size.is_multiple_of(8) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("byref-vec total_size {total_size} is not a multiple of 8"),
            ));
        }
        let orig_total = total_size / 8; // original count + spare slots
        self.layout_check::<R>(block_off)?;
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "empty byref vec has no data anchor to locate the reserve gap; write at least one element",
            ));
        }
        // Reserve word sits at the fixed original slot-table end.
        let resv_pos = block_off + 16 + orig_total * 8;
        if resv_pos + 8 > payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "byref-vec reserve word is outside the payload; inconsistent block",
            ));
        }
        let resv = u64::from_le_bytes(data[resv_pos..resv_pos + 8].try_into().unwrap());
        let spare = (resv & 0xFFFF_FFFF) as usize;
        let gap_bytes = (resv >> 32) as usize;
        if orig_total != count + spare {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "byref-vec reserve accounting is inconsistent: original slots {orig_total}, count {count}, spare {spare}"
                ),
            ));
        }
        if spare == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "byref-vec reserve exhausted: no spare table slots left; rewrite the file with a larger reserve",
            ));
        }
        // Data gap anchor: the end of the field's last value (ranges are
        // payload-relative; guaranteed by the write-side base == 0 rule).
        let last_pos = block_off + 16 + (count - 1) * 8;
        let last_br: ByRefRange =
            unsafe { crate::traits::mmap_read_pod(&data[last_pos..last_pos + 8]) };
        let last_end = last_br.end as usize;
        if last_end + gap_bytes > payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "byref-vec reserve gap extends beyond the payload; inconsistent block",
            ));
        }
        // Serialize the element and land it at its natural alignment inside
        // the gap (padding charged to the reserve, like append_indexed).
        let mut ctx = WriteContext::new();
        value.write(&mut ctx);
        let vbytes = ctx.finish();
        let vlen = vbytes.len();
        let align = std::mem::align_of::<T>().max(1);
        let gap_start = if align > 1 {
            (last_end + align - 1) & !(align - 1)
        } else {
            last_end
        };
        let consumed = (gap_start - last_end) + vlen;
        if consumed > gap_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "byref-vec data reserve exhausted: gap holds {gap_bytes} bytes, need {consumed}; rewrite the file with a larger reserve"
                ),
            ));
        }
        if gap_start + vlen > u32::MAX as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "byref range offsets exceed the u32 limit of ByRefRange",
            ));
        }
        {
            let p = self.payload_mut();
            p[gap_start..gap_start + vlen].copy_from_slice(&vbytes);
            let slot_pos = block_off + 16 + count * 8;
            let new_br = ByRefRange {
                begin: gap_start as u32,
                end: (gap_start + vlen) as u32,
            };
            p[slot_pos..slot_pos + 8].copy_from_slice(bytemuck::bytes_of(&new_br));
        }
        // Count is the visibility switch; reserve bookkeeping follows.
        self.patch_count_u64(block_off, raw, (count + 1) as u64)?;
        self.patch_u64_at(
            resv_pos,
            (((gap_bytes - consumed) as u64) << 32) | ((spare - 1) as u64),
        )?;
        Ok(())
    }

    /// Append one element to a `List<T>` block.
    ///
    /// `block_off` is the payload-relative offset of the list block
    /// (`[head][tail][count]`): use `ListReader::block_range().start`.
    ///
    /// The list block may sit anywhere in the file: chains are explicit, so
    /// the new node is appended at the very end of the payload (after any
    /// byref data) and linked in — nothing existing moves. When the chain
    /// currently ends at the payload end (the list is the root's last inline
    /// dynamic value) the offset-table sentinel is grown as well; for
    /// mid-file lists the sentinel stays untouched (the node lives outside
    /// the inline dynamic region).
    ///
    /// Crash-consistency: the new node is written first (orphaned until
    /// linked), then the chain link (`old_tail.next`, or `head` for an empty
    /// list — a single 8-byte aligned store) becomes the visibility switch,
    /// then `tail` and the count are patched. Readers iterate count-bounded,
    /// so every torn state reads as the old or the new list; a crash may
    /// leave a phantom element that the next append self-heals (it re-patches
    /// from the stale tail).
    pub fn append_list<R: MmapLayout, T: MmapWrite>(
        &mut self,
        block_off: usize,
        value: &T,
    ) -> io::Result<()> {
        let payload_len = self.payload_range.len();
        if !block_off.is_multiple_of(8) || block_off + LIST_BLOCK_SIZE > payload_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "list block offset {block_off} is misaligned or its {LIST_BLOCK_SIZE}-byte block exceeds the payload ({payload_len})"
                ),
            ));
        }
        // head/tail are node offsets RELATIVE to the block start.
        let data = self.payload();
        let head_rel = u64::from_le_bytes(data[block_off..block_off + 8].try_into().unwrap());
        let tail_rel = u64::from_le_bytes(data[block_off + 8..block_off + 16].try_into().unwrap());
        let count = u64::from_le_bytes(data[block_off + 16..block_off + 24].try_into().unwrap());
        if head_rel == 0 && tail_rel != 0 || head_rel != 0 && tail_rel == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "list head/tail pointers are inconsistent (one is zero, the other is not)",
            ));
        }
        // Where does the whole field span end?
        let chain_end = if head_rel == 0 {
            block_off + LIST_BLOCK_SIZE
        } else {
            if tail_rel < LIST_BLOCK_SIZE as u64
                || !tail_rel.is_multiple_of(8)
                || block_off + tail_rel as usize + LIST_NODE_HEADER > payload_len
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "list tail pointer (block-relative {tail_rel}) does not point at a plausible node"
                    ),
                ));
            }
            let tail_pos = block_off + tail_rel as usize;
            let tail_len =
                u32::from_le_bytes(data[tail_pos + 8..tail_pos + 12].try_into().unwrap()) as usize;
            tail_pos + LIST_NODE_HEADER + tail_len
        };
        let (sentinel_pos, sentinel) = self.layout_check::<R>(block_off)?;
        let tail_case = chain_end == payload_len;
        // Serialize the element as a self-contained node payload.
        let mut ctx = WriteContext::new();
        value.write(&mut ctx);
        let vbytes = ctx.finish();
        let vlen = vbytes.len();
        let vlen32 = u32::try_from(vlen).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("list node payload too large: {vlen} bytes (u32 limit)"),
            )
        })?;
        let node_size = LIST_NODE_HEADER + vlen;
        let node_base = (payload_len + 7) & !7;
        let pad = node_base - payload_len;
        self.grow(pad + node_size)?;
        {
            let p = self.payload_mut();
            let node = p.len() - node_size; // node base (8-aligned); fresh bytes are zero
            let len_pos = node + 8;
            p[len_pos..len_pos + 4].copy_from_slice(&vlen32.to_le_bytes());
            p[node + LIST_NODE_HEADER..].copy_from_slice(&vbytes);
        }
        let node_rel = (node_base - block_off) as u64;
        if head_rel == 0 {
            // Empty list: the head patch is the visibility switch.
            self.patch_u64_at(block_off, node_rel)?;
            self.patch_u64_at(block_off + 8, node_rel)?;
        } else {
            // Non-empty: the old tail node's next-pointer is the switch.
            let tail_pos = block_off + tail_rel as usize;
            self.patch_u64_at(tail_pos, node_rel)?;
            self.patch_u64_at(block_off + 8, node_rel)?;
        }
        if tail_case {
            // The list is the root's last inline dynamic value: its span
            // (and the offset-table sentinel) grows with the new node.
            self.patch_u64_at(sentinel_pos, sentinel + (pad + node_size) as u64)?;
        }
        self.patch_count_u64(block_off + 16, count, count + 1)?;
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
