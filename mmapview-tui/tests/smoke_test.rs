use mmapstruct::MmapStruct;

#[derive(MmapStruct, Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Simple {
    x: u32,
    y: u32,
}

#[derive(MmapStruct, Debug)]
enum Shape {
    Circle { radius: f64 },
    Rect { width: f64, height: f64 },
    Empty,
}

#[derive(MmapStruct, Debug)]
struct MyEnums {
    shapes: Vec<Shape>,
}

#[derive(MmapStruct, Clone, Copy, Debug, Eq, PartialEq, Hash, Ord, PartialOrd)]
#[repr(u32)]
enum Status {
    Active,
    Inactive,
    Pending,
}

#[derive(MmapStruct, Debug)]
struct MyHashMap {
    map: std::collections::HashMap<Status, u64>,
}

#[test]
fn test_build_tree() {
    let s = Simple { x: 42, y: 99 };
    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&s, false);

    let loaded = mmapview_tui::schema::load_schema(&buf, None).unwrap();
    let data: &'static [u8] = Box::leak(buf.into_boxed_slice());
    let schema_bytes: &'static [u8] = Box::leak(loaded.schema_bytes.into_boxed_slice());
    let ctx: &'static mmapstruct_schema::context::RuntimeSchemaCtx<'static> =
        Box::leak(Box::new(mmapstruct_schema::context::RuntimeSchemaCtx::new(data, schema_bytes)));

    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;
    let root = mmapstruct_schema::SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );

    // Use new Overscan/Scanner machinery
    let scanner = mmapview_tui::scanner::Scanner::new(ctx, root, 100);
    let os = mmapview_tui::overscan::Overscan::new(scanner);
    assert!(!os.nodes.is_empty(), "should have at least a root node");
    // Top-level nodes (immediate children of root) have depth 1
    assert_eq!(os.nodes[0].depth, 1);
    // Simple has 2 fields → 2 top-level nodes
    assert_eq!(os.nodes.len(), 2);
}

#[test]
fn test_build_enum_tree() {
    let shapes = vec![
        Shape::Circle { radius: 1.0 },
        Shape::Rect { width: 2.0, height: 3.0 },
        Shape::Empty,
    ];
    let root = MyEnums { shapes };
    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);

    let loaded = mmapview_tui::schema::load_schema(&buf, None).unwrap();
    let data: &'static [u8] = Box::leak(buf.into_boxed_slice());
    let schema_bytes: &'static [u8] = Box::leak(loaded.schema_bytes.into_boxed_slice());
    let ctx: &'static mmapstruct_schema::context::RuntimeSchemaCtx<'static> =
        Box::leak(Box::new(mmapstruct_schema::context::RuntimeSchemaCtx::new(data, schema_bytes)));

    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;
    let root = mmapstruct_schema::SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );

    let scanner = mmapview_tui::scanner::Scanner::new(ctx, root, 100);
    let os = mmapview_tui::overscan::Overscan::build(scanner, |_| true);
    assert!(!os.nodes.is_empty(), "should have at least a root node");
    // Root (MyEnums) → shapes (Vec) → 3 entries → variant fields
    assert!(
        os.nodes.len() >= 5,
        "expected enough nodes for enum variants, got {}",
        os.nodes.len()
    );
    // Verify we can format the display text without panicking
    let _ = mmapview_tui::print::inline_value_text(ctx, &os.nodes[0].value);
}

#[test]
fn test_build_hash_map() {
    use std::collections::HashMap;
    let map: HashMap<Status, u64> = [
        (Status::Active, 100),
        (Status::Inactive, 200),
        (Status::Pending, 300),
    ]
    .into();
    let root = MyHashMap { map };
    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&root, false);

    let loaded = mmapview_tui::schema::load_schema(&buf, None).unwrap();
    let data: &'static [u8] = Box::leak(buf.into_boxed_slice());
    let schema_bytes: &'static [u8] = Box::leak(loaded.schema_bytes.into_boxed_slice());
    let ctx: &'static mmapstruct_schema::context::RuntimeSchemaCtx<'static> =
        Box::leak(Box::new(mmapstruct_schema::context::RuntimeSchemaCtx::new(data, schema_bytes)));

    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;
    let root = mmapstruct_schema::SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );

    let scanner = mmapview_tui::scanner::Scanner::new(ctx, root, 100);
    let os = mmapview_tui::overscan::Overscan::build(scanner, |_| true);
    assert!(!os.nodes.is_empty(), "should have at least a root node");
    // MyHashMap { map: HashMap<Status, u64> } with 3 entries
    println!("hash_map nodes: {:?}", os.nodes.iter().map(|n| &n.display_text).collect::<Vec<_>>());
    assert!(
        os.nodes.len() >= 3,
        "expected at least 3 nodes for hash map entries, got {}",
        os.nodes.len()
    );
    // Verify we can format without panicking
    let _ = mmapview_tui::print::inline_value_text(ctx, &os.nodes[0].value);
    // Check that pod enum values show variant names, not raw numbers
    let status_entry = &os.nodes[1];
    assert!(
        status_entry.display_text.contains("Status::"),
        "expected variant name in display text, got: {}",
        status_entry.display_text
    );
}
