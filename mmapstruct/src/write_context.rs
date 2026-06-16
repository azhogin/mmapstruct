use crate::traits::{MmapWrite, ByRefRange};
use bytemuck::Pod;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

pub trait PostponedWriter<'data> {
    fn write(&self, ctx: &mut WriteContext<'data>);
}

struct ByRefWriter<'data, T: MmapWrite> {
    value: &'data T,
}

impl<'data, T: MmapWrite> PostponedWriter<'data> for ByRefWriter<'data, T> {
    fn write(&self, ctx: &mut WriteContext<'data>) {
        self.value.write(ctx);
    }
}

struct PostponedData<'data> {
    base: usize,
    placeholder_offset: usize,
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
}

impl<'data> WriteContext<'data> {
    pub fn new() -> Self {
        Self {
            buf: Vec::new(),
            postponed: Vec::new(),
        }
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
        assert!(s.len() <= size, "inline string too long: {} > {}", s.len(), size);
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
            placeholder_offset,
            dedup,
            writer: Box::new(ByRefWriter { value }),
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
                let br = ByRefRange {
                    begin: abs_begin - pd.base as u32,
                    end: abs_end - pd.base as u32,
                };
                self.write_pod_at(&br, pd.placeholder_offset);
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
            entries.push(Entry { bytes, hash: hasher.finish() });
        }

        // Group by hash
        let mut hash_to_indices: HashMap<u64, Vec<usize>> = HashMap::new();
        for i in 0..dedup_items.len() {
            hash_to_indices.entry(entries[i].hash).or_default().push(i);
        }

        // Write unique values, backpatch all placeholders
        for (_hash, indices) in &hash_to_indices {
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
                self.write_pod_at(&br, pd.placeholder_offset);
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

#[cfg(not(feature = "strict-validation"))]
fn check_write_bounds(offset: usize, size: usize, buf_len: usize) {
    debug_assert!(offset + size <= buf_len, "write bounds: offset {} + size {} > buf len {}", offset, size, buf_len);
}

#[cfg(feature = "strict-validation")]
fn check_write_bounds(offset: usize, size: usize, buf_len: usize) {
    assert!(offset + size <= buf_len, "write bounds: offset {} + size {} > buf len {}", offset, size, buf_len);
}
