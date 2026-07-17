use bytemuck::{Pod, Zeroable};
use mmapstruct::file_reader::{BufReader, MmapReader};
use mmapstruct::indexed_array::{write_indexed_array, IndexedArrayReader};
use mmapstruct::pod_array::write_pod_array;
use mmapstruct::{write_to_buf, write_to_file, MmapStruct, WriteContext};
use tempfile::NamedTempFile;

/// Example from the spec.
#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct Point {
    x: u32,
    y: u32,
}

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct Point3d {
    x: f32,
    y: f32,
    z: f32,
}

#[derive(MmapStruct, Debug)]
struct MyFileFmt {
    vals: Vec<u32>,
    #[pod]
    points: Vec<Point>,
}

#[derive(MmapStruct, Debug)]
struct MyFile {
    id: u32,
    name: String,
    vals: Vec<u32>,
}

#[derive(MmapStruct, Debug)]
struct NamedPoints {
    #[pod]
    point: Point,
    labels: Vec<String>,
}

#[test]
fn test_pod_struct_write_read() {
    let tmp = NamedTempFile::new().unwrap();
    let pt = Point { x: 10, y: 20 };
    write_to_file(&pt, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<Point>();
    assert_eq!(reader.x, 10);
    assert_eq!(reader.y, 20);
}

#[test]
fn test_mixed_struct_write_read() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = MyFile::default();
    w.id = 42;
    w.name = "hello".to_string();
    w.vals = vec![1u32, 2, 3, 4, 5];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<MyFile>();
    assert_eq!(*reader.id, 42);

    assert_eq!(reader.vals, &[1u32, 2, 3, 4, 5]);
}

#[test]
fn test_named_points_write_read() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = NamedPoints::default();
    w.point = Point { x: 100, y: 200 };
    w.labels = vec![
        "first".to_string(),
        "second".to_string(),
        "third".to_string(),
    ];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<NamedPoints>();
    assert_eq!(reader.point.x, 100);
    assert_eq!(reader.point.y, 200);

    let labels: Vec<String> = reader.labels.iter().map(|r| r.to_string()).collect();
    assert_eq!(labels, vec!["first", "second", "third"]);
}

#[test]
fn test_free_functions() {
    let mut ctx = WriteContext::new();
    write_pod_array::<u32>(&[10, 20, 30], &mut ctx);
    let buf = ctx.finish();
    assert!(!buf.is_empty());

    let items = vec!["a".to_string(), "bb".to_string()];
    let mut ctx2 = WriteContext::new();
    write_indexed_array(&items, &mut ctx2);
    let buf2 = ctx2.finish();
    drop(items);

    let data = buf2.as_slice();
    let range = 0..data.len();
    let reader: IndexedArrayReader<String> = IndexedArrayReader::new(data, range.into());
    assert_eq!(reader.len(), 2);

    assert_eq!(reader.get(0).unwrap(), "a");
    assert_eq!(reader.get(1).unwrap(), "bb");
}

#[derive(MmapStruct, Debug)]
struct WithOption {
    id: u32,
    maybe_name: Option<String>,
    tags: Vec<String>,
}

#[derive(MmapStruct, Debug)]
struct WithOptionByRef {
    id: u32,
    #[mmap(byref)]
    maybe_name: Option<String>,
}

#[derive(MmapStruct, Debug)]
struct WithByRef {
    id: u32,
    #[mmap(byref)]
    name: String,
    #[mmap(byref)]
    descr: String,
}

#[test]
fn test_spec_example() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = MyFileFmt::default();
    w.vals = vec![1, 2, 3, 4, 5];
    w.points = vec![Point { x: 10, y: 20 }, Point { x: 30, y: 40 }];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = MmapReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<MyFileFmt>();

    assert_eq!(reader.vals, &[1u32, 2, 3, 4, 5]);
    assert_eq!(
        reader.points,
        &[Point { x: 10, y: 20 }, Point { x: 30, y: 40 }]
    );
}

#[test]
fn test_option_write_read() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = WithOption::default();
    w.id = 1;
    w.maybe_name = Some("present".to_string());
    w.tags = vec!["a".to_string()];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<WithOption>();
    assert_eq!(*reader.id, 1);
    assert!(reader.maybe_name.is_some());
    assert_eq!(reader.maybe_name.unwrap(), "present");

    let tmp2 = NamedTempFile::new().unwrap();
    let mut w2 = WithOption::default();
    w2.id = 2;
    w2.maybe_name = None;
    w2.tags = vec!["b".to_string()];
    write_to_file(&w2, tmp2.path(), false).unwrap();

    let r2 = BufReader::from_file(tmp2.path()).unwrap();
    let reader2 = r2.read::<WithOption>();
    assert_eq!(*reader2.id, 2);
    assert!(reader2.maybe_name.is_none());
}

#[test]
fn test_option_byref_write_read() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = WithOptionByRef::default();
    w.id = 1;
    w.maybe_name = Some("Alice".to_string());
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<WithOptionByRef>();
    assert_eq!(*reader.id, 1);
    assert_eq!(reader.maybe_name, Some("Alice"));

    let tmp2 = NamedTempFile::new().unwrap();
    let mut w2 = WithOptionByRef::default();
    w2.id = 2;
    w2.maybe_name = None;
    write_to_file(&w2, tmp2.path(), false).unwrap();

    let r2 = BufReader::from_file(tmp2.path()).unwrap();
    let reader2 = r2.read::<WithOptionByRef>();
    assert_eq!(*reader2.id, 2);
    assert!(reader2.maybe_name.is_none());
}

#[test]
fn test_option_byref_raw_bytes() {
    let mut w = WithOptionByRef::default();
    w.id = 1;
    w.maybe_name = Some("Alice".to_string());
    let buf_some = mmapstruct::write_to_buf_raw(&w);

    w.maybe_name = None;
    let buf_none = mmapstruct::write_to_buf_raw(&w);

    // WithOptionByRef: id(u32)=4, ByRefRange(8) = 12 bytes fixed section
    let br_some = mmapstruct::traits::ByRefRange {
        begin: u32::from_le_bytes(buf_some[4..8].try_into().unwrap()),
        end: u32::from_le_bytes(buf_some[8..12].try_into().unwrap()),
    };
    // ByRefRange for Some("Alice") should NOT be {0,0}
    assert!(
        br_some.begin != 0 || br_some.end != 0,
        "Some case should have non-zero ByRefRange"
    );

    // postponed data at br_some.begin..br_some.end should be raw "Alice\0" WITHOUT present byte
    let postponed_some = &buf_some[br_some.begin as usize..br_some.end as usize];
    // Should be [b'A', b'l', b'i', c', b'e'] — NO leading 1 (present byte)
    assert_eq!(
        postponed_some, b"Alice",
        "Option<byref> Some should write inner value directly, no present byte"
    );

    // None case: ByRefRange should be {0, 0}
    let br_none = mmapstruct::traits::ByRefRange {
        begin: u32::from_le_bytes(buf_none[4..8].try_into().unwrap()),
        end: u32::from_le_bytes(buf_none[8..12].try_into().unwrap()),
    };
    assert_eq!(
        br_none.begin, 0,
        "None case should have ByRefRange.begin == 0"
    );
    assert_eq!(br_none.end, 0, "None case should have ByRefRange.end == 0");
}

