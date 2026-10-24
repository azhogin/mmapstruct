use mmapstruct::{MmapStruct, write_to_buf_raw};
use mmapstruct::traits::MmapRead;
use mmapstruct_schema::builder::build_serialized_schema;
use mmapstruct_schema::schema::TypeDefReader;
use mmapstruct_schema::{
    format_schema_value, navigate_path, RuntimeSchemaCtx, SchemaValue,
    write_with_schema,
};

#[test]
fn test_schema_build_simple() {
    #[derive(MmapStruct)]
    struct Point {
        x: u64,
        y: String,
    }

    let schema = build_serialized_schema::<Point>();
    match &schema.types[0] {
        mmapstruct_schema::schema::TypeDef::Struct { name, fields, .. } => {
            assert_eq!(name, "Point");
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name, "x");
            assert_eq!(fields[1].name, "y");
        }
        _ => panic!("expected struct"),
    }
    assert!(matches!(
        schema.types[1],
        mmapstruct_schema::schema::TypeDef::Primitive(mmapstruct_schema::schema::PrimitiveDef::U64)
    ));
    assert!(matches!(schema.types[2], mmapstruct_schema::schema::TypeDef::Str { .. }));
    assert_eq!(schema.root_type, 0);
}

#[test]
fn test_schema_build_mixed() {
    #[derive(MmapStruct)]
    struct Rec {
        id: u64,
        name: String,
        tags: Vec<String>,
    }

    let schema = build_serialized_schema::<Rec>();
    match &schema.types[0] {
        mmapstruct_schema::schema::TypeDef::Struct { name, fields, .. } => {
            assert_eq!(name, "Rec");
            assert_eq!(fields.len(), 3);
            assert_eq!(fields[0].name, "id");
            assert_eq!(fields[1].name, "name");
            assert_eq!(fields[2].name, "tags");
        }
        _ => panic!("expected struct"),
    }
    assert!(matches!(
        schema.types[1],
        mmapstruct_schema::schema::TypeDef::Primitive(mmapstruct_schema::schema::PrimitiveDef::U64)
    ));
    assert!(matches!(schema.types[2], mmapstruct_schema::schema::TypeDef::Str { .. }));

    match &schema.types[3] {
        mmapstruct_schema::schema::TypeDef::Array { layout, .. } => {
            assert!(matches!(layout, mmapstruct_schema::schema::ArrayLayoutDef::Indexed));
        }
        _ => panic!("expected array"),
    }
    assert_eq!(schema.root_type, 0);
}

#[test]
fn test_schema_build_enum() {
    #[derive(MmapStruct)]
    enum MyEnum {
        Alpha { id: u64 },
        Beta { name: String, val: u64 },
    }

    let schema = build_serialized_schema::<MyEnum>();
    match &schema.types[0] {
        mmapstruct_schema::schema::TypeDef::Enum { name, variants, .. } => {
            assert_eq!(name, "MyEnum");
            assert_eq!(variants.len(), 2);
            assert_eq!(variants[0].name, "MyEnum::Alpha");
            assert_eq!(variants[0].fields.len(), 1);
            assert_eq!(variants[1].name, "MyEnum::Beta");
            assert_eq!(variants[1].fields.len(), 2);
        }
        _ => panic!("expected enum"),
    }
    assert_eq!(schema.root_type, 0);
}

#[test]
fn test_schema_build_with_option() {
    #[derive(MmapStruct)]
    struct WithOpt {
        id: u64,
        label: Option<String>,
    }

    let schema = build_serialized_schema::<WithOpt>();
    match &schema.types[0] {
        mmapstruct_schema::schema::TypeDef::Struct { fields, .. } => {
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name, "id");
            assert_eq!(fields[1].name, "label");
            assert_eq!(fields[1].type_index, 2);
        }
        _ => panic!("expected struct"),
    }
    assert!(matches!(
        schema.types[1],
        mmapstruct_schema::schema::TypeDef::Primitive(mmapstruct_schema::schema::PrimitiveDef::U64)
    ));
    match &schema.types[2] {
        mmapstruct_schema::schema::TypeDef::Option { inner_type } => {
            assert_eq!(*inner_type, 3);
        }
        _ => panic!("expected option"),
    }
    assert!(matches!(schema.types[3], mmapstruct_schema::schema::TypeDef::Str { .. }));
    assert_eq!(schema.root_type, 0);
}

