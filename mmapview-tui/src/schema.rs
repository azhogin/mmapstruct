use mmapview::schema::SchemaResult;

/// Load schema for a data file.
///
/// Thin delegate to the unified resolver in `mmapview::schema`:
/// 1. Explicit `--schema <file>` path
/// 2. Embedded schema in v2 file header
/// 3. Registry lookup by `layout_hash`
/// 4. Sidecar `<data_path>.mmshdr`
pub fn load_schema(
    data: &[u8],
    schema_path: Option<&str>,
    data_path: Option<&str>,
) -> Result<SchemaResult, String> {
    mmapview::schema::load_schema_full(data, schema_path, data_path)
}
