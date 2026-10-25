//! PatchSession tests: in-place patching of existing mmapstruct files.

use bytemuck::{Pod, Zeroable};
use mmapstruct::file_reader::{BufReader, MmapReader};
use mmapstruct::{write_to_buf_raw, write_to_file, MmapStruct, PatchSession};
use mmapstruct_schema::write_to_file_split;
use std::path::Path;
use tempfile::NamedTempFile;

#[derive(MmapStruct, Debug, PartialEq, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
struct Note {
    id: u64,
    score: f64,
}

#[derive(MmapStruct, Debug)]
struct PatchLog {
    id: u32,
    #[pod]
    points: Vec<Note>,
}

#[derive(MmapStruct, Debug)]
struct TwoDynLog {
    vals: Vec<u32>,
    #[pod]
    points: Vec<Note>,
}

#[derive(MmapStruct, Debug)]
struct StrU32Log {
    name: String,
    vals: Vec<u32>,
}

#[derive(MmapStruct, Debug)]
struct ByRefAheadLog {
    id: u32,
    #[mmap(byref)]
    notes: Vec<String>,
    vals: Vec<u32>,
}

fn write_default_log(path: &Path) {
    let log = PatchLog {
        id: 42,
        points: vec![
            Note { id: 1, score: 1.0 },
            Note { id: 2, score: 2.0 },
            Note { id: 3, score: 3.0 },
        ],
    };
    write_to_file(&log, path, false).unwrap();
}

#[test]
fn test_patch_scalar_field_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    let note = Note { id: 7, score: 2.5 };
    write_to_file(&note, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<Note>(tmp.path()).unwrap();
    let reader = session.read::<Note>().unwrap();
    assert_eq!(reader.id, 7);
    assert_eq!(reader.score, 2.5);
    let off = session.offset_of(&reader.score).unwrap();

    session.patch_pod::<f64>(off, &9.75).unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<Note>();
    assert_eq!(reader.id, 7);
    assert_eq!(reader.score, 9.75);
}

#[test]
fn test_patch_pod_vec_element() {
    let tmp = NamedTempFile::new().unwrap();
    write_default_log(tmp.path());

    let mut session = PatchSession::open_typed::<PatchLog>(tmp.path()).unwrap();
    let reader = session.read::<PatchLog>().unwrap();
    assert_eq!(reader.points.len(), 3);
    let off = session.offset_of(&reader.points[1]).unwrap();

    let replacement = Note {
        id: 99,
        score: -4.5,
    };
    session.patch_pod::<Note>(off, &replacement).unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<PatchLog>();
    assert_eq!(*reader.id, 42);
    assert_eq!(reader.points.len(), 3);
    assert_eq!(reader.points[0], Note { id: 1, score: 1.0 });
    assert_eq!(reader.points[1], replacement);
    assert_eq!(reader.points[2], Note { id: 3, score: 3.0 });
}

#[test]
fn test_patch_session_rejects_crc_file() {
    let tmp = NamedTempFile::new().unwrap();
    let note = Note { id: 1, score: 0.0 };
    write_to_file(&note, tmp.path(), true).unwrap();

    let err = PatchSession::open_typed::<Note>(tmp.path()).unwrap_err();
    assert!(err.to_string().contains("CRC"), "unexpected error: {err}");
}