#[test]
fn test_write_with_schema() {
    #[derive(MmapStruct)]
    struct Rec {
        id: u64,
        name: String,
    }

    let pt = Rec {
        id: 10,
        name: "hello".into(),
    };
    let (data, schema_bytes) = write_with_schema(&pt);

    assert!(!data.is_empty());
    let schema = build_serialized_schema::<Rec>();
    assert_eq!(schema.root_type, 0);
    assert!(!schema_bytes.is_empty());

    let full = mmapstruct::write_to_buf(&pt, false);
    assert_eq!(&full[24..], &data[..]);
}

#[test]
fn test_schema_roundtrip_simple() {
    #[derive(MmapStruct)]
    struct Point {
        x: u64,
        y: String,
    }

    let schema = build_serialized_schema::<Point>();
    let buf = write_to_buf_raw(&schema);

    let schema2: mmapstruct_schema::schema::Schema =
        <mmapstruct_schema::schema::Schema as MmapRead>::read(&buf, 0..buf.len()).into();
    assert_eq!(schema.types.len(), schema2.types.len());
    assert_eq!(schema.root_type, schema2.root_type);
}

#[test]
fn test_schema_roundtrip_mixed() {
    #[derive(MmapStruct)]
    struct Rec {
        id: u64,
        name: String,
        tags: Vec<String>,
    }

    let schema = build_serialized_schema::<Rec>();
    let buf = write_to_buf_raw(&schema);

    let schema2: mmapstruct_schema::schema::Schema =
        <mmapstruct_schema::schema::Schema as MmapRead>::read(&buf, 0..buf.len()).into();
    assert_eq!(schema.types.len(), schema2.types.len());
}

#[test]
fn test_schema_roundtrip_enum() {
    #[derive(MmapStruct)]
    enum MyEnum {
        Alpha { id: u64 },
        Beta { name: String, val: u64 },
    }

    let schema = build_serialized_schema::<MyEnum>();
    let buf = write_to_buf_raw(&schema);

    let schema2: mmapstruct_schema::schema::Schema =
        <mmapstruct_schema::schema::Schema as MmapRead>::read(&buf, 0..buf.len()).into();
    assert_eq!(schema.types.len(), schema2.types.len());
}

#[test]
fn test_schema_roundtrip_option() {
    #[derive(MmapStruct)]
    struct WithOpt {
        id: u64,
        label: Option<String>,
    }

    let schema = build_serialized_schema::<WithOpt>();
    let buf = write_to_buf_raw(&schema);

    let schema2: mmapstruct_schema::schema::Schema =
        <mmapstruct_schema::schema::Schema as MmapRead>::read(&buf, 0..buf.len()).into();
    assert_eq!(schema.types.len(), schema2.types.len());
    assert_eq!(schema.root_type, schema2.root_type);
}

#[test]
fn test_schema_write_with_schema_readback() {
    #[derive(MmapStruct)]
    struct Rec {
        id: u64,
        name: String,
    }

    let pt = Rec {
        id: 10,
        name: "hello".into(),
    };
    let (_data, schema_bytes) = write_with_schema(&pt);

    let schema: mmapstruct_schema::schema::Schema =
        <mmapstruct_schema::schema::Schema as MmapRead>::read(
            &schema_bytes,
            0..schema_bytes.len(),
        )
        .into();
    assert!(schema.types.len() >= 1);
    assert_eq!(schema.root_type, 0);
}

#[test]
fn test_schema_context_field_get() {
    #[derive(MmapStruct)]
    struct Point {
        x: u64,
        y: String,
    }

    let pt = Point {
        x: 42,
        y: "hello".into(),
    };
    let data = mmapstruct::write_to_buf_raw(&pt);
    let schema = build_serialized_schema::<Point>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let x = ctx.field_get(&root, "x").unwrap();
    assert_eq!(ctx.read_u64(&x), 42);

    let y = ctx.field_get(&root, "y").unwrap();
    assert_eq!(ctx.read_str(&y), "hello");
}

