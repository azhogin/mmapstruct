//! # mmapstruct — Memory-Mapped Struct Library
//!
//! Zero-copy serialization/deserialization of Rust structs via memory-mapped files.
//!
//! ## Feature flags
//!
//! All safety checks are opt-in via Cargo features for maximum performance
//! in the default configuration.
//!
//! - **`safe-strings`** — Replaces `unsafe { str::from_utf8_unchecked() }` with
//!   validated `str::from_utf8()`. Invalid UTF-8 is truncated at the first
//!   valid boundary (with an `eprintln!` warning in debug builds). Default: **off**.
//!
//! - **`strict-validation`** (implies `safe-strings`) — Enables runtime bounds
//!   checking in readers, writers, and range calculations. Without this flag,
//!   only `debug_assert!` is used. Default: **off**.
//!
//! ## Performance / safety trade-offs
//!
//! | Configuration | String reads | Range checks | Write bounds |
//! |---|---|---|---|
//! | `default` | `unsafe from_utf8_unchecked` | `debug_assert` | `debug_assert` |
//! | `safe-strings` | validated UTF-8 + truncation | `debug_assert` | `debug_assert` |
//! | `strict-validation` | validated UTF-8 + truncation | `assert` on all ranges | `assert` on backpatching |
//!
//! ## Recommended usage
//!
//! - **Development / testing**: `cargo test --features strict-validation`
//! - **Production**: default features (zero overhead)
//! - **Trusted input with safety margin**: `--features safe-strings`

pub mod byref_array;
pub mod file_reader;
pub mod fixed_array;
pub mod hash_map;
pub mod indexed_array;
pub mod option;
pub mod pod;
pub mod pod_array;
pub mod reader;
pub mod string;
pub mod traits;
pub mod write_context;

#[cfg(feature = "dyn")]
pub mod schema;

pub use bytemuck;
pub use file_reader::{BufReader, MmapReader};
pub use mmapstruct_derive::MmapStruct;
#[cfg(feature = "dyn")]
pub use schema::{
    ArrayLayout, ArraySchema, DynContext, DynValueType, FieldDescriptor, GetSchema, StructSchema,
    Value,
};
pub use traits::{
    combine_hashes, compute_crc64, fnv1a, mmap_read_pod, ByRefRange, FileHeader, FileRange,
    LayoutHash, MmapPod, MmapRead, MmapWrite, ReaderOrd, CRC64_TABLE, HEADER_SIZE, MMAP_MAGIC,
};
pub use write_context::WriteContext;

use std::path::Path;

pub fn write_to_file<T: traits::MmapWrite + LayoutHash>(
    value: &T,
    path: impl AsRef<Path>,
    write_crc: bool,
) -> std::io::Result<()> {
    let buf = write_to_buf(value, write_crc);
    std::fs::write(path, &buf)
}

/// Serialize `value` with a 24-byte file header (magic + flags + layout_hash + crc64).
/// If `write_crc` is true, computes and writes CRC-64-ECMA over the payload.
/// If `write_crc` is false, the crc64 field is set to 0 (reader will skip CRC check).
pub fn write_to_buf<T: traits::MmapWrite + LayoutHash>(value: &T, write_crc: bool) -> Vec<u8> {
    let mut ctx = WriteContext::new();
    value.write(&mut ctx);
    let buf = ctx.finish();
    let mut result = Vec::with_capacity(HEADER_SIZE + buf.len());
    result.extend_from_slice(bytemuck::bytes_of(&FileHeader::new::<T>()));
    result.extend_from_slice(&buf);
    if write_crc {
        let crc = compute_crc64(&result[HEADER_SIZE..]);
        result[16..24].copy_from_slice(&crc.to_le_bytes());
    }
    result
}

/// Serialize `value` **without** a file header. For tests / raw data.
pub fn write_to_buf_raw<T: traits::MmapWrite>(value: &T) -> Vec<u8> {
    let mut ctx = WriteContext::new();
    value.write(&mut ctx);
    ctx.finish()
}

/// Trim trailing null bytes from a string (for inline strings with padding).
pub fn fixup_str(v: &str) -> &str {
    if let Some(pos) = v.find('\0') {
        &v[..pos]
    } else {
        v
    }
}
