use crate::context::RuntimeSchemaCtx;
use crate::schema::TypeDefReader;
use crate::value::SchemaValue;

/// Navigate a path like `/items[0]/label` relative to `root`.
///
/// Supports struct field access and array indexing.
pub fn navigate_path<'a>(
    ctx: &'a RuntimeSchemaCtx<'a>,
    root: &'a SchemaValue,
    path: &str,
) -> Option<SchemaValue> {
    let path = path.strip_prefix('/').unwrap_or(path);
    if path.is_empty() {
        return Some(*root);
    }
    let segments: Vec<&str> = path.split('/').collect();
    let mut current = *root;
    for seg in segments {
        if seg.is_empty() {
            continue;
        }
        if let Some((name, index)) = parse_field_segment(seg) {
            current = ctx.field_get(&current, name)?;
            if let Some(idx) = index {
                let inner_type = ctx.unwrap_type(current.type_idx);
                if matches!(ctx.type_reader(inner_type), TypeDefReader::Array { .. }) {
                    current = ctx.array_get(&current, idx)?;
                } else {
                    return None;
                }
            }
        }
    }
    Some(current)
}

/// Parse `field[0]` into `("field", Some(0))` or `field` into `("field", None)`.
fn parse_field_segment(seg: &str) -> Option<(&str, Option<u64>)> {
    if let Some(bracket) = seg.find('[') {
        let name = &seg[..bracket];
        let rest = &seg[bracket + 1..];
        if let Some(end) = rest.find(']') {
            let idx: u64 = rest[..end].parse().ok()?;
            Some((name, Some(idx)))
        } else {
            None
        }
    } else {
        Some((seg, None))
    }
}
