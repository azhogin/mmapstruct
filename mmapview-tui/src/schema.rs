use std::fs;
use std::path::Path;

/// Load schema from a data buffer.
///
/// Resolution order:
/// 1. Explicit `--schema <file>` — load from given path
/// 2. Embedded schema in v2 file header
/// 3. Registry lookup by `layout_hash` (from file header)
/// 4. Sidecar `<data_path>.mmshdr` — its embedded schema, or (if it carries
///    only a `layout_hash`) the registry keyed by that hash.
pub fn load_schema(
    data: &[u8],
    schema_path: Option<&str>,
    data_path: Option<&str>,
) -> Result<mmapview::schema::SchemaResult, String> {
    if let Some(path) = schema_path {
        let schema_bytes = fs::read(path).map_err(|e| format!("Error reading schema '{}': {}", path, e))?;
        let (payload, _, _, _) = mmapstruct::file_reader::parse_header(data);
        let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
        let payload_range = payload_start..payload_start + payload.len();
        return Ok(mmapview::schema::SchemaResult {
            schema_bytes,
            payload_range,
        });
    }

    // 2 + 3: embedded / registry
    if let Ok(r) = mmapview::schema::resolve_schema(data) {
        return Ok(r);
    }

    // 4: sidecar
    if let Some(dp) = data_path {
        let mut sidecar_path = Path::new(dp).to_path_buf();
        sidecar_path.set_extension("mmshdr");
        if let Ok(sb) = fs::read(&sidecar_path) {
            // Payload range comes from the (headerless) data file, not the sidecar.
            let (payload, _, _, _) = mmapstruct::file_reader::parse_header(data);
            let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
            let payload_range = payload_start..payload_start + payload.len();
            let (_, schema_range, hash, _) = mmapstruct::file_reader::parse_header(&sb);
            let schema_bytes = if let Some((s, e)) = schema_range {
                sb[s..e].to_vec()
            } else if let Some(h) = hash {
                mmapstruct_schema::registry::load_schema(h).ok_or_else(|| {
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
            return Ok(mmapview::schema::SchemaResult {
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
