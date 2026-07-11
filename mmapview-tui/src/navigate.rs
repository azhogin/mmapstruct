use crate::path::{Path, PathSegment, PathShift};
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::TypeDefReader;
use mmapstruct_schema::SchemaValue;

/// Navigate a string path like `/items[0]/label` relative to `root`.
///
/// Returns the final value and a `Path` describing every navigation step.
///
/// The path format per `/`-segment is `field_name` or `field_name[index]`:
/// - `field_name` → struct/variant field access (adds `PathShift::Field`)
/// - `[index]`    → array/hashmap indexing (adds `PathShift::Entry`)
///
/// If the final resolved value has children, an extra `PathShift::Field(0)`
/// segment is appended so that the cursor lands *inside* it (matching the
/// behaviour of `Scanner::new`, which starts on the root's first child).
///
/// Returns `None` if any segment does not exist or the types are incompatible.
pub fn navigate_path_segments(
    ctx: &RuntimeSchemaCtx,
    root: &SchemaValue,
    path_str: &str,
) -> Option<(SchemaValue, Path)> {
    let path = path_str.strip_prefix('/').unwrap_or(path_str);
    let segments: Vec<&str> = path.split('/').collect();
    let mut result_path = Path(Vec::new());
    let mut current = *root;

    for seg in &segments {
        if seg.is_empty() {
            continue;
        }
        let (name, index) = parse_field_segment(seg)?;

        // Descend into the named field — record index for path
        let (field_idx, field_val) = ctx.field_index_and_value(&current, name)?;
        result_path.push(PathSegment {
            val: current,
            shift: PathShift::Field(field_idx),
        });
        current = field_val;

        // Array / hashmap indexing
        if let Some(idx) = index {
            let inner_type = ctx.unwrap_type(current.type_idx);
            match ctx.type_reader(inner_type) {
                TypeDefReader::Array { .. } => {
                    result_path.push(PathSegment {
                        val: current,
                        shift: PathShift::Entry(idx as usize),
                    });
                    current = ctx.array_get(&current, idx)?;
                }
                TypeDefReader::HashMap { .. } => {
                    result_path.push(PathSegment {
                        val: current,
                        shift: PathShift::Entry(idx as usize),
                    });
                    let (_, v) = ctx.hashmap_get_index(&current, idx as usize)?;
                    current = v;
                }
                _ => return None,
            }
        }
    }

    // If the final value can hold children, step into the first one so that
    // `do_steps` positions the cursor *inside* the value, not *on* it.
    if has_children(ctx, &current) {
        result_path.push(PathSegment {
            val: current,
            shift: PathShift::Field(0),
        });
    }

    Some((current, result_path))
}

/// Returns `true` when `val` has at least one logical child in the tree view.
fn has_children(ctx: &RuntimeSchemaCtx, val: &SchemaValue) -> bool {
    let t = ctx.type_reader(ctx.unwrap_type(val.type_idx));
    match t {
        TypeDefReader::Struct { fields, .. } => fields.len() > 0,
        TypeDefReader::Enum { .. } => {
            let (variant, _) = ctx.enum_variant(val).unwrap_or((0, 0));
            let vt = ctx.type_reader(variant);
            matches!(vt, TypeDefReader::Struct { fields, .. } if fields.len() > 0)
        }
        TypeDefReader::Array { .. } => ctx.array_len(val) > 0,
        TypeDefReader::HashMap { .. } => ctx.hashmap_len(val) > 0,
        TypeDefReader::Option { .. } => val.mmap_begin < val.mmap_end,
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mmapstruct::MmapStruct;
    use mmapstruct::schema::GetSchema;
    use mmapstruct::traits::{LayoutHash, MmapRead, MmapWrite};
    use mmapstruct_schema::{write_to_file_with_embedded_schema, Schema};
    use bytemuck::{Pod, Zeroable};

    #[derive(MmapStruct, Clone, Copy, Debug, Pod, Zeroable)]
    #[repr(C)]
    struct Point {
        x: u32,
        y: u32,
    }

    #[derive(MmapStruct, Clone, Debug)]
    struct WithVec {
        items: Vec<Point>,
        extra: u32,
    }

    fn setup<T: MmapWrite + GetSchema + LayoutHash>(
        val: &T,
    ) -> (&'static RuntimeSchemaCtx<'static>, SchemaValue) {
        let buf = write_to_file_with_embedded_schema(val, false);
        let loaded = crate::schema::load_schema(&buf, None).unwrap();
        let data: &'static [u8] = Box::leak(buf.into_boxed_slice());
        let schema_bytes: &'static [u8] = Box::leak(loaded.schema_bytes.into_boxed_slice());
        let ctx: &'static RuntimeSchemaCtx<'static> = Box::leak(Box::new(
            RuntimeSchemaCtx::new(data, schema_bytes),
        ));
        let schema_reader = <Schema as MmapRead>::read(schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;
        let root = SchemaValue::new(
            loaded.payload_range.start as u64,
            loaded.payload_range.end as u64,
            root_type,
        );
        (ctx, root)
    }

    #[test]
    fn test_navigate_simple_field() {
        let v = Point { x: 10, y: 20 };
        let (ctx, root) = setup(&v);
        let (val, path) = navigate_path_segments(ctx, &root, "/x").unwrap();
        assert_eq!(ctx.read_u32(&val), 10);
        assert_eq!(path.len(), 1);
        assert!(matches!(path[0].shift, PathShift::Field(0)));
    }

    #[test]
    fn test_navigate_nested_field() {
        let v = WithVec { items: vec![Point { x: 1, y: 2 }, Point { x: 3, y: 4 }], extra: 99 };
        let (ctx, root) = setup(&v);
        let (val, path) = navigate_path_segments(ctx, &root, "/items").unwrap();
        assert_eq!(ctx.array_len(&val), 2);
        // /items → Field(0), auto-step-into first child → Field(0)
        assert_eq!(path.len(), 2);
        assert!(matches!(path[0].shift, PathShift::Field(0)));
    }

    #[test]
    fn test_navigate_array_index() {
        let v = WithVec { items: vec![Point { x: 1, y: 2 }, Point { x: 3, y: 4 }], extra: 99 };
        let (ctx, root) = setup(&v);
        let (val, path) = navigate_path_segments(ctx, &root, "/items[1]/x").unwrap();
        assert_eq!(ctx.read_u32(&val), 3);
        assert_eq!(path.len(), 3);
        assert!(matches!(path[0].shift, PathShift::Field(0)));
        assert!(matches!(path[1].shift, PathShift::Entry(1)));
        assert!(matches!(path[2].shift, PathShift::Field(0)));
    }

    #[test]
    fn test_navigate_empty_path() {
        let v = Point { x: 10, y: 20 };
        let (ctx, root) = setup(&v);
        let (val, path) = navigate_path_segments(ctx, &root, "").unwrap();
        assert_eq!(val, root);
        // Auto-step-into root → 1 segment
        assert_eq!(path.len(), 1);
        assert!(matches!(path[0].shift, PathShift::Field(0)));
    }

    #[test]
    fn test_navigate_invalid_field() {
        let v = Point { x: 10, y: 20 };
        let (ctx, root) = setup(&v);
        assert!(navigate_path_segments(ctx, &root, "/nonexistent").is_none());
    }

    #[test]
    fn test_navigate_invalid_index() {
        let v = WithVec { items: vec![Point { x: 1, y: 2 }], extra: 99 };
        let (ctx, root) = setup(&v);
        assert!(navigate_path_segments(ctx, &root, "/items[99]").is_none());
    }
}
