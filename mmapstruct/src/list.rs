//! `List<T>` — an append-only sequence container for mmapstruct files.
//!
//! Unlike `Vec<T>`, a `List<T>` can grow in place via
//! [`crate::PatchSession::append_list`] without reserving space: the nodes
//! live at explicit absolute offsets, so appending is a pure tail write.
//! Trade-offs vs `Vec<T>`: elements are node-linked (random access is O(n)),
//! and each node carries a 16-byte header.
//!
//! # File format
//!
//! The field's dynamic slot holds a 24-byte block:
//! `[head: u64][tail: u64][count: u64]` — offsets of the first/last node
//! **relative to the block start** (0 = empty list) and the element count.
//! Block-relative offsets keep the format base-independent: readers may map
//! the file with any header/schema prefix (the mmapview-tui passes the whole
//! file, the CLI passes the bare payload).
//!
//! Nodes follow the block (and may share the tail region with byref data of
//! other fields; chains are explicit, so interleaving is safe). Each node is
//! 8-byte aligned and laid out as
//! `[next: u64][len: u32][pad: u32][payload: len bytes]`, where `payload` is
//! a self-contained serialized `T` instance and `next` is the absolute
//! payload offset of the following node (0 terminates the chain).
//!
//! # Crash consistency
//!
//! Append protocol: node bytes → chain/tail link → count. The count is the
//! visibility switch (updated as two 4-byte halves); readers iterate
//! count-bounded, so a torn state always yields the old or the new view —
//! at worst a phantom element hides until the next successful append
//! re-patches the head.

use crate::traits::{FileRange, MmapRead, MmapWrite};
use crate::write_context::WriteContext;
use std::marker::PhantomData;

/// Size of the list block in the dynamic slot: `[head][tail][count]`.
pub const LIST_BLOCK_SIZE: usize = 24;
/// Size of a node header: `[next: u64][len: u32][pad: u32]`.
pub const LIST_NODE_HEADER: usize = 16;

/// RAM-side collection marking a field as a file-format `List`.
///
/// The derive maps a `List<T>` field to the linked-node file layout and a
/// [`ListReader`] view; the owned struct keeps this plain vector wrapper.
#[derive(Debug)]
pub struct List<T> {
    items: Vec<T>,
}

impl<T> List<T> {
    pub fn new() -> Self {
        Self { items: Vec::new() }
    }

    pub fn from_vec(items: Vec<T>) -> Self {
        Self { items }
    }

    pub fn push(&mut self, value: T) {
        self.items.push(value);
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items
    }
}

impl<T> Default for List<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> From<Vec<T>> for List<T> {
    fn from(items: Vec<T>) -> Self {
        Self { items }
    }
}

impl<T> FromIterator<T> for List<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self {
            items: iter.into_iter().collect(),
        }
    }
}

impl<T> IntoIterator for List<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<T> std::ops::Deref for List<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.items
    }
}

impl<T: MmapWrite> MmapWrite for List<T> {
    fn write<'data>(&'data self, ctx: &mut WriteContext<'data>) {
        ctx.align_to(8);
        let block_pos = ctx.current_offset();
        ctx.write_padding(LIST_BLOCK_SIZE);
        let mut head: u64 = 0;
        let mut tail: u64 = 0;
        let mut prev_pos: Option<usize> = None;
        for item in &self.items {
            ctx.align_to(8);
            let node_pos = ctx.current_offset();
            let node_rel = (node_pos - block_pos) as u64;
            match prev_pos {
                Some(prev) => ctx.write_u64_le_at(node_rel, prev),
                None => head = node_rel,
            }
            ctx.write_u64_le(0);
            let len_pos = ctx.current_offset();
            ctx.write_padding(8);
            let payload_start = ctx.current_offset();
            item.write(ctx);
            let payload_len = ctx.current_offset() - payload_start;
            let len32 = u32::try_from(payload_len).unwrap_or_else(|_| {
                panic!("list node payload too large: {payload_len} bytes (u32 limit)")
            });
            ctx.write_u32_le_at(len32, len_pos);
            prev_pos = Some(node_pos);
            tail = node_rel;
        }
        ctx.write_u64_le_at(head, block_pos);
        ctx.write_u64_le_at(tail, block_pos + 8);
        ctx.write_u64_le_at(self.items.len() as u64, block_pos + 16);
    }
}