#[test]
fn test_schema_context_enum_variant() {
    #[derive(MmapStruct)]
    enum MyEnum {
        Alpha { id: u32 },
        Beta { name: String, val: u64 },
    }

    let w = MyEnum::Alpha { id: 42 };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<MyEnum>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let id = ctx.field_get(&root, "id").unwrap();
    assert_eq!(ctx.read_u32(&id), 42);

    let (_variant, _data_off) = ctx.enum_variant(&root).unwrap();

    let w = MyEnum::Beta {
        name: "test".into(),
        val: 99,
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let name = ctx.field_get(&root, "name").unwrap();
    assert_eq!(ctx.read_str(&name), "test");
    let val = ctx.field_get(&root, "val").unwrap();
    assert_eq!(ctx.read_u64(&val), 99);
}

/// Regression test: a u64 field that follows a u32 in an enum variant needs
/// 4 bytes of alignment padding. The schema's offset_in_fixed must apply the
/// field's OWN alignment (matching the writer), otherwise the schema-driven
/// reader reads the u64 4 bytes early and renders `value << 32`.
#[test]
fn test_schema_context_enum_variant_u64_after_u32() {
    #[derive(MmapStruct, Debug, Clone, PartialEq)]
    enum MixedTask {
        Read {
            task_id: u64,
            begin_ns: u64,
            end_ns: u64,
            step: u32,
            layer: u16,
            eid: u16,
            ram_slot: u32,
            bytes: u64,
            is_prefetch: u32,
        },
    }

    #[derive(MmapStruct)]
    struct MixedTaskFile {
        tasks: Vec<MixedTask>,
    }

    let expected = MixedTask::Read {
        task_id: 6939,
        begin_ns: 41_185_329_442,
        end_ns: 41_185_342_832,
        step: 3,
        layer: 11,
        eid: 76,
        ram_slot: 211,
        bytes: 18_915_328,
        is_prefetch: 1,
    };
    let file = MixedTaskFile {
        tasks: vec![expected.clone()],
    };
    let data = write_to_buf_raw(&file);
    let schema = build_serialized_schema::<MixedTaskFile>();
    let schema_bytes = write_to_buf_raw(&schema);

    // The variant struct in the type table must record `bytes` at its aligned
    // offset: 3*u64 + u32 + u16 + u16 + u32 = 36, aligned to 8 -> 40.
    let root_idx = schema.root_type as usize;
    let mmapstruct_schema::schema::TypeDef::Struct { fields: file_fields, .. } = &schema.types[root_idx]
    else {
        panic!("expected root struct");
    };
    let mmapstruct_schema::schema::TypeDef::Array { elem_type, .. } = &schema.types[file_fields[0].type_index as usize]
    else {
        panic!("expected tasks array");
    };
    let mmapstruct_schema::schema::TypeDef::Enum { variant_type_indices, .. } =
        &schema.types[*elem_type as usize]
    else {
        panic!("expected enum");
    };
    let mmapstruct_schema::schema::TypeDef::Struct { fields, .. } =
        &schema.types[variant_type_indices[0] as usize]
    else {
        panic!("expected variant struct");
    };
    let off = fields
        .iter()
        .find(|f| f.name == "bytes")
        .expect("bytes field in variant")
        .offset_in_fixed;
    assert_eq!(off, 40, "u64 after u32 must be 8-aligned in the schema");

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, schema.root_type);

    let tasks = ctx.field_get(&root, "tasks").unwrap();
    let task0 = ctx.array_get(&tasks, 0).unwrap();
    assert_eq!(
        ctx.read_u64(&ctx.field_get(&task0, "task_id").unwrap()),
        6939
    );
    assert_eq!(
        ctx.read_u64(&ctx.field_get(&task0, "begin_ns").unwrap()),
        41_185_329_442
    );
    assert_eq!(
        ctx.read_u64(&ctx.field_get(&task0, "end_ns").unwrap()),
        41_185_342_832
    );
    assert_eq!(ctx.read_u32(&ctx.field_get(&task0, "step").unwrap()), 3);
    assert_eq!(ctx.read_u16(&ctx.field_get(&task0, "layer").unwrap()), 11);
    assert_eq!(ctx.read_u16(&ctx.field_get(&task0, "eid").unwrap()), 76);
    assert_eq!(ctx.read_u32(&ctx.field_get(&task0, "ram_slot").unwrap()), 211);
    assert_eq!(
        ctx.read_u64(&ctx.field_get(&task0, "bytes").unwrap()),
        18_915_328
    );
    assert_eq!(ctx.read_u32(&ctx.field_get(&task0, "is_prefetch").unwrap()), 1);
}

