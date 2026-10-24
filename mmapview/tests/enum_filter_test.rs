//! Filters over variant-specific fields of a `Vec<Enum>` via the
//! `RuntimeSchemaCtx` path — the same path `mmapview q` uses (embedded
//! serialized schema, no compile-time `GetSchema` navigation).

use mmapstruct::traits::MmapRead;
use mmapstruct::MmapStruct;
use mmapstruct_schema::builder::write_to_file_with_embedded_schema;
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::{Schema, SchemaValue};
use mmapview::{query_from_generic, EvalResult};

#[derive(MmapStruct, Debug)]
enum Event {
    Click {
        id: u32,
        x: u32,
        y: u32,
    },
    Key {
        id: u32,
        code: u32,
    },
    Probe {
        id: u32,
        test_field: u32,
        note: String,
    },
}

#[derive(MmapStruct, Debug)]
struct EventFile {
    events: Vec<Event>,
}

fn sample() -> EventFile {
    EventFile {
        events: vec![
            Event::Click {
                id: 1,
                x: 10,
                y: 20,
            },
            Event::Probe {
                id: 10,
                test_field: 5,
                note: "a".into(),
            },
            Event::Key { id: 2, code: 7 },
            Event::Probe {
                id: 11,
                test_field: 0,
                note: "b".into(),
            },
            Event::Probe {
                id: 12,
                test_field: 42,
                note: "c".into(),
            },
        ],
    }
}

/// Mirror `mmapview q` loading: v2 file with embedded schema → payload +
/// serialized schema → `RuntimeSchemaCtx` rooted at the payload start.
fn setup(file_bytes: &[u8]) -> (RuntimeSchemaCtx<'static>, SchemaValue) {
    let r = mmapview::schema::resolve_schema(file_bytes).unwrap();
    let schema_bytes: &'static [u8] = Box::leak(r.schema_bytes.clone().into_boxed_slice());
    let payload: &'static [u8] = Box::leak(
        file_bytes[r.payload_range.clone()]
            .to_vec()
            .into_boxed_slice(),
    );
    let schema_reader = <Schema as MmapRead>::read(schema_bytes, 0..schema_bytes.len());
    let root_type = *schema_reader.root_type;
    let ctx = RuntimeSchemaCtx::new(payload, schema_bytes);
    let root = SchemaValue::new(0, payload.len() as u64, root_type);
    (ctx, root)
}

