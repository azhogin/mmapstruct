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

    let loaded = mmapview_tui::schema::load_schema(&buf, None, None).unwrap();
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

    let loaded = mmapview_tui::schema::load_schema(&buf, None, None).unwrap();
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

// ── Data structures for tuple display test ──

#[derive(MmapStruct, Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct PodPoint(u32, u32);

#[derive(MmapStruct, Debug)]
struct DynPair(String, String);

#[derive(MmapStruct, Debug)]
struct WithTuple {
    point: PodPoint,
    tag: String,
}

#[derive(MmapStruct, Debug)]
enum Item {
    Named { id: u32 },
    Pair(String, u32),
}

#[derive(MmapStruct, Debug)]
struct TupleDisplayDemo {
    #[pod]
    points: Vec<PodPoint>,
    pairs: Vec<DynPair>,
    nested: WithTuple,
    items: Vec<Item>,
}

#[test]
fn test_tuple_display() {
    let demo = TupleDisplayDemo {
        points: vec![PodPoint(10, 20), PodPoint(30, 40)],
        pairs: vec![DynPair("hello".into(), "world".into())],
        nested: WithTuple {
            point: PodPoint(1, 2),
            tag: "test".into(),
        },
        items: vec![
            Item::Named { id: 7 },
            Item::Pair("foo".into(), 99),
        ],
    };
    let buf = mmapstruct_schema::write_to_file_with_embedded_schema(&demo, false);

    let loaded = mmapview_tui::schema::load_schema(&buf, None, None).unwrap();
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

    // Collect display texts for assertions
    let texts: Vec<&str> = os.nodes.iter().map(|n| n.display_text.as_str()).collect();
    let joined = texts.join("\n");
    println!("=== Tuple display nodes ===\n{joined}\n=========================");

    // Item 1: Pod tuple array – field names "0" and "1"
    assert!(
        texts.iter().any(|t| t.contains("0: u32 = 10")),
        "expected pod tuple field 0 with value 10, got:\n{}",
        joined
    );
    assert!(
        texts.iter().any(|t| t.contains("1: u32 = 20")),
        "expected pod tuple field 1 with value 20, got:\n{}",
        joined
    );

    // Item 2: All-dynamic tuple array – fields "0" and "1"
    assert!(
        texts.iter().any(|t| t.contains("0: str = \"hello\"")),
        "expected dyn pair field 0 = \"hello\", got:\n{}",
        joined
    );
    assert!(
        texts.iter().any(|t| t.contains("1: str = \"world\"")),
        "expected dyn pair field 1 = \"world\", got:\n{}",
        joined
    );

    // Item 3: Tuple struct as a direct (nested) field
    assert!(
        texts.iter().any(|t| t.contains("point = PodPoint")),
        "expected nested tuple field named 'point', got:\n{}",
        joined
    );
    assert!(
        texts.iter().any(|t| t.contains("0: u32 = 1")),
        "expected nested pod tuple field 0 = 1, got:\n{}",
        joined
    );

    // Item 4: Enum with tuple variant
    assert!(
        texts.iter().any(|t| t.contains("Item::Pair")),
        "expected tuple enum variant 'Item::Pair', got:\n{}",
        joined
    );
    assert!(
        texts.iter().any(|t| t.contains("0: str = \"foo\"")),
        "expected tuple variant field 0 = \"foo\", got:\n{}",
        joined
    );
    assert!(
        texts.iter().any(|t| t.contains("1: u32 = 99")),
        "expected tuple variant field 1 = 99, got:\n{}",
        joined
    );
}