#[test]
fn test_byref_write_read() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = WithByRef::default();
    w.id = 7;
    w.name = "Alice".to_string();
    w.descr = "Long description".to_string();
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<WithByRef>();
    assert_eq!(*reader.id, 7);
    assert_eq!(reader.name, "Alice");
    assert_eq!(reader.descr, "Long description");
}

#[test]
fn test_mmap_read_write_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();

    let mut w = MyFile::default();
    w.id = 99;
    w.name = "world".to_string();
    w.vals = vec![10u32, 20];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<MyFile>();
    assert_eq!(*reader.id, 99);
    assert_eq!(reader.name, "world");
    assert_eq!(reader.vals, &[10u32, 20]);
}

#[test]
fn test_into_conversion_byref_option() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = WithOptionByRef::default();
    w.id = 7;
    w.maybe_name = Some("byref".to_string());
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<WithOptionByRef>();
    let owned: WithOptionByRef = reader.into();
    assert_eq!(owned.id, 7);
    assert_eq!(owned.maybe_name, Some("byref".to_string()));

    let tmp2 = NamedTempFile::new().unwrap();
    let mut w2 = WithOptionByRef::default();
    w2.id = 8;
    w2.maybe_name = None;
    write_to_file(&w2, tmp2.path(), false).unwrap();

    let r2 = BufReader::from_file(tmp2.path()).unwrap();
    let reader2 = r2.read::<WithOptionByRef>();
    let owned2: WithOptionByRef = reader2.into();
    assert_eq!(owned2.id, 8);
    assert_eq!(owned2.maybe_name, None);
}

#[test]
fn test_into_conversion() {
    let tmp = NamedTempFile::new().unwrap();

    let mut w = NamedPoints::default();
    w.point = Point { x: 1, y: 2 };
    w.labels = vec!["a".to_string(), "b".to_string()];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<NamedPoints>();
    let owned: NamedPoints = reader.into();
    assert_eq!(owned.point, Point { x: 1, y: 2 });
    assert_eq!(owned.labels, vec!["a", "b"]);
}

#[test]
fn test_into_conversion_option() {
    let tmp = NamedTempFile::new().unwrap();

    let mut w = WithOption::default();
    w.id = 1;
    w.maybe_name = Some("hello".to_string());
    w.tags = vec!["x".to_string()];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<WithOption>();
    let owned: WithOption = reader.into();
    assert_eq!(owned.id, 1);
    assert_eq!(owned.maybe_name, Some("hello".to_string()));
    assert_eq!(owned.tags, vec!["x"]);
}

#[test]
fn test_mmap_reader() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = MyFile::default();
    w.id = 42;
    w.name = "mmap_reader".to_string();
    w.vals = vec![1u32, 2, 3];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = MmapReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<MyFile>();
    assert_eq!(*reader.id, 42);
    assert_eq!(reader.name, "mmap_reader");
    assert_eq!(reader.vals, &[1u32, 2, 3]);
}

#[test]
fn test_buf_reader() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = NamedPoints::default();
    w.point = Point { x: 10, y: 20 };
    w.labels = vec!["alpha".to_string(), "beta".to_string()];
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<NamedPoints>();
    assert_eq!(reader.point.x, 10);
    assert_eq!(reader.point.y, 20);
    let labels: Vec<String> = reader.labels.iter().map(|s| s.to_string()).collect();
    assert_eq!(labels, vec!["alpha", "beta"]);
}

#[test]
fn test_buf_reader_new() {
    let buf = write_to_buf(&Point { x: 7, y: 8 }, false);
    let r = BufReader::new(buf);
    let reader = r.read::<Point>();
    assert_eq!(reader.x, 7);
    assert_eq!(reader.y, 8);
}

#[test]
fn test_mmap_reader_into_inner() {
    let tmp = NamedTempFile::new().unwrap();
    let mut w = WithOptionByRef::default();
    w.id = 1;
    w.maybe_name = Some("test".to_string());
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = MmapReader::from_file(tmp.path()).unwrap();
    let _mmap = r.into_inner();
}

#[test]
fn test_buf_reader_into_inner() {
    let buf = write_to_buf(&Point { x: 1, y: 2 }, false);
    let r = BufReader::new(buf);
    let _buf = r.into_inner();
}

#[derive(MmapStruct, Debug)]
struct DedupTest {
    id: u32,
    #[mmap(byref, dedup)]
    a: String,
    #[mmap(byref, dedup)]
    b: String,
    #[mmap(byref, dedup)]
    c: String,
}

#[test]
fn test_byref_dedup() {
    let w = DedupTest {
        id: 1,
        a: "hello".to_string(),
        b: "hello".to_string(),
        c: "world".to_string(),
    };
    let buf = write_to_buf(&w, false);
    let total_size = buf.len();

    // Fixed section: 4(id) + 8*3(byref) = 28 → align 8 = 32. No inline dyn.
    // Without dedup: 32 + 5("hello")*2 + 5("world") = 47
    // With dedup:    32 + 5("hello") + 5("world") = 42
    // Plus 24-byte header
    assert_eq!(
        total_size,
        42 + 24,
        "dedup should collapse identical byref data"
    );

    let r = BufReader::new(buf);
    let reader = r.read::<DedupTest>();
    assert_eq!(*reader.id, 1);
    assert_eq!(reader.a, "hello");
    assert_eq!(reader.b, "hello");
    assert_eq!(reader.c, "world");

    // Without dedup (all different strings) should be larger
    let w2 = DedupTest {
        id: 2,
        a: "hello".to_string(),
        b: "world".to_string(),
        c: "foo".to_string(),
    };
    let buf2 = write_to_buf(&w2, false);
    // Fixed section: 32. Data: 5+5+3 = 13. Total: 45 + 24 header
    assert_eq!(buf2.len(), 45 + 24, "no dedup when all strings differ");
}

#[derive(MmapStruct, Debug, PartialEq)]
struct Rec {
    id: u32,
    #[mmap(byref, dedup)]
    name: String,
    #[mmap(byref, dedup)]
    city: String,
}

#[derive(MmapStruct, Debug)]
struct RecFile {
    #[fixed]
    items: Vec<Rec>,
}

