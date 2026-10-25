//! Regenerate the committed fuzz seeds in `fuzz/seeds/`:
//!
//! ```bash
//! cargo test -p mmapstruct --features fuzzing fuzz_seed_corpus
//! ```
//!
//! Seeds are valid payloads (and schema||payload pairs for the runtime
//! target) covering every reader branch; libFuzzer mutates from them via
//! `cargo +nightly fuzz run <target> fuzz/seeds/<target>`.

#![cfg(feature = "fuzzing")]

use mmapstruct::fuzzing::{FuzzFile, FuzzItem, FuzzKind, FuzzPoint};
use mmapstruct::{write_to_buf_raw, List};
use std::path::Path;

fn sample_files() -> Vec<FuzzFile> {
    vec![
        // Fully populated.
        FuzzFile {
            id: 1,
            name: "hello fuzz".into(),
            tags: vec![1, 2, 3],
            items: vec![
                FuzzItem {
                    a: "one".into(),
                    b: Some(10),
                },
                FuzzItem {
                    a: "two".into(),
                    b: None,
                },
            ],
            notes: List::from_vec(vec!["n1".into(), "n2".into()]),
            kind: FuzzKind::Run {
                id: 7,
                label: "run".into(),
            },
            pair: Some(FuzzPoint { x: 1, y: 2 }),
            point: FuzzPoint { x: 3, y: 4 },
        },
        // Empty containers everywhere.
        FuzzFile {
            id: 2,
            name: String::new(),
            tags: vec![],
            items: vec![],
            notes: List::new(),
            kind: FuzzKind::Idle,
            pair: None,
            point: FuzzPoint::default(),
        },
        // Meta variant, single-element containers, long string.
        FuzzFile {
            id: 3,
            name: "x".repeat(200),
            tags: vec![u32::MAX],
            items: vec![FuzzItem {
                a: "edge".into(),
                b: Some(u64::MAX),
            }],
            notes: List::from_vec(vec!["single".into()]),
            kind: FuzzKind::Meta { code: u32::MAX },
            pair: Some(FuzzPoint::default()),
            point: FuzzPoint { x: u32::MAX, y: 0 },
        },
    ]
}

fn write_seed(dir: &Path, target: &str, name: &str, bytes: &[u8]) {
    let d = dir.join(target);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join(name), bytes).unwrap();
}

#[test]
fn fuzz_seed_corpus() {
    let dir = Path::new("../fuzz/seeds");
    let files = sample_files();
    for (i, f) in files.iter().enumerate() {
        let raw = write_to_buf_raw(f);
        // typed_read + schema_interpreter: headerless payload.
        write_seed(dir, "typed_read", &format!("sample{i}"), &raw);
        write_seed(dir, "schema_interpreter", &format!("sample{i}"), &raw);
        // runtime_schema: [schema bytes || payload].
        let schema = mmapstruct_schema::build_serialized_schema::<FuzzFile>();
        let schema_bytes = mmapstruct::write_to_buf_raw(&schema);
        let mut pair = schema_bytes.clone();
        pair.extend_from_slice(&raw);
        write_seed(dir, "runtime_schema", &format!("sample{i}"), &pair);
        // tiny seeds help libFuzzer start small
        write_seed(dir, "typed_read", &format!("empty{i}"), &[]);
    }
    // tiny runtime seed (8+ bytes so the schema/payload split is non-empty)
    write_seed(dir, "runtime_schema", "tiny", &[0u8; 16]);
}
