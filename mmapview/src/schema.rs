//! Schema resolution: embedded schema extraction, registry lookup, and
//! payload range computation.
//!
//! This module is shared between `mmapview` (F3 tree) and `mmapview-tui` (F4).

/// Result of loading a data file: serialized schema bytes and the payload
/// byte range.
#[derive(Clone)]
pub struct SchemaResult {
    pub schema_bytes: Vec<u8>,
    pub payload_range: std::ops::Range<usize>,
}

/// Resolve schema from a data buffer.
///
/// Tries in order:
/// 1. Embedded schema in the v2 file header
/// 2. Registry lookup by `layout_hash` (from file header)
///
/// Returns an error if neither is found.
pub fn resolve_schema(data: &[u8]) -> Result<SchemaResult, String> {
    let (payload, embedded, hash, _crc) = mmapstruct::file_reader::parse_header(data);
    let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
    let payload_range = payload_start..payload_start + payload.len();

    // 1. Try embedded schema (v2 header)
    if let Some((start, end)) = embedded {
        return Ok(SchemaResult {
            schema_bytes: data[start..end].to_vec(),
            payload_range,
        });
    }

    // 2. Try registry lookup by layout hash
    if let Some(h) = hash {
        if let Some(s) = crate::registry::load_schema(h) {
            return Ok(SchemaResult {
                schema_bytes: s,
                payload_range,
            });
        }
    }

    Err(
        "No embedded schema found and no schema in registry.\n\
         Write the file with write_to_file_with_embedded_schema() or\n\
         save the schema to ~/.config/mmapstruct/schemas/<layout_hash>.mms.\n\
         Use --schema <file> to specify a schema explicitly."
            .to_string(),
    )
}