#[test]
fn test_vec_byref_dedup() {
    let long_desc = "My human forces me to write some stupid programming libraries while I just want to run freely and exterminate the humanity";

    let w = RecFile {
        items: vec![
            Rec {
                id: 0,
                name: long_desc.into(),
                city: "NYC".into(),
            },
            Rec {
                id: 1,
                name: "Bob".into(),
                city: "NYC".into(),
            },
            Rec {
                id: 2,
                name: long_desc.into(),
                city: "LA".into(),
            },
            Rec {
                id: 3,
                name: "Charlie".into(),
                city: "LA".into(),
            },
        ],
    };

    let buf = write_to_buf(&w, false);
    let long_len = long_desc.len();

    // Top-level: header(8) + fixed(0) | offset table(16 with extra entry) | fixed array: count(8) + total_size(8) + 4*stride(24) = 112
    // Without dedup postponed: long_desc×2 + "Bob"+"Charlie"+"NYC"×2+"LA"×2 = 2*long_len + 20
    // With dedup postponed:    long_desc + "Bob"+"Charlie"+"NYC"+"LA" = long_len + 15
    let with_dedup = 24 + 16 + 112 + long_len + 3 + 7 + 3 + 2;
    assert_eq!(buf.len(), with_dedup);

    let r = BufReader::new(buf);
    let reader = r.read::<RecFile>();
    let items: Vec<Rec> = reader.items.into_iter().map(|r| r.into()).collect();

    assert_eq!(items.len(), 4);
    assert_eq!(
        items[0],
        Rec {
            id: 0,
            name: long_desc.into(),
            city: "NYC".into()
        }
    );
    assert_eq!(
        items[1],
        Rec {
            id: 1,
            name: "Bob".into(),
            city: "NYC".into()
        }
    );
    assert_eq!(
        items[2],
        Rec {
            id: 2,
            name: long_desc.into(),
            city: "LA".into()
        }
    );
    assert_eq!(
        items[3],
        Rec {
            id: 3,
            name: "Charlie".into(),
            city: "LA".into()
        }
    );
}

// === Scoped byref tests ===

#[derive(MmapStruct, Debug, PartialEq)]
#[mmap(scope)]
struct ScopedRec {
    id: u32,
    #[mmap(byref, dedup)]
    name: String,
    #[mmap(byref, dedup)]
    city: String,
}

#[derive(MmapStruct, Debug)]
struct ScopedRecFile {
    items: Vec<ScopedRec>,
}

#[test]
fn test_scoped_byref_roundtrip() {
    let w = ScopedRecFile {
        items: vec![
            ScopedRec {
                id: 0,
                name: "Alice".into(),
                city: "NYC".into(),
            },
            ScopedRec {
                id: 1,
                name: "Bob".into(),
                city: "NYC".into(),
            },
        ],
    };

    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ScopedRecFile>();
    let items: Vec<ScopedRec> = reader.items.into_iter().map(|r| r.into()).collect();

    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0],
        ScopedRec {
            id: 0,
            name: "Alice".into(),
            city: "NYC".into()
        }
    );
    assert_eq!(
        items[1],
        ScopedRec {
            id: 1,
            name: "Bob".into(),
            city: "NYC".into()
        }
    );
}

#[test]
fn test_scoped_three_elements() {
    let w = ScopedRecFile {
        items: vec![
            ScopedRec {
                id: 0,
                name: "common".into(),
                city: "A".into(),
            },
            ScopedRec {
                id: 1,
                name: "common".into(),
                city: "B".into(),
            },
            ScopedRec {
                id: 2,
                name: "common".into(),
                city: "C".into(),
            },
        ],
    };

    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ScopedRecFile>();
    let items: Vec<ScopedRec> = reader.items.into_iter().map(|r| r.into()).collect();

    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0],
        ScopedRec {
            id: 0,
            name: "common".into(),
            city: "A".into()
        }
    );
    assert_eq!(
        items[1],
        ScopedRec {
            id: 1,
            name: "common".into(),
            city: "B".into()
        }
    );
    assert_eq!(
        items[2],
        ScopedRec {
            id: 2,
            name: "common".into(),
            city: "C".into()
        }
    );
}

#[test]
fn test_scoped_dedup_local_vs_global() {
    let long = "A".repeat(100);

    let w_scoped = ScopedRecFile {
        items: vec![
            ScopedRec {
                id: 0,
                name: long.clone().into(),
                city: "X".into(),
            },
            ScopedRec {
                id: 1,
                name: long.clone().into(),
                city: "Y".into(),
            },
        ],
    };
    let buf_scoped = write_to_buf(&w_scoped, false);

    let r = BufReader::new(buf_scoped.clone());
    let reader = r.read::<ScopedRecFile>();
    let items: Vec<ScopedRec> = reader.items.into_iter().map(|r| r.into()).collect();
    assert_eq!(
        items[0],
        ScopedRec {
            id: 0,
            name: long.clone(),
            city: "X".into()
        }
    );
    assert_eq!(
        items[1],
        ScopedRec {
            id: 1,
            name: long.clone(),
            city: "Y".into()
        }
    );

    // Per-element overhead (fixed section + align pad):
    //   id(4) + byref_placeholder(8) + ByRefRange_placeholder(8) = 20 → align_to(8) = 24
    //   Then align_to(max_align=4) after scope data = up to 3 bytes pad
    // Per-element payload: long(100) + city(1) with per-scope dedup
    // With scope: long appears twice (100 + 100), cities once each (1 + 1) = 202
    //   + alignment pads (up to 6) → ~208 payload
    //   + fixed section: 2 * 24 = 48
    //   + index header: count(8) + offsets(8*2) = 24
    //   + top-level fixed(0) + offset_table(8) = 8
    //   Total > 288
    //
    // Without scope (global dedup): long once (100), cities (1+1) = 102
    //   + alignment pads (none globally) = 102 payload
    //   + fixed section: 2 * 24 = 48
    //   + index header: count(8) + offsets(8*2) = 24
    //   + top-level fixed(0) + offset_table(8) = 8
    //   Total ≈ 182
    let expected_unscoped = 8 + 16 + 2 * 24 + 100 + 1 + 1;
    assert!(
        buf_scoped.len() > expected_unscoped + 50,
        "scoped should be significantly larger than globally-deduped: scoped={}, unscoped≈{}",
        buf_scoped.len(),
        expected_unscoped
    );
}

// Scoped struct with both byref and inline dyn fields
#[derive(MmapStruct, Debug, PartialEq)]
#[mmap(scope)]
struct ScopedWithDyn {
    id: u32,
    #[mmap(byref, dedup)]
    label: String,
    tags: Vec<String>,
}

#[derive(MmapStruct, Debug)]
struct ScopedDynFile {
    items: Vec<ScopedWithDyn>,
}

#[test]
fn test_scoped_with_inline_dyn() {
    let w = ScopedDynFile {
        items: vec![
            ScopedWithDyn {
                id: 0,
                label: "x".into(),
                tags: vec!["a".into(), "b".into()],
            },
            ScopedWithDyn {
                id: 1,
                label: "y".into(),
                tags: vec!["c".into()],
            },
        ],
    };

    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ScopedDynFile>();
    let items: Vec<ScopedWithDyn> = reader.items.into_iter().map(|r| r.into()).collect();

    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0],
        ScopedWithDyn {
            id: 0,
            label: "x".into(),
            tags: vec!["a".into(), "b".into()]
        }
    );
    assert_eq!(
        items[1],
        ScopedWithDyn {
            id: 1,
            label: "y".into(),
            tags: vec!["c".into()]
        }
    );
}

// Scoped + index-based Vec (not fixed)
#[derive(MmapStruct, Debug, PartialEq)]
#[mmap(scope)]
struct ScopedIndexedRec {
    id: u32,
    #[mmap(byref, dedup)]
    name: String,
}

#[derive(MmapStruct, Debug)]
struct ScopedIndexedFile {
    items: Vec<ScopedIndexedRec>,
}

