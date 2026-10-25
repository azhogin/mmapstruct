//! List<T> tests: linked-node container with in-place append.

use bytemuck::{Pod, Zeroable};
use mmapstruct::file_reader::BufReader;
use mmapstruct::{write_to_file, List, MmapStruct, PatchSession};
use tempfile::NamedTempFile;

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct Note {
    id: u64,
    score: f64,
}

#[derive(MmapStruct, Debug)]
struct TagFile {
    id: u32,
    tags: List<String>,
}

#[derive(MmapStruct, Debug)]
struct NotesFile {
    id: u32,
    notes: List<Note>,
}

#[derive(MmapStruct, Debug)]
struct MixedTailFile {
    id: u32,
    #[mmap(byref)]
    labels: Vec<String>,
    tags: List<String>,
}

#[derive(MmapStruct, Debug)]
struct TwoDynFile {
    tags: List<String>,
    vals: Vec<u32>,
}

#[derive(MmapStruct, Debug)]
enum TagEvent {
    Empty,
    Batch { id: u32, tags: List<String> },
}

#[test]
fn test_list_string_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    let f = TagFile {
        id: 5,
        tags: List::from_vec(vec!["alpha".into(), "beta".into(), "gamma".into()]),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<TagFile>();
    assert_eq!(*reader.id, 5);
    assert_eq!(reader.tags.len(), 3);
    let by_index: Vec<String> = (0..reader.tags.len())
        .map(|i| reader.tags.get(i).unwrap().to_string())
        .collect();
    assert_eq!(by_index, vec!["alpha", "beta", "gamma"]);
    let via_iter: Vec<String> = reader.tags.iter().map(|s| s.to_string()).collect();
    assert_eq!(via_iter, vec!["alpha", "beta", "gamma"]);
    let owned: List<String> = reader.tags.clone().into();
    assert_eq!(owned.as_slice(), ["alpha", "beta", "gamma"]);
}

#[test]
fn test_list_struct_elems_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    let f = NotesFile {
        id: 1,
        notes: List::from_vec(vec![
            Note { id: 10, score: 1.5 },
            Note {
                id: 20,
                score: -2.5,
            },
        ]),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<NotesFile>();
    assert_eq!(reader.notes.len(), 2);
    assert_eq!(*reader.notes.get(0).unwrap(), Note { id: 10, score: 1.5 });
    assert_eq!(
        *reader.notes.get(1).unwrap(),
        Note {
            id: 20,
            score: -2.5
        }
    );
    assert_eq!(reader.notes.get(2), None);
    let owned: Vec<Note> = reader.notes.clone().iter().copied().collect();
    assert_eq!(owned.len(), 2);
}

#[test]
fn test_list_empty_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    let f = TagFile {
        id: 7,
        tags: List::new(),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<TagFile>();
    assert_eq!(*reader.id, 7);
    assert!(reader.tags.is_empty());
    assert_eq!(reader.tags.iter().count(), 0);
    assert_eq!(reader.tags.get(0), None);
}

