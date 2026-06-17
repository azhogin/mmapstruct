//! Schema registry: stores/loads serialized schemas by `layout_hash`.
//!
//! Schemas are stored in `~/.config/mmapstruct/schemas/<hash_hex>.mms`.
//! The registry is a fallback for files without embedded schema.

use std::path::PathBuf;

fn registry_dir() -> PathBuf {
    let base = std::env::var("MMAPSTRUCT_SCHEMA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".config/mmapstruct/schemas")
        });
    base
}

/// Save a serialized schema to the registry, keyed by `layout_hash`.
pub fn save_schema(layout_hash: u64, schema_bytes: &[u8]) -> std::io::Result<()> {
    let dir = registry_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{:016x}.mms", layout_hash));
    std::fs::write(path, schema_bytes)
}

/// Load a serialized schema from the registry by `layout_hash`.
/// Returns `None` if not found.
pub fn load_schema(layout_hash: u64) -> Option<Vec<u8>> {
    let path = registry_dir().join(format!("{:016x}.mms", layout_hash));
    std::fs::read(path).ok()
}