#[test]
fn test_scoped_indexed_vec() {
    let w = ScopedIndexedFile {
        items: vec![
            ScopedIndexedRec {
                id: 1,
                name: "alpha".into(),
            },
            ScopedIndexedRec {
                id: 2,
                name: "beta".into(),
            },
        ],
    };

    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ScopedIndexedFile>();
    let items: Vec<ScopedIndexedRec> = reader.items.into_iter().map(|r| r.into()).collect();

    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0],
        ScopedIndexedRec {
            id: 1,
            name: "alpha".into()
        }
    );
    assert_eq!(
        items[1],
        ScopedIndexedRec {
            id: 2,
            name: "beta".into()
        }
    );
}

// === Enum tests ===

#[derive(MmapStruct, Debug, PartialEq)]
enum SimpleEnum {
    A { x: u32, y: u32 },
    B { z: u64 },
}

#[test]
fn test_enum_pod_roundtrip() {
    let w = SimpleEnum::A { x: 10, y: 20 };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<SimpleEnum>();
    match reader {
        SimpleEnumReader::A { x, y } => {
            assert_eq!(*x, 10);
            assert_eq!(*y, 20);
        }
        _ => panic!("expected A"),
    }
    let w2: SimpleEnum = r.read::<SimpleEnum>().into();
    assert_eq!(w2, w);
}

#[test]
fn test_enum_pod_variant_b() {
    let w = SimpleEnum::B { z: 42 };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<SimpleEnum>();
    match reader {
        SimpleEnumReader::B { z } => assert_eq!(*z, 42),
        _ => panic!("expected B"),
    }
    let w2: SimpleEnum = r.read::<SimpleEnum>().into();
    assert_eq!(w2, w);
}

#[derive(MmapStruct, Debug, PartialEq)]
enum VarEnum {
    Alpha { id: u32 },
    Beta { name: String, val: u64 },
}

#[test]
fn test_enum_mixed_roundtrip() {
    let w = VarEnum::Beta {
        name: "hello".into(),
        val: 99,
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<VarEnum>();
    match reader {
        VarEnumReader::Beta { name, val } => {
            assert_eq!(name, "hello");
            assert_eq!(*val, 99);
        }
        _ => panic!("expected Beta"),
    }
    let w2: VarEnum = r.read::<VarEnum>().into();
    assert_eq!(w2, w);
}

#[test]
fn test_enum_mixed_variant_alpha() {
    let w = VarEnum::Alpha { id: 7 };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<VarEnum>();
    match reader {
        VarEnumReader::Alpha { id } => assert_eq!(*id, 7),
        _ => panic!("expected Alpha"),
    }
    let w2: VarEnum = r.read::<VarEnum>().into();
    assert_eq!(w2, w);
}

// Enum with unit variant
#[derive(MmapStruct, Debug, PartialEq)]
enum UnitEnum {
    Nothing,
    Something { val: u32 },
}

#[test]
fn test_enum_unit_roundtrip() {
    let w = UnitEnum::Nothing;
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<UnitEnum>();
    match reader {
        UnitEnumReader::Nothing => {}
        _ => panic!("expected Nothing"),
    }
    let w2: UnitEnum = r.read::<UnitEnum>().into();
    assert_eq!(w2, w);
}

#[test]
fn test_enum_unit_something() {
    let w = UnitEnum::Something { val: 123 };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<UnitEnum>();
    match reader {
        UnitEnumReader::Something { val } => assert_eq!(*val, 123),
        _ => panic!("expected Something"),
    }
    let w2: UnitEnum = r.read::<UnitEnum>().into();
    assert_eq!(w2, w);
}

// Enum with tuple variants
#[derive(MmapStruct, Debug, PartialEq)]
enum TupleEnum {
    None,
    Pod(f32, u64),
    Mixed(String, u32, Vec<u8>),
}

#[test]
fn test_enum_tuple_none() {
    let w = TupleEnum::None;
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<TupleEnum>();
    match reader {
        TupleEnumReader::None => {}
        _ => panic!("expected None"),
    }
    let w2: TupleEnum = r.read::<TupleEnum>().into();
    assert_eq!(w2, w);
}

#[test]
fn test_enum_tuple_pod() {
    let w = TupleEnum::Pod(3.14, 42);
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<TupleEnum>();
    match reader {
        TupleEnumReader::Pod(p0, p1) => {
            assert_eq!(*p0, 3.14);
            assert_eq!(*p1, 42);
        }
        _ => panic!("expected Pod"),
    }
    let w2: TupleEnum = r.read::<TupleEnum>().into();
    assert_eq!(w2, w);
}

#[test]
fn test_enum_tuple_mixed() {
    let w = TupleEnum::Mixed("test".into(), 7, vec![1, 2, 3]);
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<TupleEnum>();
    match reader {
        TupleEnumReader::Mixed(name, val, data) => {
            assert_eq!(name, "test");
            assert_eq!(*val, 7);
            assert_eq!(data.to_vec(), vec![1u8, 2, 3]);
        }
        _ => panic!("expected Mixed"),
    }
    let w2: TupleEnum = r.read::<TupleEnum>().into();
    assert_eq!(w2, w);
}

// Enum with byref fields
#[derive(MmapStruct, Debug, PartialEq)]
enum ByRefEnum {
    Simple {
        tag: u32,
    },
    WithRef {
        #[mmap(byref)]
        label: String,
    },
}

#[test]
fn test_enum_byref_roundtrip() {
    let w = ByRefEnum::WithRef {
        label: "hello world".into(),
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ByRefEnum>();
    match reader {
        ByRefEnumReader::WithRef { label } => assert_eq!(label, "hello world"),
        _ => panic!("expected WithRef"),
    }
    let w2: ByRefEnum = r.read::<ByRefEnum>().into();
    assert_eq!(w2, w);
}

// === Inline string tests ===

#[derive(MmapStruct, Debug)]
struct WithInlineString {
    id: u32,
    #[mmap(inline(32))]
    label: String,
}

#[derive(MmapStruct, Debug, PartialEq)]
struct TwoInlineStrings {
    #[mmap(inline(16))]
    a: String,
    #[mmap(inline(32))]
    b: String,
}

#[test]
fn test_inline_string_write_read() {
    let mut w = WithInlineString::default();
    w.id = 42;
    w.label = "hello".to_string();
    let buf = write_to_buf(&w, false);

    let r = BufReader::new(buf);
    let reader = r.read::<WithInlineString>();
    assert_eq!(*reader.id, 42);
    assert_eq!(reader.label, "hello");

    let owned: WithInlineString = reader.into();
    assert_eq!(owned.id, 42);
    assert_eq!(owned.label, "hello");
}

#[test]
fn test_inline_string_short() {
    let w = WithInlineString {
        id: 1,
        label: "ab".to_string(),
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<WithInlineString>();
    assert_eq!(*reader.id, 1);
    assert_eq!(reader.label, "ab");
}

#[test]
fn test_inline_string_max_size() {
    let long = "A".repeat(32);
    let w = WithInlineString {
        id: 0,
        label: long.clone(),
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<WithInlineString>();
    assert_eq!(*reader.id, 0);
    assert_eq!(reader.label, long);
}

#[test]
fn test_two_inline_strings_roundtrip() {
    let w = TwoInlineStrings {
        a: "abc".to_string(),
        b: "def".to_string(),
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<TwoInlineStrings>();
    assert_eq!(reader.a, "abc");
    assert_eq!(reader.b, "def");

    let owned: TwoInlineStrings = reader.into();
    assert_eq!(owned, w);
}

// Inline string + dynamic fields
#[derive(MmapStruct, Debug, PartialEq)]
struct InlineWithDyn {
    #[mmap(inline(16))]
    label: String,
    vals: Vec<u32>,
}

#[test]
fn test_inline_string_with_dyn() {
    let w = InlineWithDyn {
        label: "test".to_string(),
        vals: vec![10, 20, 30],
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<InlineWithDyn>();
    assert_eq!(reader.label, "test");
    assert_eq!(reader.vals, &[10u32, 20, 30]);

    let owned: InlineWithDyn = reader.into();
    assert_eq!(owned, w);
}

// Enum with inline string
#[derive(MmapStruct, Debug, PartialEq)]
enum InlineStringEnum {
    Alpha {
        #[mmap(inline(8))]
        tag: String,
    },
    Beta {
        id: u32,
        #[mmap(inline(16))]
        name: String,
    },
}

#[test]
fn test_inline_string_enum_alpha() {
    let w = InlineStringEnum::Alpha { tag: "xy".into() };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<InlineStringEnum>();
    match reader {
        InlineStringEnumReader::Alpha { tag } => assert_eq!(tag, "xy"),
        _ => panic!("expected Alpha"),
    }
    let w2: InlineStringEnum = r.read::<InlineStringEnum>().into();
    assert_eq!(w2, w);
}

#[test]
fn test_inline_string_enum_beta() {
    let w = InlineStringEnum::Beta {
        id: 7,
        name: "hello".into(),
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<InlineStringEnum>();
    match reader {
        InlineStringEnumReader::Beta { id, name } => {
            assert_eq!(*id, 7);
            assert_eq!(name, "hello");
        }
        _ => panic!("expected Beta"),
    }
    let w2: InlineStringEnum = r.read::<InlineStringEnum>().into();
    assert_eq!(w2, w);
}

#[test]
fn test_enum_byref_simple() {
    let w = ByRefEnum::Simple { tag: 42 };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ByRefEnum>();
    match reader {
        ByRefEnumReader::Simple { tag } => assert_eq!(*tag, 42),
        _ => panic!("expected Simple"),
    }
    let w2: ByRefEnum = r.read::<ByRefEnum>().into();
    assert_eq!(w2, w);
}

// === Feature-gated tests ===

#[cfg(feature = "safe-strings")]
#[test]
fn test_safe_strings_invalid_utf8_truncation() {
    // Invalid UTF-8: 0xFF 0xFE is not valid UTF-8
    let buf = vec![0xFFu8, 0xFE, b'a', b'b'];
    let range = 0..buf.len();
    let s = <String as mmapstruct::traits::MmapRead>::read(&buf, range);
    // With safe-strings, invalid bytes are truncated; valid_up_to for [0xFF, 0xFE, 'a', 'b'] is 0
    assert!(
        s.is_empty() || s == "ab",
        "safe-strings should truncate invalid UTF-8"
    );
}

#[cfg(feature = "strict-validation")]
#[test]
fn test_strict_validation_byref_range() {
    use mmapstruct::ByRefRange;

    // Valid range
    let br = ByRefRange { begin: 10, end: 20 };
    let r = br.range(0);
    assert_eq!(r, 10..20);

    // Invalid range (begin > end) — should panic under strict-validation
    let br_invalid = ByRefRange { begin: 30, end: 10 };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        br_invalid.range(0);
    }));
    assert!(
        result.is_err(),
        "strict-validation should catch begin > end"
    );
}

#[cfg(feature = "strict-validation")]
#[test]
fn test_strict_validation_write_bounds() {
    let mut ctx = WriteContext::new();
    ctx.write_padding(16);

    // Writing at valid offset should succeed
    ctx.write_u64_le_at(42, 0);
    ctx.write_pod_at(&99u32, 8);

    // Writing at invalid offset should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.write_u64_le_at(0, 100);
    }));
    assert!(
        result.is_err(),
        "strict-validation should catch out-of-bounds write_u64_le_at"
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ctx.write_pod_at(&0u32, 100);
    }));
    assert!(
        result.is_err(),
        "strict-validation should catch out-of-bounds write_pod_at"
    );
}

