//! Reserve tests: self-describing reserved blocks + in-place appends.

use bytemuck::{Pod, Zeroable};
use mmapstruct::file_reader::BufReader;
use mmapstruct::write_context::Reserve;
use mmapstruct::{write_to_file, write_to_file_with, MmapStruct, PatchSession, ReservePlan};
use mmapstruct_schema::write_to_file_split_with;
use tempfile::NamedTempFile;

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct Note {
    id: u64,
    score: f64,
}

#[derive(MmapStruct, Debug)]
struct ReserveFile {
    id: u32,
    tags: Vec<String>,
    #[pod]
    points: Vec<Note>,
    vals: Vec<u32>,
}

fn plan() -> ReservePlan {
    let mut p = ReservePlan::new();
    p.set(
        "tags",
        Reserve {
            slots: 4,
            bytes: 256,
        },
    );
    p.set(
        "points",
        Reserve {
            slots: 0,
            bytes: 128,
        },
    );
    p
}

fn sample() -> ReserveFile {
    ReserveFile {
        id: 77,
        tags: vec!["t0".to_string()],
        points: vec![Note { id: 1, score: 1.0 }, Note { id: 2, score: 2.0 }],
        vals: vec![9, 8, 7],
    }
}

#[test]
fn test_reserved_blocks_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&sample(), tmp.path(), false, plan()).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ReserveFile>();
    assert_eq!(*reader.id, 77);
    assert_eq!(reader.tags.len(), 1);
    assert_eq!(reader.tags.get(0).unwrap(), "t0");
    assert_eq!(reader.points.len(), 2);
    assert_eq!(reader.points[1], Note { id: 2, score: 2.0 });
    assert_eq!(reader.vals.len(), 3);
    assert_eq!(reader.vals[2], 7);
}

#[test]
fn test_append_pod_vec_inplace_midfile() {
    // `points` is a reserved pod vec in the MIDDLE of the file (tags before,
    // vals after): appending must be a pure in-place write.
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&sample(), tmp.path(), false, plan()).unwrap();
    let size_before = std::fs::metadata(tmp.path()).unwrap().len();

    let mut session = PatchSession::open_typed::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let elems_off = session.offset_of(&reader.points[0]).unwrap();

    session
        .append_pod_vec::<ReserveFile, Note>(elems_off, &[Note { id: 5, score: 5.0 }])
        .unwrap();
    session.flush().unwrap();

    assert_eq!(
        std::fs::metadata(tmp.path()).unwrap().len(),
        size_before,
        "in-place append must not grow the file"
    );

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ReserveFile>();
    assert_eq!(*reader.id, 77);
    assert_eq!(reader.points.len(), 3);
    assert_eq!(reader.points[2], Note { id: 5, score: 5.0 });
    // siblings untouched
    assert_eq!(reader.tags.len(), 1);
    assert_eq!(reader.vals.len(), 3);
}

#[test]
fn test_append_pod_vec_reserve_exhausted() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&sample(), tmp.path(), false, plan()).unwrap();

    let mut session = PatchSession::open_typed::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let elems_off = session.offset_of(&reader.points[0]).unwrap();

    // 128-byte gap fits exactly 8 Notes (16 bytes each).
    let eight: Vec<Note> = (0..8).map(|i| Note { id: i, score: 0.5 }).collect();
    session
        .append_pod_vec::<ReserveFile, Note>(elems_off, &eight)
        .unwrap();
    let err = session
        .append_pod_vec::<ReserveFile, Note>(elems_off, &[Note::default()])
        .unwrap_err();
    assert!(
        err.to_string().contains("data reserve exhausted"),
        "unexpected error: {err}"
    );
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    assert_eq!(r.read::<ReserveFile>().points.len(), 10);
}