#[test]
fn test_field_type_display() {
    use mmapstruct_schema::write_to_file_with_embedded_schema;

    // Use three separate structs to avoid scanner issues with byref Option.
    // ── Inline string + Option<u32> ──
    #[derive(MmapStruct, Debug)]
    struct InlineDemo {
        #[mmap(inline(32))]
        code: String,
        maybe_count: Option<u32>,
    }
    let demo1 = InlineDemo {
        code: "ABC123".into(),
        maybe_count: Some(42),
    };
    let buf1 = write_to_file_with_embedded_schema(&demo1, false);
    let loaded1 = mmapview_tui::schema::load_schema(&buf1, None, None).unwrap();
    let d1: &'static [u8] = Box::leak(buf1.into_boxed_slice());
    let sb1: &'static [u8] = Box::leak(loaded1.schema_bytes.into_boxed_slice());
    let ctx1 = Box::leak(Box::new(
        mmapstruct_schema::context::RuntimeSchemaCtx::new(d1, sb1),
    ));
    let sr1 = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(sb1, 0..sb1.len());
    let root1 = mmapstruct_schema::SchemaValue::new(
        loaded1.payload_range.start as u64,
        loaded1.payload_range.end as u64,
        *sr1.root_type,
    );
    let sc1 = mmapview_tui::scanner::Scanner::new(ctx1, root1, 100);
    let os1 = mmapview_tui::overscan::Overscan::build(sc1, |_| true);
    let t1: Vec<&str> = os1.nodes.iter().map(|n| n.display_text.as_str()).collect();
    let j1 = t1.join("\n");
    println!("=== Inline + Option<u32> ===\n{j1}\n============================");

    assert!(t1.iter().any(|t| t.contains("ABC123")), "inline value missing:\n{j1}");
    assert!(t1.iter().any(|t| t.contains("42")), "option value 42 missing:\n{j1}");

    // ── Vec<String> (indexed array) ──
    #[derive(MmapStruct, Debug)]
    struct VecDemo {
        tags: Vec<String>,
    }
    let demo2 = VecDemo {
        tags: vec!["alpha".into(), "beta".into()],
    };
    let buf2 = write_to_file_with_embedded_schema(&demo2, false);
    let loaded2 = mmapview_tui::schema::load_schema(&buf2, None, None).unwrap();
    let d2: &'static [u8] = Box::leak(buf2.into_boxed_slice());
    let sb2: &'static [u8] = Box::leak(loaded2.schema_bytes.into_boxed_slice());
    let ctx2 = Box::leak(Box::new(
        mmapstruct_schema::context::RuntimeSchemaCtx::new(d2, sb2),
    ));
    let sr2 = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(sb2, 0..sb2.len());
    let root2 = mmapstruct_schema::SchemaValue::new(
        loaded2.payload_range.start as u64,
        loaded2.payload_range.end as u64,
        *sr2.root_type,
    );
    let sc2 = mmapview_tui::scanner::Scanner::new(ctx2, root2, 100);
    let os2 = mmapview_tui::overscan::Overscan::build(sc2, |_| true);
    let t2: Vec<&str> = os2.nodes.iter().map(|n| n.display_text.as_str()).collect();
    let j2 = t2.join("\n");
    println!("=== Vec<String> ===\n{j2}\n====================");

    assert!(t2.iter().any(|t| t.contains("alpha")), "array elem alpha missing:\n{j2}");
    assert!(t2.iter().any(|t| t.contains("beta")), "array elem beta missing:\n{j2}");

    // ── #[mmap(bool)] u32 ──
    #[derive(MmapStruct, Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
    #[repr(C)]
    struct BoolDemo {
        #[mmap(bool)]
        enabled: u32,
    }
    let demo3 = BoolDemo { enabled: 1 };
    let buf3 = write_to_file_with_embedded_schema(&demo3, false);
    let loaded3 = mmapview_tui::schema::load_schema(&buf3, None, None).unwrap();
    let d3: &'static [u8] = Box::leak(buf3.into_boxed_slice());
    let sb3: &'static [u8] = Box::leak(loaded3.schema_bytes.into_boxed_slice());
    let ctx3 = Box::leak(Box::new(
        mmapstruct_schema::context::RuntimeSchemaCtx::new(d3, sb3),
    ));
    let sr3 = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(sb3, 0..sb3.len());
    let root3 = mmapstruct_schema::SchemaValue::new(
        loaded3.payload_range.start as u64,
        loaded3.payload_range.end as u64,
        *sr3.root_type,
    );
    let sc3 = mmapview_tui::scanner::Scanner::new(ctx3, root3, 100);
    let os3 = mmapview_tui::overscan::Overscan::build(sc3, |_| true);
    let t3: Vec<&str> = os3.nodes.iter().map(|n| n.display_text.as_str()).collect();
    let j3 = t3.join("\n");
    println!("=== BoolDemo ===\n{j3}\n=================");

    assert!(t3.iter().any(|t| t.contains("true")), "bool value true missing:\n{j3}");

}

/// Regression test: byref Option should not cause scanner infinite loop.
#[test]
fn test_byref_option_scanner_bug() {
    use mmapstruct_schema::write_to_file_with_embedded_schema;

    #[derive(MmapStruct, Debug)]
    struct BugDemo {
        #[mmap(byref)]
        label: Option<String>,
    }
    let demo = BugDemo { label: Some("test".into()) };
    let buf = write_to_file_with_embedded_schema(&demo, false);

    let loaded = mmapview_tui::schema::load_schema(&buf, None, None).unwrap();
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
    let texts: Vec<&str> = os.nodes.iter().map(|n| n.display_text.as_str()).collect();
    let joined = texts.join("\n");

    // A healthy struct with one field produces at most 3 nodes (field header + inner).
    // The bug produces 100 entries (full scanner capacity).
    assert!(
        texts.len() <= 5,
        "byref Option scanner bug: expected ≤5 nodes, got {}:\n{}",
        texts.len(),
        joined,
    );
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

    let loaded = mmapview_tui::schema::load_schema(&buf, None, None).unwrap();
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

// ── FixedArray display ──

#[derive(MmapStruct, Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct MatrixDemo {
    id: u32,
    matrix: [[u32; 4]; 4],
}