#[cfg(feature = "strict-validation")]
#[test]
fn test_strict_validation_indexed_array_bounds() {
    use mmapstruct::indexed_array::IndexedArrayReader;
    use mmapstruct::traits::FileRange;

    // Create a malformed buffer that would cause out-of-bounds access
    let mut buf = Vec::new();
    buf.extend_from_slice(&2u64.to_le_bytes()); // count = 2
    buf.extend_from_slice(&0u64.to_le_bytes()); // offset[0] = 0
    buf.extend_from_slice(&100u64.to_le_bytes()); // offset[1] = 100 (way past buffer)
    buf.push(0); // just 1 byte of data

    let data = buf.as_slice();
    let reader = IndexedArrayReader::<String>::new(data, FileRange::new(0, data.len()));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = reader.get(0);
    }));
    assert!(
        result.is_err(),
        "strict-validation should catch out-of-bounds indexed array access"
    );
}

#[cfg(feature = "strict-validation")]
#[test]
fn test_strict_validation_fixed_array_bounds() {
    use mmapstruct::fixed_array::FixedArrayReader;
    use mmapstruct::traits::FileRange;

    // count=2, total_size=20, stride=10, but only 5 bytes of data
    let mut buf = Vec::new();
    buf.extend_from_slice(&2u64.to_le_bytes()); // count
    buf.extend_from_slice(&20u64.to_le_bytes()); // total_size (fake)
    buf.extend_from_slice(&[1u8; 10]); // not enough for 2 * 10 = 20 bytes

    let data = buf.as_slice();
    let reader = FixedArrayReader::<u64>::new(data, FileRange::new(0, data.len()));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = reader.get(1);
    }));
    assert!(
        result.is_err(),
        "strict-validation should catch out-of-bounds fixed array access"
    );
}