#[test]
fn test_append_indexed_inplace_midfile() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&sample(), tmp.path(), false, plan()).unwrap();
    let size_before = std::fs::metadata(tmp.path()).unwrap().len();

    let mut session = PatchSession::open_typed::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let block_off = reader.tags.block_range().start;

    session
        .append_indexed::<ReserveFile, String>(block_off, &"t1".to_string())
        .unwrap();
    session
        .append_indexed::<ReserveFile, String>(block_off, &"t2".to_string())
        .unwrap();
    session.flush().unwrap();

    assert_eq!(
        std::fs::metadata(tmp.path()).unwrap().len(),
        size_before,
        "in-place append must not grow the file"
    );

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ReserveFile>();
    assert_eq!(reader.tags.len(), 3);
    assert_eq!(reader.tags.get(0).unwrap(), "t0");
    assert_eq!(reader.tags.get(1).unwrap(), "t1");
    assert_eq!(reader.tags.get(2).unwrap(), "t2");
    // siblings untouched
    assert_eq!(reader.points.len(), 2);
    assert_eq!(reader.vals.len(), 3);
}

#[test]
fn test_append_indexed_exhausted_slots() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&sample(), tmp.path(), false, plan()).unwrap();

    let mut session = PatchSession::open_typed::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let block_off = reader.tags.block_range().start;

    // 4 spare slots: four appends succeed, the fifth fails.
    for i in 0..4 {
        session
            .append_indexed::<ReserveFile, String>(block_off, &format!("x{i}"))
            .unwrap();
    }
    let err = session
        .append_indexed::<ReserveFile, String>(block_off, &"overflow".to_string())
        .unwrap_err();
    assert!(
        err.to_string().contains("no spare table slots"),
        "unexpected error: {err}"
    );
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ReserveFile>();
    assert_eq!(reader.tags.len(), 5);
    assert_eq!(reader.tags.get(4).unwrap(), "x3");
}

