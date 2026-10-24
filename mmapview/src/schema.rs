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

    Err("No embedded schema found and no schema in registry.\n\
         Write the file with write_to_file_with_embedded_schema() or\n\
         save the schema to ~/.config/mmapstruct/schemas/<layout_hash>.mms.\n\
         Use --schema <file> to specify a schema explicitly."
        .to_string())
}

/// Full schema resolution used by all frontends (`mmapview q`, tree, TUI).
///
/// Order:
/// 1. Explicit `--schema <file>` — load schema bytes from the given path
/// 2. Embedded schema in the v2 file header
/// 3. Registry lookup by `layout_hash` (from file header)
/// 4. Sidecar `<data_path>.mmshdr` — its embedded schema, or (if it carries
///    only a `layout_hash`) the registry keyed by that hash.
pub fn load_schema_full(
    data: &[u8],
    schema_path: Option<&str>,
    data_path: Option<&str>,
) -> Result<SchemaResult, String> {
    if let Some(path) = schema_path {
        let raw =
            std::fs::read(path).map_err(|e| format!("Error reading schema '{}': {}", path, e))?;
        // Strip any mmapstruct header — schema file may be raw or v2.
        let schema_bytes = mmapstruct::file_reader::parse_header(&raw).0.to_vec();
        let (payload, _, _, _) = mmapstruct::file_reader::parse_header(data);
        let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
        let payload_range = payload_start..payload_start + payload.len();
        return Ok(SchemaResult {
            schema_bytes,
            payload_range,
        });
    }

    // 2 + 3: embedded / registry
    if let Ok(r) = resolve_schema(data) {
        return Ok(r);
    }

    // 4: sidecar
    if let Some(dp) = data_path {
        let mut sidecar_path = std::path::Path::new(dp).to_path_buf();
        sidecar_path.set_extension("mmshdr");
        if let Ok(sb) = std::fs::read(&sidecar_path) {
            // Payload range comes from the (headerless) data file, not the sidecar.
            let (payload, _, _, _) = mmapstruct::file_reader::parse_header(data);
            let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
            let payload_range = payload_start..payload_start + payload.len();
            let (_, schema_range, hash, _) = mmapstruct::file_reader::parse_header(&sb);
            let schema_bytes = if let Some((s, e)) = schema_range {
                sb[s..e].to_vec()
            } else if let Some(h) = hash {
                crate::registry::load_schema(h).ok_or_else(|| {
                    format!(
                        "sidecar '{}' references layout_hash {:016x} but no schema in registry",
                        sidecar_path.display(),
                        h
                    )
                })?
            } else {
                return Err(format!(
                    "sidecar '{}' has neither embedded schema nor layout_hash",
                    sidecar_path.display()
                ));
            };
            return Ok(SchemaResult {
                schema_bytes,
                payload_range,
            });
        }
    }

    Err(
        "No embedded schema found and no schema in registry or sidecar.\n\
         Write the file with write_to_file_with_embedded_schema() / write_to_file_split() or\n\
         save the schema to ~/.config/mmapstruct/schemas/<layout_hash>.mms.\n\
         Use --schema <file> to specify a schema explicitly."
            .to_string(),
    )
}