/// Typed read view over a `List<T>` field: `[head][tail][count]` block plus
/// the linked node chain at absolute payload offsets.
#[derive(Clone)]
pub struct ListReader<'a, T: MmapRead> {
    data: &'a [u8],
    /// File-relative range of the whole field span (block + nodes).
    range: FileRange,
    head_rel: u64,
    count: u64,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> ListReader<'a, T> {
    pub fn new(data: &'a [u8], range: FileRange) -> Self {
        #[cfg(feature = "strict-validation")]
        {
            assert!(
                range.len() >= LIST_BLOCK_SIZE,
                "ListReader: block range len {} < {LIST_BLOCK_SIZE}",
                range.len()
            );
            assert!(
                range.start + LIST_BLOCK_SIZE <= data.len(),
                "ListReader: block range {:?} exceeds data len {}",
                range,
                data.len()
            );
        }
        debug_assert!(
            range.len() >= LIST_BLOCK_SIZE,
            "ListReader: block range len {} < {LIST_BLOCK_SIZE}",
            range.len()
        );
        let head_rel = u64::from_le_bytes(data[range.start..range.start + 8].try_into().unwrap());
        let count =
            u64::from_le_bytes(data[range.start + 16..range.start + 24].try_into().unwrap());
        Self {
            data,
            range,
            head_rel,
            count,
            _phantom: PhantomData,
        }
    }

    /// File-relative range of the whole field span (block + nodes).
    /// `start` is the position of the `[head][tail][count]` block — the
    /// anchor for append-style patching.
    pub fn block_range(&self) -> FileRange {
        self.range
    }

    pub fn len(&self) -> usize {
        self.count as usize
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Data-relative offset of the element with `index`'s node (O(index)
    /// walk). Node offsets are stored relative to the block; this returns
    /// the absolute position within `data`.
    pub fn node_offset(&self, index: usize) -> Option<usize> {
        if index >= self.len() {
            return None;
        }
        let mut cur = self.head_rel;
        for _ in 0..index {
            cur = self.next_of(cur)?;
        }
        if cur == 0 {
            return None;
        }
        Some(self.block_base() + cur as usize)
    }

    /// Read the element with `index` (O(index) walk).
    pub fn get(&self, index: usize) -> Option<T::Reader<'a>> {
        let node = self.node_offset(index)?;
        Some(self.read_node(node))
    }

    fn block_base(&self) -> usize {
        self.range.start
    }

    fn next_of(&self, rel: u64) -> Option<u64> {
        if rel == 0 {
            return None;
        }
        let p = self.block_base() + rel as usize;
        if p + 8 > self.data.len() {
            return None;
        }
        Some(u64::from_le_bytes(self.data[p..p + 8].try_into().unwrap()))
    }

    fn read_node(&self, node: usize) -> T::Reader<'a> {
        let len = u32::from_le_bytes(self.data[node + 8..node + 12].try_into().unwrap()) as usize;
        let start = node + LIST_NODE_HEADER;
        let end = start + len;
        #[cfg(feature = "strict-validation")]
        {
            assert!(
                end <= self.data.len(),
                "ListReader: node payload [{start}, {end}) exceeds data len {}",
                self.data.len()
            );
        }
        debug_assert!(
            end <= self.data.len(),
            "ListReader: node payload [{start}, {end}) exceeds data len {}",
            self.data.len()
        );
        T::read(self.data, start..end)
    }

    /// Count-bounded iterator over the chain (consistent with [`Self::len`];
    /// a chain longer than the count is truncated, a chain ending early
    /// stops the iteration).
    pub fn iter(&self) -> ListIter<'a, T> {
        ListIter {
            data: self.data,
            block_base: self.block_base(),
            cur: self.head_rel,
            remaining: self.count,
            _phantom: PhantomData,
        }
    }
}

impl<'a, T: MmapRead + 'a> IntoIterator for &ListReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = ListIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T: MmapRead + 'a> IntoIterator for ListReader<'a, T> {
    type Item = T::Reader<'a>;
    type IntoIter = ListIter<'a, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, T> From<ListReader<'a, T>> for List<T>
where
    T: MmapRead + 'a,
    <T as MmapRead>::Reader<'a>: Into<T>,
{
    fn from(r: ListReader<'a, T>) -> Self {
        List::from_vec(r.iter().map(|e| e.into()).collect())
    }
}

pub struct ListIter<'a, T: MmapRead> {
    data: &'a [u8],
    block_base: usize,
    cur: u64,
    remaining: u64,
    _phantom: PhantomData<T>,
}

impl<'a, T: MmapRead + 'a> Iterator for ListIter<'a, T> {
    type Item = T::Reader<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 || self.cur == 0 {
            return None;
        }
        let node = self.block_base + self.cur as usize;
        if node + LIST_NODE_HEADER > self.data.len() {
            self.remaining = 0;
            return None;
        }
        let len = u32::from_le_bytes(self.data[node + 8..node + 12].try_into().unwrap()) as usize;
        let payload_start = node + LIST_NODE_HEADER;
        let payload_end = payload_start + len;
        #[cfg(feature = "strict-validation")]
        {
            assert!(
                payload_end <= self.data.len(),
                "ListIter: node payload [{payload_start}, {payload_end}) exceeds data len {}",
                self.data.len()
            );
        }
        debug_assert!(
            payload_end <= self.data.len(),
            "ListIter: node payload [{payload_start}, {payload_end}) exceeds data len {}",
            self.data.len()
        );
        self.cur = u64::from_le_bytes(self.data[node..node + 8].try_into().unwrap());
        self.remaining -= 1;
        Some(T::read(self.data, payload_start..payload_end))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let rem = self.remaining as usize;
        (rem, Some(rem))
    }
}
