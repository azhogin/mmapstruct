use mmapstruct::MmapStruct;

#[derive(MmapStruct, Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Simple {
    x: u32,
    y: u32,
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
