use crate::traits::{ByRefRange, MmapWrite};
use bytemuck::Pod;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

pub trait PostponedWriter<'data> {
    fn write(&self, ctx: &mut WriteContext<'data>);
}

/// Per-field reserve amounts baked into a container block at write time.
///
/// `slots` — spare index/range-table entries appended to the table;
/// `bytes` — spare data bytes placed after the field's values. Both make the
/// field patchable in place later (see `crate::PatchSession`). Reserves are
/// self-describing in the file (`BLOCK_FLAG_RESERVED` + a reserve word), so
/// readers never need the plan; it is only a write-time input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Reserve {
    pub slots: u32,
    pub bytes: u64,
}

impl Reserve {
    pub const NONE: Reserve = Reserve { slots: 0, bytes: 0 };

    pub fn is_none(&self) -> bool {
        self.slots == 0 && self.bytes == 0
    }
}

/// Field-name keyed reserve plan for a write.
#[derive(Clone, Debug, Default)]
pub struct ReservePlan {
    map: std::collections::HashMap<String, Reserve>,
}

impl ReservePlan {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reserve slack for the field with the given (Rust) name.
    pub fn set(&mut self, field: impl Into<String>, reserve: Reserve) -> &mut Self {
        self.map.insert(field.into(), reserve);
        self
    }

    pub fn get(&self, field: &str) -> Reserve {
        self.map.get(field).copied().unwrap_or(Reserve::NONE)
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

struct ByRefWriter<'data, T: MmapWrite> {
    value: &'data T,
}

impl<'data, T: MmapWrite> PostponedWriter<'data> for ByRefWriter<'data, T> {
    fn write(&self, ctx: &mut WriteContext<'data>) {
        self.value.write(ctx);
    }
}

/// Reserved run queued after a field's postponed data — the patchable gap
/// for future byref appends/sets (no ByRefRange to backpatch). `marked`
/// gaps start with a `{BYREF_GAP_MAGIC, usable}` marker pair so the patch
/// session can size the free run from the field's range end alone
/// (standalone byref set); unmarked gaps are plain zero runs (byref-vec
/// append, anchored by the block's reserve word instead).
struct GapWriter {
    bytes: usize,
    marked: bool,
}

impl<'data> PostponedWriter<'data> for GapWriter {
    fn write(&self, ctx: &mut WriteContext<'data>) {
        if self.marked {
            assert!(
                self.bytes >= 16,
                "mmapstruct: standalone byref reserve must be at least 16 bytes (gap marker overhead)"
            );
            ctx.write_u64_le(crate::traits::BYREF_GAP_MAGIC);
            ctx.write_u64_le((self.bytes - 16) as u64);
            ctx.write_padding(self.bytes - 16);
        } else {
            ctx.write_padding(self.bytes);
        }
    }
}

struct PostponedData<'data> {
    base: usize,
    /// `Some` for byref values (a ByRefRange placeholder to backpatch),
    /// `None` for raw gap runs.
    placeholder_offset: Option<usize>,
    dedup: bool,
    writer: Box<dyn PostponedWriter<'data> + 'data>,
}

impl<'data> PostponedData<'data> {
    fn serialize_to_vec(&self) -> Vec<u8> {
        let mut tmp = WriteContext::new();
        self.writer.write(&mut tmp);
        tmp.finish()
    }
}

pub struct WriteContext<'data> {
    buf: Vec<u8>,
    postponed: Vec<PostponedData<'data>>,
    reserve_plan: Option<ReservePlan>,
}