#[test]
fn test_patch_session_rejects_wrong_type() {
    let tmp = NamedTempFile::new().unwrap();
    let note = Note { id: 1, score: 0.0 };
    write_to_file(&note, tmp.path(), false).unwrap();

    let err = PatchSession::open_typed::<PatchLog>(tmp.path()).unwrap_err();
    assert!(
        err.to_string().contains("layout_hash mismatch"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_patch_session_rejects_raw_file() {
    let tmp = NamedTempFile::new().unwrap();
    let note = Note { id: 1, score: 0.0 };
    let raw = write_to_buf_raw(&note);
    std::fs::write(tmp.path(), &raw).unwrap();

    let err = PatchSession::open_typed::<Note>(tmp.path()).unwrap_err();
    assert!(
        err.to_string().contains("no layout_hash"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_offset_of_outside_payload_rejected() {
    let tmp = NamedTempFile::new().unwrap();
    let note = Note { id: 1, score: 0.0 };
    write_to_file(&note, tmp.path(), false).unwrap();

    let session = PatchSession::open_typed::<Note>(tmp.path()).unwrap();
    let local = Note { id: 5, score: 5.0 };
    let err = session.offset_of(&local).unwrap_err();
    assert!(
        err.to_string().contains("outside the file payload"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_patch_pod_misaligned_offset_rejected() {
    let tmp = NamedTempFile::new().unwrap();
    let note = Note { id: 7, score: 2.5 };
    write_to_file(&note, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<Note>(tmp.path()).unwrap();
    let reader = session.read::<Note>().unwrap();
    let off = session.offset_of(&reader.score).unwrap();

    let err = session.patch_pod::<f64>(off + 1, &9.75).unwrap_err();
    assert!(
        err.to_string().contains("not aligned"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_patch_pod_out_of_bounds_rejected() {
    let tmp = NamedTempFile::new().unwrap();
    let note = Note { id: 7, score: 2.5 };
    write_to_file(&note, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<Note>(tmp.path()).unwrap();
    let payload_len = session.payload().len();
    assert_eq!(payload_len % 8, 0); // Note is 16 bytes; offset is 8-aligned
    let err = session.patch_pod::<f64>(payload_len, &9.75).unwrap_err();
    assert!(
        err.to_string().contains("exceeds payload length"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_patch_split_file_with_sidecar() {
    let tmp = NamedTempFile::new().unwrap();
    let log = PatchLog {
        id: 1,
        points: vec![Note {
            id: 10,
            score: 10.0,
        }],
    };
    write_to_file_split(tmp.path(), &log, false, false).unwrap();

    let mut session = PatchSession::open_typed_with_sidecar::<PatchLog>(tmp.path()).unwrap();
    let reader = session.read::<PatchLog>().unwrap();
    assert_eq!(reader.points[0].id, 10);
    let off = session.offset_of(&reader.points[0]).unwrap();

    session
        .patch_pod::<Note>(
            off,
            &Note {
                id: 11,
                score: 11.0,
            },
        )
        .unwrap();
    session.flush().unwrap();

    let r = MmapReader::from_file_with_sidecar(tmp.path()).unwrap();
    let reader = r.read::<PatchLog>();
    assert_eq!(*reader.id, 1);
    assert_eq!(
        reader.points[0],
        Note {
            id: 11,
            score: 11.0
        }
    );
}

#[test]
fn test_append_pod_vec_tail() {
    let tmp = NamedTempFile::new().unwrap();
    write_default_log(tmp.path());

    let mut session = PatchSession::open_typed::<PatchLog>(tmp.path()).unwrap();
    let reader = session.read::<PatchLog>().unwrap();
    assert_eq!(reader.points.len(), 3);
    let elems_off = session.offset_of(&reader.points[0]).unwrap();

    let add = [Note { id: 4, score: 4.0 }, Note { id: 5, score: 5.5 }];
    session
        .append_pod_vec::<PatchLog, Note>(elems_off, &add)
        .unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<PatchLog>();
    assert_eq!(*reader.id, 42);
    assert_eq!(reader.points.len(), 5);
    assert_eq!(reader.points[3], Note { id: 4, score: 4.0 });
    assert_eq!(reader.points[4], Note { id: 5, score: 5.5 });
    assert_eq!(reader.points[0], Note { id: 1, score: 1.0 });
}

#[test]
fn test_append_pod_vec_from_empty() {
    let tmp = NamedTempFile::new().unwrap();
    let log = PatchLog {
        id: 9,
        points: vec![],
    };
    write_to_file(&log, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<PatchLog>(tmp.path()).unwrap();
    // Empty vec: the count field is the payload tail.
    let elems_off = session.payload().len();
    let add = [Note { id: 1, score: 1.25 }];
    session
        .append_pod_vec::<PatchLog, Note>(elems_off, &add)
        .unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<PatchLog>();
    assert_eq!(*reader.id, 9);
    assert_eq!(reader.points.len(), 1);
    assert_eq!(reader.points[0], Note { id: 1, score: 1.25 });
}

#[test]
fn test_append_pod_vec_noop_on_empty_add() {
    let tmp = NamedTempFile::new().unwrap();
    write_default_log(tmp.path());
    let size_before = std::fs::metadata(tmp.path()).unwrap().len();

    let mut session = PatchSession::open_typed::<PatchLog>(tmp.path()).unwrap();
    let reader = session.read::<PatchLog>().unwrap();
    let elems_off = session.offset_of(&reader.points[0]).unwrap();
    session
        .append_pod_vec::<PatchLog, Note>(elems_off, &[])
        .unwrap();
    session.flush().unwrap();

    assert_eq!(std::fs::metadata(tmp.path()).unwrap().len(), size_before);
    let r = BufReader::from_file(tmp.path()).unwrap();
    assert_eq!(r.read::<PatchLog>().points.len(), 3);
}

#[test]
fn test_append_pod_vec_u32_after_string_block() {
    // The vals block follows a string block and lands 4-mod-8 aligned;
    // the count switch must still update safely (two 4-byte halves).
    let tmp = NamedTempFile::new().unwrap();
    let log = StrU32Log {
        name: "x".to_string(),
        vals: vec![10, 20],
    };
    write_to_file(&log, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<StrU32Log>(tmp.path()).unwrap();
    let reader = session.read::<StrU32Log>().unwrap();
    assert_eq!(reader.vals.len(), 2);
    let elems_off = session.offset_of(&reader.vals[0]).unwrap();

    session
        .append_pod_vec::<StrU32Log, u32>(elems_off, &[30, 40, 50])
        .unwrap();
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<StrU32Log>();
    assert_eq!(reader.name, "x");
    assert_eq!(reader.vals.len(), 5);
    assert_eq!(reader.vals[2], 30);
    assert_eq!(reader.vals[4], 50);
}

#[test]
fn test_append_rejects_non_tail_container() {
    // points is the last dyn value (appendable); vals is not.
    let tmp = NamedTempFile::new().unwrap();
    let log = TwoDynLog {
        vals: vec![1, 2],
        points: vec![Note { id: 1, score: 1.0 }],
    };
    write_to_file(&log, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<TwoDynLog>(tmp.path()).unwrap();
    let reader = session.read::<TwoDynLog>().unwrap();
    // points is the last dyn value (appendable); vals is not.
    let points_off = session.offset_of(&reader.points[0]).unwrap();
    let vals_off = session.offset_of(&reader.vals[0]).unwrap();

    // positive case: the tail container appends fine
    session
        .append_pod_vec::<TwoDynLog, Note>(points_off, &[Note { id: 2, score: 2.0 }])
        .unwrap();

    // negative case: the first dyn container is not the tail
    let err = session
        .append_pod_vec::<TwoDynLog, u32>(vals_off, &[3])
        .unwrap_err();
    assert!(
        err.to_string().contains("not at the payload tail"),
        "unexpected error: {err}"
    );
    session.flush().unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<TwoDynLog>();
    assert_eq!(reader.vals.len(), 2);
    assert_eq!(reader.points.len(), 2);
    assert_eq!(reader.points[1], Note { id: 2, score: 2.0 });
}

#[test]
fn test_append_rejects_when_byref_data_follows() {
    // byref (postponed) data is written after all inline dyn values, so the
    // vals block is no longer the payload tail.
    let tmp = NamedTempFile::new().unwrap();
    let log = ByRefAheadLog {
        id: 3,
        notes: vec!["n1".to_string()],
        vals: vec![7, 8],
    };
    write_to_file(&log, tmp.path(), false).unwrap();

    let mut session = PatchSession::open_typed::<ByRefAheadLog>(tmp.path()).unwrap();
    let reader = session.read::<ByRefAheadLog>().unwrap();
    let vals_off = session.offset_of(&reader.vals[0]).unwrap();

    let err = session
        .append_pod_vec::<ByRefAheadLog, u32>(vals_off, &[9])
        .unwrap_err();
    assert!(
        err.to_string().contains("not at the payload tail"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_append_rejects_wrong_root_layout() {
    let tmp = NamedTempFile::new().unwrap();
    write_default_log(tmp.path());

    let mut session = PatchSession::open_typed::<PatchLog>(tmp.path()).unwrap();
    let reader = session.read::<PatchLog>().unwrap();
    let elems_off = session.offset_of(&reader.points[0]).unwrap();

    // Note has no dynamic region at all -> sentinel check fires.
    let err = session
        .append_pod_vec::<Note, Note>(elems_off, &[Note::default()])
        .unwrap_err();
    assert!(
        err.to_string().contains("no dynamic region"),
        "unexpected error: {err}"
    );
}
