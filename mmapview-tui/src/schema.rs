use std::fs;

/// Load schema from a data buffer.
///
/// Resolution order:
/// 1. Explicit `--schema <file>` — load from given path
/// 2. Embedded schema in v2 file header
/// 3. Registry lookup by `layout_hash`
pub fn load_schema(data: &[u8], schema_path: Option<&str>) -> Result<mmapview::schema::SchemaResult, String> {
    if let Some(path) = schema_path {
        let schema_bytes = fs::read(path).map_err(|e| format!("Error reading schema '{}': {}", path, e))?;
        let (payload, _, _, _) = mmapstruct::file_reader::parse_header(data);
        let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
        let payload_range = payload_start..payload_start + payload.len();
        Ok(mmapview::schema::SchemaResult {
            schema_bytes,
            payload_range,
        })
    } else {
        mmapview::schema::resolve_schema(data)
    }
}