#[test]
fn test_schema_context_array() {
    #[derive(MmapStruct)]
    struct Item {
        id: u32,
        label: String,
    }

    #[derive(MmapStruct)]
    struct ItemFile {
        items: Vec<Item>,
    }

    let w = ItemFile {
        items: vec![
            Item {
                id: 10,
                label: "a".into(),
            },
            Item {
                id: 20,
                label: "b".into(),
            },
            Item {
                id: 30,
                label: "c".into(),
            },
        ],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<ItemFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let items = ctx.field_get(&root, "items").unwrap();
    assert_eq!(ctx.array_len(&items), 3);

    let elem0 = ctx.array_get(&items, 0).unwrap();
    let id0 = ctx.field_get(&elem0, "id").unwrap();
    assert_eq!(ctx.read_u32(&id0), 10);
    let label0 = ctx.field_get(&elem0, "label").unwrap();
    assert_eq!(ctx.read_str(&label0), "a");

    let elem2 = ctx.array_get(&items, 2).unwrap();
    let id2 = ctx.field_get(&elem2, "id").unwrap();
    assert_eq!(ctx.read_u32(&id2), 30);
}

#[test]
fn test_schema_context_fixed_array() {
    use mmapstruct_schema::RuntimeSchemaCtx;
    use mmapstruct_schema::SchemaValue;

    #[derive(MmapStruct, Debug)]
    struct FixedItem {
        id: u64,
        #[mmap(byref, dedup)]
        name: String,
    }

    #[derive(MmapStruct, Debug)]
    struct FixedFile {
        #[fixed]
        items: Vec<FixedItem>,
    }

    let w = FixedFile {
        items: vec![
            FixedItem { id: 10, name: "alpha".into() },
            FixedItem { id: 20, name: "beta".into() },
            FixedItem { id: 30, name: "alpha".into() },
        ],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<FixedFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let items = ctx.field_get(&root, "items").unwrap();
    assert_eq!(ctx.array_len(&items), 3);

    let elem0 = ctx.array_get(&items, 0).unwrap();
    let id0 = ctx.field_get(&elem0, "id").unwrap();
    assert_eq!(ctx.read_u64(&id0), 10);
    let name0 = ctx.field_get(&elem0, "name").unwrap();
    assert_eq!(ctx.read_str(&name0), "alpha");

    let elem1 = ctx.array_get(&items, 1).unwrap();
    let name1 = ctx.field_get(&elem1, "name").unwrap();
    assert_eq!(ctx.read_str(&name1), "beta");

    let elem2 = ctx.array_get(&items, 2).unwrap();
    let id2 = ctx.field_get(&elem2, "id").unwrap();
    assert_eq!(ctx.read_u64(&id2), 30);
    let name2 = ctx.field_get(&elem2, "name").unwrap();
    assert_eq!(ctx.read_str(&name2), "alpha");
}

#[test]
fn test_schema_context_read_as() {
    use mmapstruct_schema::RuntimeSchemaCtx;
    use mmapstruct_schema::SchemaValue;

    #[derive(MmapStruct)]
    struct Stats {
        avg: f64,
        count: u32,
        name: String,
    }

    let w = Stats {
        avg: 3.5,
        count: 100,
        name: "test".into(),
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<Stats>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let avg = ctx.field_get(&root, "avg").unwrap();
    assert!((ctx.read_f64(&avg) - 3.5).abs() < 1e-10);
    assert!((ctx.read_as_f64(&avg).unwrap() - 3.5).abs() < 1e-10);

    let count = ctx.field_get(&root, "count").unwrap();
    assert_eq!(ctx.read_u32(&count), 100);
    assert_eq!(ctx.read_as_i64(&count), Some(100i64));

    let name = ctx.field_get(&root, "name").unwrap();
    assert_eq!(ctx.read_str(&name), "test");
}

#[test]
fn test_schema_context_byref_string() {
    #[derive(MmapStruct)]
    struct Rec {
        name: String,
    }

    let w = Rec {
        name: "world".into(),
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<Rec>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let name = ctx.field_get(&root, "name").unwrap();
    assert_eq!(ctx.read_str(&name), "world");
}

#[test]
fn test_schema_context_option_none() {
    #[derive(MmapStruct)]
    struct WithOpt {
        label: Option<String>,
    }

    let w = WithOpt { label: None };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<WithOpt>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let label = ctx.field_get(&root, "label").unwrap();
    assert_eq!(label.mmap_begin, label.mmap_end);
}

#[test]
fn test_schema_context_option_some() {
    #[derive(MmapStruct)]
    struct WithOpt {
        label: Option<String>,
    }

    let w = WithOpt {
        label: Some("present".into()),
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<WithOpt>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let label = ctx.field_get(&root, "label").unwrap();
    assert_eq!(ctx.read_str(&label), "present");
}

#[test]
fn test_schema_format_value_struct() {
    #[derive(MmapStruct)]
    struct Point {
        x: u64,
        y: String,
    }

    let pt = Point {
        x: 42,
        y: "hello".into(),
    };
    let data = mmapstruct::write_to_buf_raw(&pt);
    let schema = build_serialized_schema::<Point>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let s = format_schema_value(&ctx, &root);
    assert_eq!(s, "{\n  \"x\": 42,\n  \"y\": \"hello\"\n}");
}

#[test]
fn test_schema_format_value_enum() {
    #[derive(MmapStruct)]
    enum MyEnum {
        Alpha { id: u32 },
        Beta { name: String, val: u64 },
    }

    let w = MyEnum::Beta {
        name: "test".into(),
        val: 99,
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<MyEnum>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let s = format_schema_value(&ctx, &root);
    assert_eq!(s, "MyEnum::Beta {\n  \"name\": \"test\",\n  \"val\": 99\n}");

    // Verify data_off > 0 for this enum (u64 fields → alignment 8 → skip
    // 4 bytes after u32 discriminant). The formatter delegates field lookup
    // to field_get which handles the shift internally.
    let (_variant_type_idx, data_off) = ctx.enum_variant(&root).unwrap();
    assert!(data_off > 0, "enum with u64 field should have non-zero data offset");
}

#[test]
fn test_schema_format_value_array() {
    #[derive(MmapStruct)]
    struct Item {
        id: u32,
        label: String,
    }

    #[derive(MmapStruct)]
    struct ItemFile {
        items: Vec<Item>,
    }

    let w = ItemFile {
        items: vec![
            Item {
                id: 1,
                label: "a".into(),
            },
            Item {
                id: 2,
                label: "b".into(),
            },
        ],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<ItemFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let items = ctx.field_get(&root, "items").unwrap();
    let s = format_schema_value(&ctx, &items);
    assert_eq!(
        s,
        "[\n  {\n    \"id\": 1,\n    \"label\": \"a\"\n  },\n  {\n    \"id\": 2,\n    \"label\": \"b\"\n  }\n]"
    );
}

#[test]
fn test_schema_deep_nested_enum_field() {
    // Deep nesting: struct → inline-dyn struct → inline-dyn struct → enum → variant → inline-dyn String
    #[derive(MmapStruct)]
    struct DeepA {
        tag: u64,
        child: DeepB,
    }
    #[derive(MmapStruct)]
    struct DeepB {
        val: u64,
        child: DeepC,
    }
    #[derive(MmapStruct)]
    struct DeepC {
        data: String,
        inner: InnerEnum,
    }
    #[derive(MmapStruct)]
    enum InnerEnum {
        Alpha { x: u64 },
        Beta { nested: DeeperStruct },
    }
    #[derive(MmapStruct)]
    struct DeeperStruct {
        message: String,
        extra: u64,
    }

    let w = DeepA {
        tag: 1,
        child: DeepB {
            val: 2,
            child: DeepC {
                data: "hello".into(),
                inner: InnerEnum::Beta {
                    nested: DeeperStruct {
                        message: "deep".into(),
                        extra: 99,
                    },
                },
            },
        },
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<DeepA>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);
    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    // Navigate: root → child → child → inner → Beta → nested → message
    let fv = ctx.field_get(&root, "child").unwrap();
    let fv = ctx.field_get(&fv, "child").unwrap();
    let fv = ctx.field_get(&fv, "inner").unwrap();
    let fv = ctx.field_get(&fv, "nested").unwrap();
    let msg = ctx.field_get(&fv, "message").unwrap();
    assert_eq!(
        ctx.read_str(&msg),
        "deep",
        "deep nested string via enum variant"
    );
    assert_eq!(
        ctx.read_u64(&ctx.field_get(&fv, "extra").unwrap()),
        99,
        "deep nested extra via enum variant"
    );

    // Also test formatting through the deep path
    let fmt = format_schema_value(&ctx, &root);
    assert!(
        fmt.contains("\"message\": \"deep\""),
        "format should contain deep nested field"
    );
}

#[test]
fn test_schema_query_path() {
    #[derive(MmapStruct)]
    struct Item {
        id: u32,
        label: String,
    }
    #[derive(MmapStruct)]
    struct ItemFile {
        items: Vec<Item>,
    }

    let w = ItemFile {
        items: vec![
            Item {
                id: 10,
                label: "x".into(),
            },
            Item {
                id: 20,
                label: "y".into(),
            },
        ],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<ItemFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let v = navigate_path(&ctx, &root, "/items[0]/label").unwrap();
    assert_eq!(ctx.read_str(&v), "x");

    let v = navigate_path(&ctx, &root, "/items[1]/id").unwrap();
    assert_eq!(ctx.read_u32(&v), 20);
}

#[test]
fn test_schema_fixed_array() {
    use mmapstruct_schema::RuntimeSchemaCtx;
    use mmapstruct_schema::SchemaValue;
    use bytemuck::{Pod, Zeroable};

    #[derive(MmapStruct, Clone, Copy, Pod, Zeroable)]
    #[repr(C)]
    struct WithArray {
        id: u32,
        mat: [[u32; 4]; 4],
    }

    let w = WithArray {
        id: 42,
        mat: [[1, 2, 3, 4], [5, 6, 7, 8], [9, 10, 11, 12], [13, 14, 15, 16]],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<WithArray>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    // Verify id field
    let id_val = ctx.field_get(&root, "id").unwrap();
    assert_eq!(ctx.read_u32(&id_val), 42);

    // Verify matrix field — top-level array
    let mat_val = ctx.field_get(&root, "mat").unwrap();
    assert_eq!(ctx.array_len(&mat_val), 4); // 4 rows

    // Access row 2 (index 2)
    let row = ctx.array_get(&mat_val, 2).unwrap();
    assert_eq!(ctx.array_len(&row), 4); // 4 cols per row

    // Access element [2][1] = 10
    let elem = ctx.array_get(&row, 1).unwrap();
    assert_eq!(ctx.read_u32(&elem), 10);

    // Quick format check
    let s = format_schema_value(&ctx, &root);
    assert!(s.contains("\"id\": 42"), "missing id: {s}");
    assert!(s.contains("[\n      1,\n      2,\n      3,\n      4\n    ]"), "missing row 0: {s}");
    assert!(s.contains("[\n      13,\n      14,\n      15,\n      16\n    ]"), "missing row 3: {s}");
}

#[test]
fn test_schema_fixed_array_top_level() {
    use bytemuck::{Pod, Zeroable};

    #[derive(MmapStruct, Clone, Copy, Pod, Zeroable)]
    #[repr(C)]
    struct SimpleArray {
        arr: [u32; 4],
    }

    let w = SimpleArray { arr: [10, 20, 30, 40] };
    let data = write_to_buf_raw(&w);
    let schema = build_serialized_schema::<SimpleArray>();
    let schema_bytes = write_to_buf_raw(&schema);

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, *<mmapstruct_schema::schema::Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len()).root_type);

    let arr_val = ctx.field_get(&root, "arr").unwrap();
    assert_eq!(ctx.array_len(&arr_val), 4);
    assert_eq!(ctx.read_u32(&ctx.array_get(&arr_val, 0).unwrap()), 10);
    assert_eq!(ctx.read_u32(&ctx.array_get(&arr_val, 3).unwrap()), 40);
}

#[test]
fn test_schema_fixed_array_podstruct() {
    use bytemuck::{Pod, Zeroable};

    #[derive(MmapStruct, Clone, Copy, Pod, Zeroable, Default)]
    #[repr(C)]
    struct PodPoint {
        x: f32,
        y: f32,
    }

    #[derive(MmapStruct)]
    struct WithPoints {
        pts: [PodPoint; 3],
        tag: u32,
    }

    let w = WithPoints {
        pts: [PodPoint { x: 1.0, y: 2.0 }, PodPoint { x: 3.0, y: 4.0 }, PodPoint { x: 5.0, y: 6.0 }],
        tag: 99,
    };
    let data = write_to_buf_raw(&w);
    let schema = build_serialized_schema::<WithPoints>();
    let schema_bytes = write_to_buf_raw(&schema);

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, *<mmapstruct_schema::schema::Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len()).root_type);

    let pts_val = ctx.field_get(&root, "pts").unwrap();
    assert_eq!(ctx.array_len(&pts_val), 3);

    let p0 = ctx.array_get(&pts_val, 0).unwrap();
    assert!((ctx.read_f32(&p0) - 1.0).abs() < 1e-6);

    let tag_val = ctx.field_get(&root, "tag").unwrap();
    assert_eq!(ctx.read_u32(&tag_val), 99);
}

#[test]
fn test_array_get_out_of_range_returns_none() {
    #[derive(MmapStruct)]
    struct ArrFile {
        nums: Vec<u32>,
    }

    let w = ArrFile {
        nums: vec![10, 20, 30],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<ArrFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let arr = ctx.field_get(&root, "nums").unwrap();
    assert_eq!(ctx.array_len(&arr), 3);

    // In-bounds reads work.
    assert_eq!(ctx.read_u32(&ctx.array_get(&arr, 0).unwrap()), 10);
    assert_eq!(ctx.read_u32(&ctx.array_get(&arr, 2).unwrap()), 30);

    // Out-of-range returns None instead of a garbage SchemaValue
    // (consistency with the static SchemaCtx::array_get).
    assert!(ctx.array_get(&arr, 3).is_none());
    assert!(ctx.array_get(&arr, 1000).is_none());
}

/// Regression (bugs/enum-podcustom-field-layout-8-vs-write-4.md): enum
/// variant with a `#[pod]` custom field (real size 4) followed by more
/// fixed fields — the serialized schema's offsets must match the writer's
/// real layout, not the old macro-time guess of 8. The last record's
/// trailing `u8` sits at the payload edge (panic on overread).
#[test]
fn test_schema_enum_podcustom_last_record() {
    #[derive(MmapStruct, Debug, Clone, Copy, PartialEq, Eq)]
    #[repr(u32)]
    enum Src {
        A = 0,
        B = 1,
    }

    #[derive(MmapStruct)]
    enum Rec {
        Direct {
            id: u64,
            #[pod]
            src: Src,
            half: u8,
        },
        Other {
            id: u64,
        },
    }

    #[derive(MmapStruct)]
    struct Log {
        recs: Vec<Rec>,
    }

    let w = Log {
        recs: vec![
            Rec::Direct { id: 1, src: Src::B, half: 7 },
            Rec::Other { id: 2 },
            Rec::Direct { id: 3, src: Src::A, half: 9 },
        ],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<Log>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);

    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let recs = ctx.field_get(&root, "recs").unwrap();
    assert_eq!(ctx.array_len(&recs), 3);

    // Mid-array record: fields after the #[pod] custom field must be at
    // their real offsets (the old bug read `half` +4 past its position,
    // silently returning the next record's outer discriminant byte).
    let r0 = ctx.array_get(&recs, 0).unwrap();
    assert_eq!(ctx.read_u64(&ctx.field_get(&r0, "id").unwrap()), 1);
    assert_eq!(ctx.read_u32(&ctx.field_get(&r0, "src").unwrap()), 1);
    assert_eq!(ctx.read_u8(&ctx.field_get(&r0, "half").unwrap()), 7);

    // Last record: the trailing u8 ends exactly at the payload edge.
    let r2 = ctx.array_get(&recs, 2).unwrap();
    assert_eq!(ctx.read_u64(&ctx.field_get(&r2, "id").unwrap()), 3);
    assert_eq!(ctx.read_u32(&ctx.field_get(&r2, "src").unwrap()), 0);
    assert_eq!(ctx.read_u8(&ctx.field_get(&r2, "half").unwrap()), 9);
}

// ---------------------------------------------------------------------------
// Option<T> inner alignment: the schema must mirror the writer's real
// `align_of::<T>()`, not a hardcoded 8.
// ---------------------------------------------------------------------------

#[test]
fn test_schema_option_enum_align4_inner_offset() {
    // Enum with only u32 fields: Rust align_of = 4. `Option<Small>` inner
    // data starts 4 bytes after the present byte (writer: align_to(4)); the
    // old hardcoded align-8 schema read the discriminant 4 bytes too late.
    #[derive(MmapStruct, Debug, PartialEq)]
    enum Small {
        A { x: u32 },
        B { y: u32 },
    }

    #[derive(MmapStruct, Debug)]
    struct OptEnumFile {
        opt: Option<Small>,
    }

    let w = OptEnumFile {
        opt: Some(Small::A { x: 7 }),
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<OptEnumFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);
    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let opt = ctx.field_get(&root, "opt").unwrap();
    // resolve_field_value unwraps Some: variant A, x = 7.
    let x = ctx.field_get(&opt, "x").unwrap();
    assert_eq!(ctx.read_u32(&x), 7);

    // None case still resolves to a zero-length value.
    let w2 = OptEnumFile { opt: None };
    let data2 = mmapstruct::write_to_buf_raw(&w2);
    let ctx2 = RuntimeSchemaCtx::new(&data2, &schema_bytes);
    let root2 = SchemaValue::new(0, data2.len() as u64, root_type);
    let opt2 = ctx2.field_get(&root2, "opt").unwrap();
    assert!(ctx2.field_get(&opt2, "x").is_none());
}

#[test]
fn test_schema_option_pod_struct_align4() {
    // Pod struct with align 4 (two u32s): Option<S> inner data at +4; the old
    // size-based heuristic (size 8 → align 8) overestimated.
    #[derive(MmapStruct, Debug, PartialEq, Eq, Ord, PartialOrd, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default)]
    #[repr(C)]
    struct Pair {
        a: u32,
        b: u32,
    }

    #[derive(MmapStruct, Debug)]
    struct OptPodFile {
        opt: Option<Pair>,
    }

    let w = OptPodFile {
        opt: Some(Pair { a: 11, b: 22 }),
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<OptPodFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);
    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let opt = ctx.field_get(&root, "opt").unwrap();
    assert_eq!(ctx.read_u32(&ctx.field_get(&opt, "a").unwrap()), 11);
    assert_eq!(ctx.read_u32(&ctx.field_get(&opt, "b").unwrap()), 22);
}

// ---------------------------------------------------------------------------
// Pod enum discriminant bounds: disc_size (repr(u8) → 1 byte) must be used
// for the discriminant read, and a 1-byte enum at the very end of the buffer
// must not panic or misread the neighbouring element's bytes.
// ---------------------------------------------------------------------------

#[test]
fn test_schema_pod_enum_disc_size_bounds() {
    #[derive(MmapStruct, Debug, PartialEq, Clone, Copy)]
    #[repr(u8)]
    enum Mode {
        Off = 0,
        On = 1,
        Auto = 2,
    }

    #[derive(MmapStruct, Debug)]
    struct ModeFile {
        #[pod]
        modes: Vec<Mode>,
    }

    let w = ModeFile {
        modes: vec![Mode::On, Mode::Auto],
    };
    let data = mmapstruct::write_to_buf_raw(&w);
    let schema = build_serialized_schema::<ModeFile>();
    let schema_bytes = mmapstruct::write_to_buf_raw(&schema);
    let schema_reader = <mmapstruct_schema::schema::Schema as MmapRead>::read(
        &schema_bytes,
        0..schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    let ctx = RuntimeSchemaCtx::new(&data, &schema_bytes);
    let root = SchemaValue::new(0, data.len() as u64, root_type);

    let modes = ctx.field_get(&root, "modes").unwrap();
    assert_eq!(ctx.array_len(&modes), 2);

    // The last element is a single byte at the payload edge: reading its
    // variant name must not over-read (the old code read a full u32).
    let last = ctx.array_get(&modes, 1).unwrap();
    let (vt_last, _) = ctx.enum_variant(&last).unwrap();
    let TypeDefReader::Struct { name, .. } = ctx.type_reader(vt_last) else {
        panic!("expected variant struct type")
    };
    assert_eq!(*name, "Mode::Auto");

    let first = ctx.array_get(&modes, 0).unwrap();
    let (vt_first, _) = ctx.enum_variant(&first).unwrap();
    let TypeDefReader::Struct { name, .. } = ctx.type_reader(vt_first) else {
        panic!("expected variant struct type")
    };
    assert_eq!(*name, "Mode::On");
}