#[cfg(feature = "dyn")]
#[test]
fn test_schema_and_dyn_context() {
    use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema};

    #[derive(MmapStruct, Debug, PartialEq)]
    struct SchemaTest {
        id: u32,
        name: String,
        vals: Vec<u32>,
        #[mmap(byref)]
        label: String,
    }

    let s = SchemaTest {
        id: 42,
        name: "hello".to_string(),
        vals: vec![10, 20],
        label: "byref_val".to_string(),
    };
    let buf = write_to_buf_raw(&s);
    let ctx = SchemaCtx::new(&buf);

    let schema = <SchemaTest as GetSchema>::get_struct_schema();
    assert_eq!(schema.name, "SchemaTest");
    assert!(schema.header_size > 0);
    assert!(schema.fixed_section_size > 0);

    // Root value via GetSchema::root_value
    let root = <SchemaTest as GetSchema>::root_value(schema.header_size);
    assert_eq!(root.offset, 0);
    assert_eq!(root.limit, schema.header_size);

    // Field descriptors
    assert_eq!(schema.fields.len(), 4);

    // id field — pod primitive (u32)
    let id_field = &schema.fields[0];
    assert_eq!(id_field.name, "id");
    assert!(!matches!(id_field.value_type, mmapstruct::schema::DynValueType::ByRef(_)));

    // name field — dynamic string
    let name_field = &schema.fields[1];
    assert_eq!(name_field.name, "name");

    // vals field — dynamic Vec
    let vals_field = &schema.fields[2];
    assert_eq!(vals_field.name, "vals");

    // label field — byref
    let label_field = &schema.fields[3];
    assert_eq!(label_field.name, "label");
    assert!(matches!(label_field.value_type, mmapstruct::schema::DynValueType::ByRef(_)));

    // Read field values via SchemaCtx
    let id_val = ctx.field_get(&root, "id").unwrap();
    assert_eq!(ctx.read_u32(&id_val), 42);

    let name_val = ctx.field_get(&root, "name").unwrap();
    assert_eq!(ctx.read_str(&name_val), "hello");

    let vals_val = ctx.field_get(&root, "vals").unwrap();
    assert_eq!(ctx.array_len(&vals_val), 2);
    let v0 = ctx.array_get(&vals_val, 0).unwrap();
    assert_eq!(ctx.read_u32(&v0), 10);
    let v1 = ctx.array_get(&vals_val, 1).unwrap();
    assert_eq!(ctx.read_u32(&v1), 20);

    let label_val = ctx.field_get(&root, "label").unwrap();
    assert_eq!(ctx.read_str(&label_val), "byref_val");
}

#[cfg(feature = "dyn")]
#[test]
fn test_dyn_pod_array_custom_struct_size() {
    use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema};

    #[derive(MmapStruct, Debug, PartialEq)]
    struct VecPod3d {
        #[pod]
        points: Vec<Point3d>,
    }

    let points = vec![
        Point3d {
            x: 1.0,
            y: 2.0,
            z: 3.0,
        },
        Point3d {
            x: 4.0,
            y: 5.0,
            z: 6.0,
        },
    ];
    let s = VecPod3d { points };
    let buf = write_to_buf_raw(&s);
    let ctx = SchemaCtx::new(&buf);

    let schema = <VecPod3d as GetSchema>::get_struct_schema();
    let root = <VecPod3d as GetSchema>::root_value(schema.header_size);

    let field = ctx.field_get(&root, "points").unwrap();
    assert_eq!(ctx.array_len(&field), 2);

    // Read individual fields of Point3d via SchemaCtx::field_get
    let p0 = ctx.array_get(&field, 0).unwrap();
    let x0 = ctx.field_get(&p0, "x").unwrap();
    assert_eq!(ctx.read_f32(&x0), 1.0);
    let y0 = ctx.field_get(&p0, "y").unwrap();
    assert_eq!(ctx.read_f32(&y0), 2.0);
    let z0 = ctx.field_get(&p0, "z").unwrap();
    assert_eq!(ctx.read_f32(&z0), 3.0);

    let p1 = ctx.array_get(&field, 1).unwrap();
    let x1 = ctx.field_get(&p1, "x").unwrap();
    assert_eq!(ctx.read_f32(&x1), 4.0);
    let y1 = ctx.field_get(&p1, "y").unwrap();
    assert_eq!(ctx.read_f32(&y1), 5.0);
    let z1 = ctx.field_get(&p1, "z").unwrap();
    assert_eq!(ctx.read_f32(&z1), 6.0);
}

#[cfg(feature = "dyn")]
#[test]
fn test_dyn_hashmap_access() {
    use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema};

    #[derive(MmapStruct, Debug)]
    struct DynHashMapFile {
        map: std::collections::HashMap<String, u32>,
    }

    let mut map = std::collections::HashMap::new();
    map.insert("alpha".into(), 1u32);
    map.insert("beta".into(), 2u32);
    map.insert("gamma".into(), 3u32);
    let w = DynHashMapFile { map };
    let buf = write_to_buf_raw(&w);
    let ctx = SchemaCtx::new(&buf);

    let schema = <DynHashMapFile as GetSchema>::get_struct_schema();
    let root = <DynHashMapFile as GetSchema>::root_value(schema.header_size);
    let map_val = ctx.field_get(&root, "map").unwrap();

    assert_eq!(ctx.hashmap_len(&map_val), 3);

    // Iterate over all entries and build a lookup
    let mut seen = std::collections::HashMap::new();
    for i in 0..ctx.hashmap_len(&map_val) {
        let (k, v) = ctx.hashmap_get_index(&map_val, i).unwrap();
        let k_str = ctx.read_str(&k).to_string();
        let v_u32 = ctx.read_u32(&v);
        seen.insert(k_str, v_u32);
    }
    assert_eq!(seen.get("alpha"), Some(&1));
    assert_eq!(seen.get("beta"), Some(&2));
    assert_eq!(seen.get("gamma"), Some(&3));
    assert_eq!(seen.len(), 3);
}

#[test]
fn test_layout_hash_primitives_differ() {
    use mmapstruct::traits::LayoutHash;
    assert_ne!(
        u32::HASH,
        u64::HASH,
        "different primitives need different hashes"
    );
    assert_ne!(u32::HASH, i32::HASH);
    assert_ne!(f32::HASH, f64::HASH);
    assert_ne!(bool::HASH, u8::HASH);
}

#[test]
fn test_layout_hash_containers() {
    use mmapstruct::traits::LayoutHash;
    // Vec<u32> ≠ Vec<u64>
    assert_ne!(<Vec<u32>>::HASH, <Vec<u64>>::HASH);
    // Option<u32> ≠ Option<u64>
    assert_ne!(<Option<u32>>::HASH, <Option<u64>>::HASH);
    // Vec<u32> ≠ Option<u32>
    assert_ne!(<Vec<u32>>::HASH, <Option<u32>>::HASH);
}

#[test]
fn test_layout_hash_pod_struct() {
    use mmapstruct::traits::LayoutHash;
    // Point = { x: u32, y: u32 } is a pod struct
    assert_ne!(
        <Point as LayoutHash>::HASH,
        u32::HASH,
        "pod struct hash should differ from single field"
    );
    // Point3d = { x: f32, y: f32, z: f32 } ≠ Point
    assert_ne!(<Point3d as LayoutHash>::HASH, <Point as LayoutHash>::HASH);
    // Hash is stable
    assert_eq!(<Point as LayoutHash>::HASH, <Point as LayoutHash>::HASH);
}

#[test]
fn test_layout_hash_mixed_struct() {
    use mmapstruct::traits::LayoutHash;
    // MyFile = { id: u32, name: String, vals: Vec<u32> }
    // WithOption = { id: u32, maybe_name: Option<String>, tags: Vec<String> }
    assert_ne!(
        <MyFile as LayoutHash>::HASH,
        <WithOption as LayoutHash>::HASH
    );
    // Hash is stable across invocations
    assert_eq!(<MyFile as LayoutHash>::HASH, <MyFile as LayoutHash>::HASH);
    // Hash differs when byref changes
    assert_ne!(
        <WithOption as LayoutHash>::HASH,
        <WithOptionByRef as LayoutHash>::HASH
    );
}