#[test]
fn test_append_indexed_data_gap_exhausted() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&sample(), tmp.path(), false, plan()).unwrap();

    let mut session = PatchSession::open_typed::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let block_off = reader.tags.block_range().start;

    let long = "y".repeat(300); // gap is 256 bytes
    let err = session
        .append_indexed::<ReserveFile, String>(block_off, &long)
        .unwrap_err();
    assert!(
        err.to_string().contains("data reserve exhausted"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_append_indexed_unflagged_rejected() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file(&sample(), tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let block_off = reader.tags.block_range().start;

    let err = session
        .append_indexed::<ReserveFile, String>(block_off, &"z".to_string())
        .unwrap_err();
    assert!(
        err.to_string().contains("no reserve"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_unreserved_tail_append_still_works() {
    // `vals` has no reserve and sits at the payload tail: the stage-2 grow
    // path must keep working.
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&sample(), tmp.path(), false, plan()).unwrap();
    let size_before = std::fs::metadata(tmp.path()).unwrap().len();

    let mut session = PatchSession::open_typed::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let vals_off = session.offset_of(&reader.vals[0]).unwrap();

    session
        .append_pod_vec::<ReserveFile, u32>(vals_off, &[6, 5])
        .unwrap();
    session.flush().unwrap();

    assert!(std::fs::metadata(tmp.path()).unwrap().len() > size_before);
    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ReserveFile>();
    assert_eq!(reader.vals.len(), 5);
    assert_eq!(reader.vals[3], 6);
    assert_eq!(reader.vals[4], 5);
}

#[test]
fn test_reserve_enum_variant_blocks() {
    // Enum variants route through the same dyn writer: a reserved pod vec in
    // a variant reads back flag-aware.
    #[derive(MmapStruct, Debug)]
    enum NotesEvent {
        Idle,
        Batch {
            id: u32,
            #[pod]
            notes: Vec<Note>,
        },
    }

    let tmp = NamedTempFile::new().unwrap();
    let mut p = ReservePlan::new();
    p.set(
        "notes",
        Reserve {
            slots: 0,
            bytes: 64,
        },
    );
    let ev = NotesEvent::Batch {
        id: 3,
        notes: vec![Note { id: 1, score: 1.0 }],
    };
    write_to_file_with(&ev, tmp.path(), false, p).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<NotesEvent>();
    match reader {
        NotesEventReader::Batch { id, notes } => {
            assert_eq!(*id, 3);
            assert_eq!(notes.len(), 1);
            assert_eq!(notes[0], Note { id: 1, score: 1.0 });
        }
        _ => panic!("expected Batch"),
    }
}

#[test]
fn test_reserve_split_file_with_sidecar() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_split_with(tmp.path(), &sample(), false, false, plan()).unwrap();

    let mut session = PatchSession::open_typed_with_sidecar::<ReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ReserveFile>().unwrap();
    let block_off = reader.tags.block_range().start;
    let elems_off = session.offset_of(&reader.points[0]).unwrap();

    session
        .append_indexed::<ReserveFile, String>(block_off, &"split".to_string())
        .unwrap();
    session
        .append_pod_vec::<ReserveFile, Note>(elems_off, &[Note { id: 42, score: 4.2 }])
        .unwrap();
    session.flush().unwrap();

    let r = mmapstruct::file_reader::MmapReader::from_file_with_sidecar(tmp.path()).unwrap();
    let reader = r.read::<ReserveFile>();
    assert_eq!(reader.tags.len(), 2);
    assert_eq!(reader.tags.get(1).unwrap(), "split");
    assert_eq!(reader.points.len(), 3);
    assert_eq!(reader.points[2], Note { id: 42, score: 4.2 });
}

// ---------------------------------------------------------------------------
// byref-vec reserve: spare ByRefRange slots + postponed data gap
// ---------------------------------------------------------------------------

#[derive(MmapStruct, Debug)]
struct ByRefReserveFile {
    id: u32,
    #[mmap(byref)]
    notes: Vec<String>,
    vals: Vec<u32>,
}

fn byref_plan() -> ReservePlan {
    let mut p = ReservePlan::new();
    p.set(
        "notes",
        Reserve {
            slots: 3,
            bytes: 128,
        },
    );
    p
}

fn byref_sample() -> ByRefReserveFile {
    ByRefReserveFile {
        id: 5,
        notes: vec!["n0".to_string(), "n1".to_string()],
        vals: vec![1, 2],
    }
}

#[test]
fn test_byref_reserve_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&byref_sample(), tmp.path(), false, byref_plan()).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ByRefReserveFile>();
    assert_eq!(*reader.id, 5);
    let notes: Vec<String> = reader.notes.iter().map(|s| s.to_string()).collect();
    assert_eq!(notes, vec!["n0", "n1"]);
    assert_eq!(reader.vals.len(), 2);
}

#[test]
fn test_append_byref_vec_inplace_midfile() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&byref_sample(), tmp.path(), false, byref_plan()).unwrap();
    let size_before = std::fs::metadata(tmp.path()).unwrap().len();

    let mut session = PatchSession::open_typed::<ByRefReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ByRefReserveFile>().unwrap();
    let block_off = reader.notes.block_range().start;

    session
        .append_byref_vec::<ByRefReserveFile, String>(block_off, &"n2".to_string())
        .unwrap();
    session
        .append_byref_vec::<ByRefReserveFile, String>(block_off, &"n3".to_string())
        .unwrap();
    session.flush().unwrap();

    assert_eq!(
        std::fs::metadata(tmp.path()).unwrap().len(),
        size_before,
        "in-place byref append must not grow the file"
    );

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ByRefReserveFile>();
    assert_eq!(*reader.id, 5);
    let notes: Vec<String> = reader.notes.iter().map(|s| s.to_string()).collect();
    assert_eq!(notes, vec!["n0", "n1", "n2", "n3"]);
    assert_eq!(reader.vals.len(), 2);
}

