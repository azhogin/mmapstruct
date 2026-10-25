//! SchemaPatchSession tests: name-addressed patching with no compile-time
//! root type at the patching site. Files are written with the typed API
//! (to produce real files) and patched purely through the runtime schema.

use bytemuck::{Pod, Zeroable};
use mmapstruct::file_reader::BufReader;
use mmapstruct::write_context::Reserve;
use mmapstruct::ReservePlan;
use mmapstruct::{List, MmapStruct};
use mmapstruct_schema::builder::{
    write_to_file_with_embedded_schema, write_to_file_with_embedded_schema_with,
};
use mmapstruct_schema::patch::SchemaPatchSession;
use tempfile::NamedTempFile;

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct Note {
    id: u64,
    score: f64,
}

#[derive(MmapStruct, Debug)]
enum LayerLog {
    Decode {
        model: String,
        entries: List<String>,
    },
    Prefill {
        model: String,
        entries: List<String>,
        #[pod]
        batch: Vec<Note>,
    },
}

#[derive(MmapStruct, Debug)]
struct DemoFile {
    id: u32,
    tags: List<String>,
    #[mmap(byref)]
    notes: Vec<String>,
    #[mmap(byref)]
    title: String,
    vals: Vec<u32>,
}

fn utf8(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

#[test]
fn test_schema_patch_struct_file() {
    let tmp = NamedTempFile::new().unwrap();
    let demo = DemoFile {
        id: 42,
        tags: List::from_vec(vec!["t0".into()]),
        notes: vec!["n0".to_string()],
        title: "the title".to_string(),
        vals: vec![1, 2],
    };
    // Reserves: the byref fields need slot/gap slack, and `vals` needs a
    // data gap because byref postponed data trails every inline dyn value
    // (an unreserved pod vec can never tail-append in a file with byref
    // fields).
    let mut plan = ReservePlan::new();
    plan.set(
        "notes",
        Reserve {
            slots: 2,
            bytes: 64,
        },
    );
    plan.set(
        "title",
        Reserve {
            slots: 0,
            bytes: 64,
        },
    );
    plan.set(
        "vals",
        Reserve {
            slots: 0,
            bytes: 32,
        },
    );
    std::fs::write(
        tmp.path(),
        write_to_file_with_embedded_schema_with(&demo, false, plan),
    )
    .unwrap();
    let size_before = std::fs::metadata(tmp.path()).unwrap().len();

    // Schema-only patching: no DemoFile types used from here on.
    let mut sp = SchemaPatchSession::open(tmp.path()).unwrap();
    assert_eq!(sp.root_layout().inline_dyn_count, 3); // tags, notes-vec, vals (title is fixed-byref)

    // The tail container appends BEFORE any mid-file list append (the
    // documented ordering caveat): a mid-file list node lands at the
    // payload end and ends the previous tail's grow-path.
    sp.append_pod_vec("vals", &[9u32]).unwrap();
    // List append by path (nodes land at the payload end; the file grows).
    sp.append_list_raw("tags", &utf8("t1")).unwrap();
    // Indexed (byref vec) append in place.
    sp.append_byref_vec_raw("notes", &utf8("n1")).unwrap();
    // Standalone byref set (copy-on-write into the marked gap).
    sp.set_byref_raw("title", &utf8("a longer title")).unwrap();
    sp.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<DemoFile>();
    assert_eq!(*reader.id, 42);
    assert_eq!(reader.tags.len(), 2);
    assert_eq!(reader.tags.get(1).unwrap(), "t1");
    let notes: Vec<String> = reader.notes.iter().map(|s| s.to_string()).collect();
    assert_eq!(notes, vec!["n0", "n1"]);
    assert_eq!(reader.title, "a longer title");
    assert_eq!(reader.vals.len(), 3);
    assert_eq!(reader.vals[2], 9);

    // In-place byref ops must not have grown the file; the list append did.
    let size_after = std::fs::metadata(tmp.path()).unwrap().len();
    assert!(size_after > size_before, "list append grows the file");
}

#[test]
fn test_schema_patch_enum_root() {
    let tmp = NamedTempFile::new().unwrap();
    let log = LayerLog::Prefill {
        model: "m2".to_string(),
        entries: List::from_vec(vec!["p1".into()]),
        batch: vec![Note { id: 1, score: 1.5 }],
    };
    std::fs::write(tmp.path(), write_to_file_with_embedded_schema(&log, false)).unwrap();

    let mut sp = SchemaPatchSession::open(tmp.path()).unwrap();
    // The active variant is Prefill (disc 1): its layout facts are used.
    assert_eq!(sp.root_layout().disc_prefix, 8);
    // The tail container appends BEFORE the mid-file list (documented
    // ordering caveat: a mid-file list node lands at the payload end and
    // ends the previous tail's grow-path).
    sp.append_pod_vec("batch", &[Note { id: 2, score: 2.5 }])
        .unwrap();
    sp.append_list_raw("entries", &utf8("p2")).unwrap();
    sp.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<LayerLog>();
    match reader {
        LayerLogReader::Prefill { entries, batch, .. } => {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries.get(1).unwrap(), "p2");
            assert_eq!(batch.len(), 2);
            assert_eq!(batch[1], Note { id: 2, score: 2.5 });
        }
        _ => panic!("expected Prefill"),
    }
}

