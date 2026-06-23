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
    let ctx = mmapstruct_schema::context::RuntimeSchemaCtx::new(&buf, &loaded.schema_bytes);
    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        &loaded.schema_bytes,
        0..loaded.schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;
    let root = mmapstruct_schema::SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );
    let nodes = mmapview_tui::tree::build_tree(&ctx, &root, "Simple", 3, None);
    assert!(!nodes.is_empty(), "should have at least a root node");
    assert_eq!(nodes[0].depth, 0);
    assert_eq!(nodes[0].children.len(), 2);
}