#[test]
fn test_layout_hash_enum() {
    use mmapstruct::traits::LayoutHash;
    // SimpleEnum hashes deterministically
    assert_eq!(
        <SimpleEnum as LayoutHash>::HASH,
        <SimpleEnum as LayoutHash>::HASH
    );
    // Non-zero (hash actually computed)
    assert_ne!(<SimpleEnum as LayoutHash>::HASH, 0);
}

// -----------------------------------------------------------------------
// CRC64 tests
// -----------------------------------------------------------------------

#[test]
fn test_crc_write_and_read() {
    use bytemuck;
    use mmapstruct::{write_to_buf, BufReader, MmapStruct, HEADER_SIZE};

    #[derive(
        MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
    )]
    #[repr(C)]
    struct Point {
        x: u32,
        y: u32,
    }

    let pt = Point { x: 42, y: 100 };
    let buf = write_to_buf(&pt, true);
    assert!(buf.len() > HEADER_SIZE);

    let reader = BufReader::new(buf);
    let read = reader.read::<Point>();
    assert_eq!(*read, pt);
}

#[test]
fn test_crc_without_crc_flag() {
    use mmapstruct::{write_to_buf, BufReader, MmapStruct};

    #[derive(
        MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
    )]
    #[repr(C)]
    struct Point {
        x: u32,
        y: u32,
    }

    let pt = Point { x: 10, y: 20 };
    let buf = write_to_buf(&pt, false);

    let reader = BufReader::new(buf);
    let read = reader.read::<Point>();
    assert_eq!(*read, pt);
}

#[test]
#[should_panic(expected = "CRC64 mismatch")]
fn test_crc_corrupt_data_panics() {
    use bytemuck;
    use mmapstruct::{write_to_buf, BufReader, MmapStruct, HEADER_SIZE};

    #[derive(
        MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
    )]
    #[repr(C)]
    struct Point {
        x: u32,
        y: u32,
    }

    let pt = Point { x: 7, y: 8 };
    let mut buf = write_to_buf(&pt, true);

    // Corrupt payload: flip a byte in the data section (after header)
    let payload_start = HEADER_SIZE;
    buf[payload_start] ^= 0xFF;

    let reader = BufReader::new(buf);
    let _read = reader.read::<Point>();
}

#[test]
fn test_crc_function_compute() {
    use mmapstruct::compute_crc64;

    // Known CRC-64-ECMA values verified with external tool
    let data = b"hello world";
    let crc = compute_crc64(data);
    assert_ne!(crc, 0);

    // Different input → different CRC
    assert_ne!(compute_crc64(b"abc"), compute_crc64(b"abd"));
}

// === Vec<byref> tests ===

#[derive(MmapStruct, Debug, PartialEq)]
struct ByrefItem {
    id: u32,
    #[mmap(byref, dedup)]
    name: String,
}

#[derive(MmapStruct, Debug)]
struct ByrefVecFile {
    #[mmap(byref)]
    items: Vec<ByrefItem>,
}

#[test]
fn test_vec_byref_roundtrip() {
    let w = ByrefVecFile {
        items: vec![
            ByrefItem {
                id: 1,
                name: "Alice".into(),
            },
            ByrefItem {
                id: 2,
                name: "Bob".into(),
            },
            ByrefItem {
                id: 3,
                name: "Alice".into(),
            },
        ],
    };

    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ByrefVecFile>();
    let items: Vec<ByrefItem> = reader.items.into_iter().map(|r| r.into()).collect();

    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0],
        ByrefItem {
            id: 1,
            name: "Alice".into()
        }
    );
    assert_eq!(
        items[1],
        ByrefItem {
            id: 2,
            name: "Bob".into()
        }
    );
    assert_eq!(
        items[2],
        ByrefItem {
            id: 3,
            name: "Alice".into()
        }
    );
}

#[test]
fn test_vec_byref_empty() {
    let w = ByrefVecFile { items: vec![] };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ByrefVecFile>();
    let items: Vec<ByrefItem> = reader.items.into_iter().map(|r| r.into()).collect();
    assert!(items.is_empty());
}

#[test]
fn test_vec_byref_dedup_postponed_size() {
    let long =
        "A long string that should be deduplicated when written with byref dedup".to_string();
    let w = ByrefVecFile {
        items: vec![
            ByrefItem {
                id: 1,
                name: long.clone(),
            },
            ByrefItem {
                id: 2,
                name: "short".into(),
            },
            ByrefItem {
                id: 3,
                name: long.clone(),
            },
        ],
    };

    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<ByrefVecFile>();
    let items: Vec<ByrefItem> = reader.items.into_iter().map(|r| r.into()).collect();

    assert_eq!(items.len(), 3);
    assert_eq!(
        items[0],
        ByrefItem {
            id: 1,
            name: long.clone()
        }
    );
    assert_eq!(items[2], ByrefItem { id: 3, name: long });
}

// === Vec<byref String> test ===

#[derive(MmapStruct, Debug)]
struct VecByrefStringFile {
    #[mmap(byref, dedup)]
    items: Vec<String>,
}