#[test]
fn test_runtime_schema_filter_enum_vec_variant_field() {
    let w = sample();
    let file = write_to_file_with_embedded_schema(&w, false);
    let (ctx, root) = setup(&file);

    // Sanity: schema navigation reaches the array and its elements.
    let ev = ctx.field_get(&root, "events").unwrap();
    assert_eq!(ctx.array_len(&ev), 5);

    // Variant-specific field present only on Probe: jq-style — elements
    // whose active variant lacks the field simply do not match.
    let result = query_from_generic("/events[?test_field>0]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 2);
    for r in &result {
        let EvalResult::Buf(v) = r else {
            panic!("expected Buf")
        };
        let tf = ctx.read_u32(&ctx.field_get(v, "test_field").unwrap());
        assert!(tf > 0, "expected positive test_field, got {tf}");
    }

    // Equality on a variant field.
    let result = query_from_generic("/events[?test_field=5]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 1);
    let EvalResult::Buf(v) = &result[0] else {
        panic!("expected Buf")
    };
    assert_eq!(ctx.read_u32(&ctx.field_get(v, "test_field").unwrap()), 5);

    // AND: common field + variant-specific field on the same element.
    let result = query_from_generic("/events[?id>10&test_field>0]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 1);
    let EvalResult::Buf(v) = &result[0] else {
        panic!("expected Buf")
    };
    assert_eq!(ctx.read_u32(&ctx.field_get(v, "id").unwrap()), 12);

    // OR across different variants.
    let result = query_from_generic("/events[?code=7|test_field=42]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 2);

    // Field missing in every variant: no matches, no error.
    let result = query_from_generic("/events[?no_such_field>0]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 0);

    // select() pipe with a variant field.
    let result = query_from_generic("/events | select(.test_field>0)", &ctx, &root).unwrap();
    assert_eq!(result.len(), 2);

    // Consecutive filters intersect on the same elements.
    let result = query_from_generic("/events[?test_field>0][?id=12]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 1);
}

#[test]
fn test_runtime_schema_aggregate_enum_vec_variant_field() {
    let w = sample();
    let file = write_to_file_with_embedded_schema(&w, false);
    let (ctx, root) = setup(&file);

    // count() sees every element.
    let result = query_from_generic("/events | count()", &ctx, &root).unwrap();
    assert!(matches!(result[0], EvalResult::U64(5)));

    // max/avg skip elements whose variant lacks the field.
    let result = query_from_generic("/events | max(.test_field)", &ctx, &root).unwrap();
    let EvalResult::F64(m) = result[0] else {
        panic!("expected F64")
    };
    assert_eq!(m, 42.0);

    let result = query_from_generic("/events | avg(.test_field)", &ctx, &root).unwrap();
    let EvalResult::F64(a) = result[0] else {
        panic!("expected F64")
    };
    // sample() has 5 events: Click, Probe(5), Key, Probe(0), Probe(42).
    // avg over variant field: (5 + 0 + 42) / 3 = 15.667 (zero-valued Probe
    // counts; non-Probe elements are skipped).
    assert!((a - 47.0 / 3.0).abs() < 0.001);
}

#[test]
fn test_runtime_schema_direct_variant_field_access() {
    let w = sample();
    let file = write_to_file_with_embedded_schema(&w, false);
    let (ctx, root) = setup(&file);

    let ev = ctx.field_get(&root, "events").unwrap();

    // Variant-specific field: present only on Probe (disc 2).
    let click = ctx.array_get(&ev, 0).unwrap();
    assert!(ctx.field_get(&click, "test_field").is_none());

    let probe0 = ctx.array_get(&ev, 1).unwrap();
    assert_eq!(
        ctx.read_u32(&ctx.field_get(&probe0, "test_field").unwrap()),
        5
    );
    assert_eq!(ctx.read_u32(&ctx.field_get(&probe0, "id").unwrap()), 10);
    assert_eq!(ctx.read_str(&ctx.field_get(&probe0, "note").unwrap()), "a");

    let key = ctx.array_get(&ev, 2).unwrap();
    assert!(ctx.field_get(&key, "test_field").is_none());
    assert_eq!(ctx.read_u32(&ctx.field_get(&key, "code").unwrap()), 7);

    // Last element at the payload edge.
    let probe2 = ctx.array_get(&ev, 4).unwrap();
    assert_eq!(
        ctx.read_u32(&ctx.field_get(&probe2, "test_field").unwrap()),
        42
    );
    assert_eq!(ctx.read_str(&ctx.field_get(&probe2, "note").unwrap()), "c");

    assert!(ctx.field_get(&probe2, "no_such").is_none());
}

#[test]
fn test_runtime_schema_filter_dollar_variant() {
    let w = sample();
    let file = write_to_file_with_embedded_schema(&w, false);
    let (ctx, root) = setup(&file);

    // Short name. sample() has 3 Probe elements.
    let result = query_from_generic("/events[?$variant=Probe]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 3);
    let EvalResult::Buf(v) = &result[0] else {
        panic!("expected Buf")
    };
    assert_eq!(ctx.read_u32(&ctx.field_get(v, "id").unwrap()), 10);

    // Full schema name (quoted).
    let result = query_from_generic(r#"/events[?$variant="Event::Key"]"#, &ctx, &root).unwrap();
    assert_eq!(result.len(), 1);
    let EvalResult::Buf(v) = &result[0] else {
        panic!("expected Buf")
    };
    assert_eq!(ctx.read_u32(&ctx.field_get(v, "code").unwrap()), 7);

    // Contains on the short form.
    let result = query_from_generic("/events[?$variant~Prob]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 3);

    // != matches non-matching variants (Click + Key).
    let result = query_from_generic("/events[?$variant!=Probe]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 2);

    // Combined with a regular field.
    let result = query_from_generic("/events[?$variant=Click&id>0]", &ctx, &root).unwrap();
    assert_eq!(result.len(), 1);

    // select() pipe form.
    let result = query_from_generic("/events | select(.$variant=Key)", &ctx, &root).unwrap();
    assert_eq!(result.len(), 1);
}

#[test]
fn test_runtime_schema_option_enum_field_navigation() {
    // Option<enum> with a small (align-4) enum: the schema must unwrap the
    // Some case at the writer's real alignment (align_of::<Small>() = 4).
    #[derive(MmapStruct, Debug)]
    enum Small {
        A { x: u32 },
        B { y: u32 },
    }

    #[derive(MmapStruct, Debug)]
    struct OptFile {
        opt: Option<Small>,
    }

    // Some case.
    let w = OptFile {
        opt: Some(Small::B { y: 33 }),
    };
    let file = write_to_file_with_embedded_schema(&w, false);
    let (ctx, root) = setup(&file);
    let opt = ctx.field_get(&root, "opt").unwrap();
    let y = ctx.field_get(&opt, "y").unwrap();
    assert_eq!(ctx.read_u32(&y), 33);
    // The inactive variant's field is not present.
    assert!(ctx.field_get(&opt, "x").is_none());

    // None case.
    let w2 = OptFile { opt: None };
    let file2 = write_to_file_with_embedded_schema(&w2, false);
    let (ctx2, root2) = setup(&file2);
    let opt2 = ctx2.field_get(&root2, "opt").unwrap();
    assert!(ctx2.field_get(&opt2, "y").is_none());
}