#[test]
fn test_fixed_array_display() {
    use mmapstruct_schema::write_to_file_with_embedded_schema;
    let demo = MatrixDemo {
        id: 1,
        matrix: [[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12], [13, 14, 15, 16]],
    };
    let buf = write_to_file_with_embedded_schema(&demo, false);
    let loaded = mmapview_tui::schema::load_schema(&buf, None, None).unwrap();
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
    let scanner = mmapview_tui::scanner::Scanner::new(ctx, root, 200);
    let os = mmapview_tui::overscan::Overscan::build(scanner, |_| true);
    let texts: Vec<&str> = os.nodes.iter().map(|n| n.display_text.as_str()).collect();
    let joined = texts.join("\n");
    println!("=== MatrixDemo ===\n{joined}\n=================");

    assert!(texts.iter().any(|t| t.contains("matrix")), "matrix field missing");
    let array_nodes: Vec<&&str> = texts.iter().filter(|t| t.starts_with('[')).collect();
    assert!(array_nodes.len() >= 4, "expected at least 4 array elements, got {}:\n{joined}", array_nodes.len());
    assert!(texts.iter().any(|t| t.contains("= 6")), "value 6 missing");
    assert!(texts.iter().any(|t| t.contains("= 16")), "value 16 missing");
}

#[derive(MmapStruct, Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Splittable {
    x: u32,
    y: f32,
}

#[test]
fn test_load_schema_sidecar() {
    let v = Splittable { x: 123, y: 3.25 };
    let path = std::env::temp_dir().join("sidecar_test.mms");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("mmshdr"));
    mmapstruct_schema::write_to_file_split(&path, &v, true, true).unwrap();
    assert!(path.exists());
    assert!(path.with_extension("mmshdr").exists());

    let buf = std::fs::read(&path).unwrap();
    // headerless data must not start with the mmapstruct magic
    assert_ne!(&buf[0..4], b"MMAP");

    // auto-detect the sidecar via the data file path
    let loaded = mmapview_tui::schema::load_schema(
        &buf,
        None,
        Some(path.to_str().unwrap()),
    )
    .unwrap();
    assert!(!loaded.schema_bytes.is_empty());
    assert_eq!(loaded.payload_range, 0..buf.len());

    // and the typed reader must open it via the sidecar
    let r = mmapstruct::MmapReader::from_file_with_sidecar(&path).unwrap();
    let got: Splittable = r.read::<Splittable>().into();
    assert_eq!(got, v);
}

#[test]
fn test_load_schema_sidecar_registry() {
    // Use an isolated registry dir so we don't touch the real home.
    let reg = std::env::temp_dir().join(format!("mmap_schema_registry_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&reg);
    std::env::set_var("MMAPSTRUCT_SCHEMA_DIR", &reg);

    let v = Splittable { x: 7, y: 2.5 };
    // Save the shared schema to the registry (what the split command does).
    mmapstruct_schema::ensure_registry_schema::<Splittable>();

    let path = std::env::temp_dir().join("sidecar_reg_test.mms");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("mmshdr"));
    // Hash-only sidecar (embed_schema = false): schema comes from the registry.
    mmapstruct_schema::write_to_file_split(&path, &v, false, false).unwrap();
    assert!(path.exists());
    assert!(path.with_extension("mmshdr").exists());

    let buf = std::fs::read(&path).unwrap();
    assert_ne!(&buf[0..4], b"MMAP");

    // Sidecar carries only layout_hash -> schema resolved from the registry.
    let loaded = mmapview_tui::schema::load_schema(
        &buf,
        None,
        Some(path.to_str().unwrap()),
    )
    .unwrap();
    assert!(!loaded.schema_bytes.is_empty());
    assert_eq!(loaded.payload_range, 0..buf.len());

    // Typed reader opens it via the sidecar hash.
    let r = mmapstruct::MmapReader::from_file_with_sidecar(&path).unwrap();
    let got: Splittable = r.read::<Splittable>().into();
    assert_eq!(got, v);

    let _ = std::fs::remove_dir_all(&reg);
}