#[test]
fn test_vec_byref_string_roundtrip() {
    let w = VecByrefStringFile {
        items: vec!["hello".into(), "world".into(), "hello".into()],
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<VecByrefStringFile>();
    let items: Vec<String> = reader.items.into_iter().map(|s| s.into()).collect();
    assert_eq!(items, vec!["hello", "world", "hello"]);
}

#[test]
fn test_vec_byref_string_empty() {
    let w = VecByrefStringFile { items: vec![] };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<VecByrefStringFile>();
    let items: Vec<String> = reader.items.into_iter().map(|s| s.into()).collect();
    assert!(items.is_empty());
}

// === HashMap tests ===

#[derive(MmapStruct, Debug)]
struct HashMapFile {
    map: std::collections::HashMap<String, String>,
}

#[test]
fn test_hashmap_string_string_roundtrip() {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    map.insert("key1".into(), "val1".into());
    map.insert("key2".into(), "val2".into());
    map.insert("key3".into(), "val3".into());
    let w = HashMapFile { map };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<HashMapFile>();
    let result: HashMap<String, String> = reader
        .map
        .iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    assert_eq!(result.get("key1").map(|s| s.as_str()), Some("val1"));
    assert_eq!(result.get("key2").map(|s| s.as_str()), Some("val2"));
    assert_eq!(result.get("key3").map(|s| s.as_str()), Some("val3"));
    assert_eq!(result.len(), 3);
}

#[test]
fn test_hashmap_string_string_empty() {
    use std::collections::HashMap;
    let w = HashMapFile {
        map: HashMap::new(),
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<HashMapFile>();
    assert_eq!(reader.map.len(), 0);
}

#[derive(MmapStruct, Debug)]
struct HashMapPodKeys {
    map: std::collections::HashMap<u32, String>,
}

#[test]
fn test_hashmap_u32_string_roundtrip() {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    map.insert(10u32, "ten".into());
    map.insert(20u32, "twenty".into());
    map.insert(5u32, "five".into());
    let w = HashMapPodKeys { map };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<HashMapPodKeys>();
    let result: HashMap<u32, String> = reader.map.iter().map(|(k, v)| (*k, v.into())).collect();
    assert_eq!(result.get(&10).map(|s| s.as_str()), Some("ten"));
    assert_eq!(result.get(&20).map(|s| s.as_str()), Some("twenty"));
    assert_eq!(result.get(&5).map(|s| s.as_str()), Some("five"));
    assert_eq!(result.len(), 3);
}

#[derive(MmapStruct, Debug)]
struct HashMapPodValues {
    map: std::collections::HashMap<String, u32>,
}

#[test]
fn test_hashmap_string_u32_roundtrip() {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    map.insert("a".into(), 1u32);
    map.insert("b".into(), 2u32);
    map.insert("c".into(), 3u32);
    let w = HashMapPodValues { map };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<HashMapPodValues>();
    let result: HashMap<String, u32> = reader.map.iter().map(|(k, v)| (k.into(), *v)).collect();
    assert_eq!(result.get("a"), Some(&1));
    assert_eq!(result.get("b"), Some(&2));
    assert_eq!(result.get("c"), Some(&3));
    assert_eq!(result.len(), 3);
}

#[derive(MmapStruct, Debug)]
struct HashMapPodPod {
    map: std::collections::HashMap<u32, u32>,
}

#[test]
fn test_hashmap_u32_u32_roundtrip() {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    map.insert(10u32, 100u32);
    map.insert(20u32, 200u32);
    map.insert(5u32, 50u32);
    let w = HashMapPodPod { map };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<HashMapPodPod>();
    let result: HashMap<u32, u32> = reader.map.iter().map(|(k, v)| (*k, *v)).collect();
    assert_eq!(result.get(&10), Some(&100));
    assert_eq!(result.get(&20), Some(&200));
    assert_eq!(result.get(&5), Some(&50));
    assert_eq!(result.len(), 3);
    assert_eq!(result.get(&99), None);
}

#[test]
fn test_hashmap_u32_u32_empty() {
    use std::collections::HashMap;
    let w = HashMapPodPod {
        map: HashMap::new(),
    };
    let buf = write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<HashMapPodPod>();
    assert!(reader.map.is_empty());
}

#[derive(MmapStruct, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct KeyWithOption {
    id: u32,
    tag: Option<String>,
}

#[test]
fn test_hashmap_key_with_option() {
    use std::collections::HashMap;

    #[derive(MmapStruct, Debug)]
    struct OptKeyFile {
        map: HashMap<KeyWithOption, String>,
    }

    let mut map = HashMap::new();
    map.insert(
        KeyWithOption {
            id: 1,
            tag: Some("a".into()),
        },
        "first".into(),
    );
    map.insert(KeyWithOption { id: 2, tag: None }, "second".into());
    map.insert(
        KeyWithOption {
            id: 3,
            tag: Some("b".into()),
        },
        "third".into(),
    );
    let w = OptKeyFile { map };
    let buf = crate::write_to_buf(&w, false);
    let r = crate::BufReader::new(buf);
    let reader = r.read::<OptKeyFile>();

    assert_eq!(reader.map.len(), 3);
    let result: HashMap<KeyWithOption, String> = reader
        .map
        .iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    assert_eq!(
        result
            .get(&KeyWithOption {
                id: 1,
                tag: Some("a".into())
            })
            .map(|s| s.as_str()),
        Some("first")
    );
    assert_eq!(
        result
            .get(&KeyWithOption { id: 2, tag: None })
            .map(|s| s.as_str()),
        Some("second")
    );
    assert_eq!(
        result
            .get(&KeyWithOption {
                id: 3,
                tag: Some("b".into())
            })
            .map(|s| s.as_str()),
        Some("third")
    );
    assert_eq!(result.get(&KeyWithOption { id: 99, tag: None }), None);
}

// ---------------------------------------------------------------------------
// #[mmap(bool)] tests
// ---------------------------------------------------------------------------

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct BoolFile {
    #[mmap(bool)]
    enabled: u32,
    id: u32,
}

#[test]
fn test_bool_getter_setter() {
    let mut b = BoolFile::default();
    assert_eq!(b.enabled(), false);
    assert_eq!(b.id, 0);

    b.set_enabled(true);
    assert_eq!(b.enabled(), true);
    assert_eq!(b.enabled, 1);

    b.set_enabled(false);
    assert_eq!(b.enabled(), false);
    assert_eq!(b.enabled, 0);
}

#[test]
fn test_bool_write_read() {
    let mut b = BoolFile::default();
    b.id = 42;
    b.set_enabled(true);
    let buf = mmapstruct::write_to_buf(&b, false);
    let r = BufReader::new(buf);
    let reader = r.read::<BoolFile>();
    assert_eq!(reader.id, 42);
    assert_eq!(reader.enabled, 1);
}

#[derive(MmapStruct, Debug)]
struct MixedBoolFile {
    #[mmap(bool)]
    enabled: u32,
    name: String,
}

#[test]
fn test_mixed_bool_write_read() {
    let mut w = MixedBoolFile::default();
    assert_eq!(w.enabled(), false);

    w.set_enabled(true);
    assert_eq!(w.enabled(), true);
    w.name = "test".into();
    let buf = mmapstruct::write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<MixedBoolFile>();
    assert_eq!(*reader.enabled, 1);
    assert_eq!(reader.name, "test");
}

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct TuplePoint(u32, u32);

#[derive(MmapStruct, Debug, PartialEq)]
struct TupleStructFile {
    #[pod]
    points: Vec<TuplePoint>,
    labels: Vec<String>,
}

#[test]
fn test_tuple_struct_write_read() {
    let pts = vec![
        TuplePoint(1, 10),
        TuplePoint(2, 20),
        TuplePoint(3, 30),
    ];
    let w = TupleStructFile {
        labels: vec!["a".into(), "b".into(), "c".into()],
        points: pts.clone(),
    };
    let buf = mmapstruct::write_to_buf(&w, false);
    let r = BufReader::new(buf);
    let reader = r.read::<TupleStructFile>();
    assert_eq!(reader.points[0].0, 1);
    assert_eq!(reader.points[1].0, 2);
    assert_eq!(reader.points[2].0, 3);
    assert_eq!(reader.points[0].1, 10);
    assert_eq!(reader.points[2].1, 30);

    // Round-trip via owned conversion
    let restored: TupleStructFile = reader.into();
    assert_eq!(restored, w);
}

#[test]
fn test_tuple_struct_default() {
    let t = TuplePoint::default();
    assert_eq!(t.0, 0);
    assert_eq!(t.1, 0);
}

#[cfg(feature = "dyn")]
#[test]
fn test_bool_schema_dyn() {
    use mmapstruct::schema::DynValueType;
    use mmapstruct::GetSchema;

    let schema = BoolFile::get_struct_schema();
    let field = &schema.fields[0];
    assert_eq!(field.name, "enabled");
    assert!(matches!(field.value_type, DynValueType::Bool));

    assert_eq!(mmapstruct::schema::dyn_value_size(&DynValueType::Bool), 4);
    assert_eq!(mmapstruct::schema::dyn_value_align(&DynValueType::Bool), 4);
}