#[test]
fn test_schema_patch_errors() {
    let tmp = NamedTempFile::new().unwrap();
    let demo = DemoFile {
        id: 1,
        tags: List::from_vec(vec![]),
        notes: vec![],
        title: "t".to_string(),
        vals: vec![1],
    };
    std::fs::write(tmp.path(), write_to_file_with_embedded_schema(&demo, false)).unwrap();

    let mut sp = SchemaPatchSession::open(tmp.path()).unwrap();

    // Wrong op for the layout: tags is a list, not an indexed array.
    let err = sp.append_indexed_raw("tags", &utf8("x")).unwrap_err();
    assert!(
        err.to_string().contains("is a list array, not indexed"),
        "unexpected error: {err}"
    );

    // Missing field.
    let err = sp.append_list_raw("nope", &utf8("x")).unwrap_err();
    assert!(
        err.to_string().contains("no such field"),
        "unexpected error: {err}"
    );

    // Unknown root path segment.
    let err = sp.append_list_raw("a.b.c", &utf8("x")).unwrap_err();
    assert!(
        err.to_string().contains("no such field \"a\""),
        "unexpected error: {err}"
    );
}

#[test]
fn test_schema_patch_open_with_schema_and_sidecar() {
    // Split file: headerless payload + .mmshdr sidecar carrying the schema
    // (embed_schema = true), resolved through `open`.
    use mmapstruct_schema::write_to_file_split;
    let tmp = NamedTempFile::new().unwrap();
    let demo = DemoFile {
        id: 7,
        tags: List::from_vec(vec!["s0".into()]),
        notes: vec![],
        title: "split".to_string(),
        vals: vec![3],
    };
    write_to_file_split(tmp.path(), &demo, false, true).unwrap();

    let mut sp = SchemaPatchSession::open(tmp.path()).unwrap();
    sp.append_list_raw("tags", &utf8("s1")).unwrap();
    sp.flush().unwrap();

    let r = mmapstruct::file_reader::MmapReader::from_file_with_sidecar(tmp.path()).unwrap();
    let reader = r.read::<DemoFile>();
    assert_eq!(reader.tags.len(), 2);
    assert_eq!(reader.tags.get(1).unwrap(), "s1");

    // Explicit schema bytes: strip the sidecar header and pass them in.
    let sidecar = std::fs::read({
        let mut p = tmp.path().to_path_buf();
        p.set_extension("mmshdr");
        p
    })
    .unwrap();
    let (_, schema_range, _, _) = mmapstruct::file_reader::parse_header(&sidecar);
    let (s, e) = schema_range.unwrap();
    let mut sp2 = SchemaPatchSession::open_with_schema(tmp.path(), &sidecar[s..e]).unwrap();
    sp2.append_list_raw("tags", &utf8("s2")).unwrap();
    sp2.flush().unwrap();

    let r = mmapstruct::file_reader::MmapReader::from_file_with_sidecar(tmp.path()).unwrap();
    let reader = r.read::<DemoFile>();
    assert_eq!(reader.tags.len(), 3);
    assert_eq!(reader.tags.get(2).unwrap(), "s2");
}