impl<'data> Default for WriteContext<'data> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'data> WriteContext<'data> {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            postponed: Vec::new(),
            reserve_plan: None,
        }
    }

    /// Attach a reserve plan: container helpers query it by field name and
    /// bake the requested slack (flag + reserve word + spare slots + gap)
    /// into the file.
    pub fn with_reserve_plan(plan: ReservePlan) -> Self {
        Self {
            buf: Vec::new(),
            postponed: Vec::new(),
            reserve_plan: Some(plan),
        }
    }

    /// Look up the reserve amounts for a field (NONE when no plan or no
    /// entry).
    pub fn field_reserve(&self, field: &str) -> Reserve {
        self.reserve_plan
            .as_ref()
            .map(|p| p.get(field))
            .unwrap_or(Reserve::NONE)
    }

    pub fn current_offset(&self) -> usize {
        self.buf.len()
    }

    pub fn offset_u32(&self) -> u32 {
        self.buf.len() as u32
    }

    /// Write a Pod value (appends to buffer).
    pub fn write_pod<T: Pod>(&mut self, value: &T) {
        self.buf.extend_from_slice(bytemuck::bytes_of(value));
    }

    /// Write a Pod value at a specific (already allocated) position.
    pub fn write_pod_at<T: Pod>(&mut self, value: &T, offset: usize) {
        let size = std::mem::size_of::<T>();
        check_write_bounds(offset, size, self.buf.len());
        self.buf[offset..offset + size].copy_from_slice(bytemuck::bytes_of(value));
    }

    /// Write u64 as little-endian.
    pub fn write_u64_le(&mut self, value: u64) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Write u32 as little-endian.
    pub fn write_u32_le(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_le_bytes());
    }

    /// Write u32 as little-endian at a specific offset.
    pub fn write_u32_le_at(&mut self, value: u32, offset: usize) {
        check_write_bounds(offset, 4, self.buf.len());
        self.buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Write u64 as little-endian at a specific offset.
    pub fn write_u64_le_at(&mut self, value: u64, offset: usize) {
        check_write_bounds(offset, 8, self.buf.len());
        self.buf[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    /// Write raw bytes, advancing the offset.
    pub fn write_raw(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Write a string as a fixed-size inline field, null-padded to `size`.
    pub fn write_inline_string(&mut self, s: &str, size: usize) {
        assert!(
            s.len() <= size,
            "inline string too long: {} > {}",
            s.len(),
            size
        );
        self.buf.extend_from_slice(s.as_bytes());
        let pad = size - s.len();
        if pad > 0 {
            self.buf.resize(self.buf.len() + pad, 0);
        }
    }

    /// Write a single byte.
    pub fn write_u8(&mut self, byte: u8) {
        self.buf.push(byte);
    }

    /// Write count bytes of zeros.
    pub fn write_padding(&mut self, count: usize) {
        self.buf.resize(self.buf.len() + count, 0);
    }

    /// Pad to align from buffer start.
    pub fn align_to(&mut self, align: usize) {
        let offset = self.buf.len() % align;
        if offset != 0 {
            self.write_padding(align - offset);
        }
    }

    /// Pad to align relative to a base position.
    pub fn align_from(&mut self, base: usize, align: usize) {
        let offset = (self.buf.len() - base) % align;
        if offset != 0 {
            self.write_padding(align - offset);
        }
    }

    /// Register a byref field for postponed writing.
    /// When `dedup` is true, identical values are written only once.
    pub fn write_byref<T: MmapWrite>(&mut self, value: &'data T, base: usize, dedup: bool) {
        let placeholder_offset = self.buf.len();
        self.write_padding(8);
        self.postponed.push(PostponedData {
            base,
            placeholder_offset: Some(placeholder_offset),
            dedup,
            writer: Box::new(ByRefWriter { value }),
        });
    }

    /// Queue `bytes` of zero padding after the field's postponed data — the
    /// gap a patch session appends byref values into. The run carries no
    /// ByRefRange placeholder and is written in registration order (non-dedup).
    pub fn queue_postponed_gap(&mut self, bytes: u64) {
        self.postponed.push(PostponedData {
            base: 0,
            placeholder_offset: None,
            dedup: false,
            writer: Box::new(GapWriter {
                bytes: bytes as usize,
                marked: false,
            }),
        });
    }

    /// [`Self::queue_postponed_gap`] prefixed with a
    /// `{BYREF_GAP_MAGIC, usable}` marker: the standalone-byref set op finds
    /// the free run from the field's range end without any block metadata.
    pub fn queue_postponed_gap_marked(&mut self, bytes: u64) {
        self.postponed.push(PostponedData {
            base: 0,
            placeholder_offset: None,
            dedup: false,
            writer: Box::new(GapWriter {
                bytes: bytes as usize,
                marked: true,
            }),
        });
    }

    /// Execute all postponed writes and return the buffer.
    /// Entries with `dedup: true` are deduplicated by content hash.
    /// Save current postponed boundary for a scoped section.
    pub fn scope_begin(&self) -> usize {
        self.postponed.len()
    }

    /// Finalize all postponed entries since `saved`, writing them at the current buffer position.
    /// This enables per-section byref data locality with local dedup.
    pub fn scope_end(&mut self, saved: usize) {
        if saved >= self.postponed.len() {
            return;
        }
        let scope_postponed: Vec<PostponedData<'data>> = self.postponed.drain(saved..).collect();
        self.process_postponed(scope_postponed);
    }

    /// Process a list of postponed entries: non-dedup written immediately, dedup grouped by hash.
    fn process_postponed(&mut self, postponed: Vec<PostponedData<'data>>) {
        if postponed.is_empty() {
            return;
        }

        // Partition: non-dedup written immediately, dedup grouped by hash
        let mut dedup_items: Vec<PostponedData<'data>> = Vec::new();

        for pd in postponed {
            if pd.dedup {
                dedup_items.push(pd);
            } else {
                let abs_begin = self.buf.len() as u32;
                pd.writer.write(self);
                let abs_end = self.buf.len() as u32;
                if let Some(placeholder_offset) = pd.placeholder_offset {
                    let br = ByRefRange {
                        begin: abs_begin - pd.base as u32,
                        end: abs_end - pd.base as u32,
                    };
                    self.write_pod_at(&br, placeholder_offset);
                }
            }
        }

        // Dedup phase: serialize, hash, group, write unique, backpatch
        if dedup_items.is_empty() {
            return;
        }

        // Serialize each dedup entry and compute hash
        struct Entry {
            bytes: Vec<u8>,
            hash: u64,
        }
        let mut entries: Vec<Entry> = Vec::with_capacity(dedup_items.len());
        for pd in &dedup_items {
            let bytes = pd.serialize_to_vec();
            let mut hasher = DefaultHasher::new();
            bytes.hash(&mut hasher);
            entries.push(Entry {
                bytes,
                hash: hasher.finish(),
            });
        }

        // Group by hash
        let mut hash_to_indices: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            hash_to_indices.entry(e.hash).or_default().push(i);
        }

        // Write unique values, backpatch all placeholders
        for indices in hash_to_indices.values() {
            let leader = indices[0];
            let abs_begin = self.buf.len() as u32;
            self.buf.extend_from_slice(&entries[leader].bytes);
            let abs_end = self.buf.len() as u32;

            for &idx in indices {
                let pd = &dedup_items[idx];
                let br = ByRefRange {
                    begin: abs_begin - pd.base as u32,
                    end: abs_end - pd.base as u32,
                };
                if let Some(placeholder_offset) = pd.placeholder_offset {
                    self.write_pod_at(&br, placeholder_offset);
                }
            }
        }
    }

    /// Execute all postponed writes and return the buffer.
    /// Entries with `dedup: true` are deduplicated by content hash.
    pub fn finish(mut self) -> Vec<u8> {
        while !self.postponed.is_empty() {
            let postponed = std::mem::take(&mut self.postponed);
            self.process_postponed(postponed);
        }
        self.buf
    }
}

/// [`WriteContext::write_byref`] with an optional standalone-byref reserve:
/// queues a marked data gap right after the value's postponed data, making
/// the field patchable via `PatchSession::set_byref`.
///
/// Restrictions (fail fast): root-level non-dedup fields only (`base == 0`,
/// `dedup == false` — the patcher must reconstruct the payload-relative
/// range and the gap must precede any dedup group), and `reserve.bytes`
/// must cover the 16-byte gap marker.
/// [`WriteContext::write_byref`] variant for `Option<T>` fields with
/// `#[mmap(byref)]`: `Some` queues the value's postponed data, `None` writes
/// the `{0, 0}` sentinel placeholder. With a reserve, a marked gap is queued
/// after either, so a `Some` value can be replaced in place later
/// (`PatchSession::set_byref`); setting a `None` back to a value is not
/// patchable (the sentinel `{0, 0}` gives no data anchor).
pub fn write_byref_option_r<'data, T: MmapWrite>(
    value: &'data Option<T>,
    ctx: &mut WriteContext<'data>,
    base: usize,
    dedup: bool,
    reserve: Reserve,
) {
    if reserve.is_none() {
        match value {
            Some(v) => ctx.write_byref(v, base, dedup),
            None => ctx.write_pod(&crate::traits::ByRefRange { begin: 0, end: 0 }),
        }
        return;
    }
    if dedup || base != 0 {
        panic!(
            "mmapstruct: standalone byref reserve requires a root-level non-dedup field (base == 0, dedup == false); got base {base}, dedup {dedup}"
        );
    }
    if reserve.bytes < 16 {
        panic!(
            "mmapstruct: standalone byref reserve must be at least 16 bytes (gap marker overhead); got {}",
            reserve.bytes
        );
    }
    match value {
        Some(v) => ctx.write_byref(v, base, dedup),
        None => ctx.write_pod(&crate::traits::ByRefRange { begin: 0, end: 0 }),
    }
    ctx.queue_postponed_gap_marked(reserve.bytes);
}

pub fn write_byref_r<'data, T: MmapWrite>(
    value: &'data T,
    ctx: &mut WriteContext<'data>,
    base: usize,
    dedup: bool,
    reserve: Reserve,
) {
    if reserve.is_none() {
        ctx.write_byref(value, base, dedup);
        return;
    }
    if dedup || base != 0 {
        panic!(
            "mmapstruct: standalone byref reserve requires a root-level non-dedup field (base == 0, dedup == false); got base {base}, dedup {dedup}"
        );
    }
    if reserve.bytes < 16 {
        panic!(
            "mmapstruct: standalone byref reserve must be at least 16 bytes (gap marker overhead); got {}",
            reserve.bytes
        );
    }
    ctx.write_byref(value, base, dedup);
    ctx.queue_postponed_gap_marked(reserve.bytes);
}

#[cfg(not(feature = "strict-validation"))]
fn check_write_bounds(offset: usize, size: usize, buf_len: usize) {
    debug_assert!(
        offset + size <= buf_len,
        "write bounds: offset {} + size {} > buf len {}",
        offset,
        size,
        buf_len
    );
}

#[cfg(feature = "strict-validation")]
fn check_write_bounds(offset: usize, size: usize, buf_len: usize) {
    assert!(
        offset + size <= buf_len,
        "write bounds: offset {} + size {} > buf len {}",
        offset,
        size,
        buf_len
    );
}