#[test]
fn test_list_with_byref_sibling() {
    // Nodes share the tail region with byref data; explicit chains make the
    // interleaving safe.
    let tmp = NamedTempFile::new().unwrap();
    let f = MixedTailFile {
        id: 3,
        labels: vec!["l1".to_string(), "l2".to_string()],
        tags: List::from_vec(vec!["t1".into(), "t2".into(), "t3".into()]),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<MixedTailFile>();
    assert_eq!(*reader.id, 3);
    let labels: Vec<String> = reader.labels.iter().map(|s| s.to_string()).collect();
    assert_eq!(labels, vec!["l1", "l2"]);
    let tags: Vec<String> = reader.tags.iter().map(|s| s.to_string()).collect();
    assert_eq!(tags, vec!["t1", "t2", "t3"]);
}

#[test]
fn test_append_list_basic() {
    let tmp = NamedTempFile::new().unwrap();
    let f = TagFile {
        id: 11,
        tags: List::from_vec(vec!["a".into(), "b".into()]),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<TagFile>(tmp.path()).unwrap();
    let reader = session.read::<TagFile>().unwrap();
    assert_eq!(reader.tags.len(), 2);
    let block_off = reader.tags.block_range().start;

    session
        .append_list::<TagFile, String>(block_off, &"c".to_string())
        .unwrap();
    session
        .append_list::<TagFile, String>(block_off, &"d".to_string())
        .unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<TagFile>();
    assert_eq!(*reader.id, 11);
    assert_eq!(reader.tags.len(), 4);
    let tags: Vec<String> = reader.tags.iter().map(|s| s.to_string()).collect();
    assert_eq!(tags, vec!["a", "b", "c", "d"]);
}

#[test]
fn test_append_list_from_empty() {
    let tmp = NamedTempFile::new().unwrap();
    let f = TagFile {
        id: 12,
        tags: List::new(),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<TagFile>(tmp.path()).unwrap();
    let reader = session.read::<TagFile>().unwrap();
    let block_off = reader.tags.block_range().start;

    session
        .append_list::<TagFile, String>(block_off, &"first".to_string())
        .unwrap();
    session
        .append_list::<TagFile, String>(block_off, &"second".to_string())
        .unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<TagFile>();
    assert_eq!(reader.tags.len(), 2);
    let tags: Vec<String> = reader.tags.iter().map(|s| s.to_string()).collect();
    assert_eq!(tags, vec!["first", "second"]);
}

#[test]
fn test_append_list_struct_elem() {
    let tmp = NamedTempFile::new().unwrap();
    let f = NotesFile {
        id: 2,
        notes: List::from_vec(vec![Note { id: 1, score: 1.0 }]),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<NotesFile>(tmp.path()).unwrap();
    let reader = session.read::<NotesFile>().unwrap();
    let block_off = reader.notes.block_range().start;

    let add = Note {
        id: 99,
        score: -7.25,
    };
    session
        .append_list::<NotesFile, Note>(block_off, &add)
        .unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<NotesFile>();
    assert_eq!(reader.notes.len(), 2);
    assert_eq!(*reader.notes.get(0).unwrap(), Note { id: 1, score: 1.0 });
    assert_eq!(*reader.notes.get(1).unwrap(), add);
}

#[test]
fn test_append_list_rejects_non_tail() {
    // vals follows the list, so the list chain does not end at payload end.
    let tmp = NamedTempFile::new().unwrap();
    let f = TwoDynFile {
        tags: List::from_vec(vec!["t".into()]),
        vals: vec![1, 2, 3],
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<TwoDynFile>(tmp.path()).unwrap();
    let reader = session.read::<TwoDynFile>().unwrap();
    let block_off = reader.tags.block_range().start;

    let err = session
        .append_list::<TwoDynFile, String>(block_off, &"x".to_string())
        .unwrap_err();
    assert!(
        err.to_string().contains("not at the payload tail"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_append_list_rejects_corrupt_tail_pointer() {
    let tmp = NamedTempFile::new().unwrap();
    let f = TagFile {
        id: 13,
        tags: List::from_vec(vec!["only".into()]),
    };
    write_to_file(&f, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<TagFile>(tmp.path()).unwrap();
    let reader = session.read::<TagFile>().unwrap();
    let block_off = reader.tags.block_range().start;

    // Corrupt the tail pointer (8 bytes after head) to a bogus value.
    session.payload_mut()[block_off + 8..block_off + 16]
        .copy_from_slice(&0xFFFF_0000u64.to_le_bytes());
    let err = session
        .append_list::<TagFile, String>(block_off, &"x".to_string())
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("does not point at a plausible node"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_list_in_enum_variant() {
    let tmp = NamedTempFile::new().unwrap();
    let ev = TagEvent::Batch {
        id: 4,
        tags: List::from_vec(vec!["e1".into(), "e2".into(), "e3".into()]),
    };
    write_to_file(&ev, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<TagEvent>();
    match reader {
        TagEventReader::Batch { id, tags } => {
            assert_eq!(*id, 4);
            let tags: Vec<String> = tags.iter().map(|s| s.to_string()).collect();
            assert_eq!(tags, vec!["e1", "e2", "e3"]);
        }
        _ => panic!("expected Batch variant"),
    }
}

#[cfg(feature = "dyn")]
#[test]
fn test_list_schema_emission() {
    use mmapstruct_schema::build_serialized_schema;
    use mmapstruct_schema::schema::{ArrayLayoutDef, TypeDef};
    // GetSchema compiles for a struct with a List field and the runtime
    // schema carries the List array layout.
    let schema = build_serialized_schema::<TagFile>();
    assert!(schema.types.iter().any(|t| matches!(
        t,
        TypeDef::Array {
            layout: ArrayLayoutDef::List,
            ..
        }
    )));
}