#[test]
fn test_append_byref_vec_exhausted_slots() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&byref_sample(), tmp.path(), false, byref_plan()).unwrap();

    let mut session = PatchSession::open_typed::<ByRefReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ByRefReserveFile>().unwrap();
    let block_off = reader.notes.block_range().start;

    for i in 0..3 {
        session
            .append_byref_vec::<ByRefReserveFile, String>(block_off, &format!("x{i}"))
            .unwrap();
    }
    let err = session
        .append_byref_vec::<ByRefReserveFile, String>(block_off, &"overflow".to_string())
        .unwrap_err();
    assert!(
        err.to_string().contains("no spare table slots"),
        "unexpected error: {err}"
    );
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<ByRefReserveFile>();
    assert_eq!(reader.notes.len(), 5);
    let notes: Vec<String> = reader.notes.iter().map(|s| s.to_string()).collect();
    assert_eq!(notes, vec!["n0", "n1", "x0", "x1", "x2"]);
}

#[test]
fn test_append_byref_vec_data_gap_exhausted() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(&byref_sample(), tmp.path(), false, byref_plan()).unwrap();

    let mut session = PatchSession::open_typed::<ByRefReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ByRefReserveFile>().unwrap();
    let block_off = reader.notes.block_range().start;

    let long = "y".repeat(200); // gap is 128 bytes
    let err = session
        .append_byref_vec::<ByRefReserveFile, String>(block_off, &long)
        .unwrap_err();
    assert!(
        err.to_string().contains("data reserve exhausted"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_append_byref_vec_unflagged_rejected() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file(&byref_sample(), tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<ByRefReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ByRefReserveFile>().unwrap();
    let block_off = reader.notes.block_range().start;

    let err = session
        .append_byref_vec::<ByRefReserveFile, String>(block_off, &"z".to_string())
        .unwrap_err();
    assert!(
        err.to_string().contains("no reserve"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_append_byref_vec_empty_rejected() {
    let tmp = NamedTempFile::new().unwrap();
    write_to_file_with(
        &ByRefReserveFile {
            id: 6,
            notes: vec![],
            vals: vec![9],
        },
        tmp.path(),
        false,
        byref_plan(),
    )
    .unwrap();

    let mut session = PatchSession::open_typed::<ByRefReserveFile>(tmp.path()).unwrap();
    let reader = session.read::<ByRefReserveFile>().unwrap();
    let block_off = reader.notes.block_range().start;

    let err = session
        .append_byref_vec::<ByRefReserveFile, String>(block_off, &"first".to_string())
        .unwrap_err();
    assert!(
        err.to_string().contains("no data anchor"),
        "unexpected error: {err}"
    );
}

#[test]
#[should_panic(expected = "byref-vec reserve requires a root-level non-dedup field")]
fn test_byref_reserve_dedup_rejected_at_write() {
    #[derive(MmapStruct, Debug)]
    struct DedupFile {
        id: u32,
        #[mmap(byref, dedup)]
        notes: Vec<String>,
    }
    let mut p = ReservePlan::new();
    p.set(
        "notes",
        Reserve {
            slots: 2,
            bytes: 64,
        },
    );
    let _ = write_to_file_with(
        &DedupFile {
            id: 1,
            notes: vec!["a".to_string(), "a".to_string()],
        },
        "/tmp/nonexistent-byref-dedup.mms",
        false,
        p,
    );
}

#[test]
#[should_panic(expected = "byref-vec reserve requires a root-level non-dedup field")]
fn test_byref_reserve_nested_rejected_at_write() {
    #[derive(MmapStruct, Debug)]
    struct Inner {
        #[mmap(byref)]
        tags: Vec<String>,
    }
    #[derive(MmapStruct, Debug)]
    struct Outer {
        id: u32,
        inner: Inner,
    }
    let mut p = ReservePlan::new();
    p.set(
        "tags",
        Reserve {
            slots: 2,
            bytes: 64,
        },
    );
    let _ = write_to_file_with(
        &Outer {
            id: 1,
            inner: Inner {
                tags: vec!["t".to_string()],
            },
        },
        "/tmp/nonexistent-byref-nested.mms",
        false,
        p,
    );
}
