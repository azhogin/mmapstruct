//! Fuzz-target functions, shared between the cargo-fuzz harnesses (`fuzz/`,
//! nightly, coverage-guided) and the stable corpus-replay test
//! (`tests/fuzz_regression.rs`).
//!
//! Policy (see `../.agents/ideas/idea_error_handling.md` §3.8):
//! - **default build**: the library is trusted-data by design — panics on
//!   corrupt input are expected. Targets catch every panic and return; the
//!   fuzzers look for OOM/hangs via libFuzzer's rss/malloc limits instead.
//! - **`strict-validation` build**: every panic must be a *meaningful*
//!   mmapstruct assert. Raw panics ("index out of bounds", "slice index",
//!   "range end index", arithmetic overflow, "index out of range") indicate
//!   unchecked arithmetic / bounds math leaking through the strict layer —
//!   they are re-raised so libFuzzer (or the replay test) records them as
//!   findings.
//!
//! `fuzz_typed_read` feeds a *headerless* payload: the layout-hash check
//! would reject random bytes before the readers are exercised, so the
//! corpus payloads bypass the header entirely and the fuzzers get the full
//! byte chaos (offset tables, List chains, Option bytes, discriminants).

#![cfg(feature = "fuzzing")]

use crate::file_reader::BufReader;
use crate::schema::GetSchema;
use crate::{List, MmapStruct};
use bytemuck::{Pod, Zeroable};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Raw panic messages that must never escape a `strict-validation` build.
const RAW_PANIC_PATTERNS: &[&str] = &[
    "index out of bounds",
    "index out of range",
    "slice index starts at",
    "range end index",
    "range start index",
    "with overflow",
    "capacity overflow",
    "capacity overflowed",
    "misaligned pointer dereference",
];

/// libfuzzer-sys installs a panic hook that aborts the process on ANY panic,
/// so `catch_unwind` never gets a chance to apply the policy. Replace the
/// hook once: raw panics (findings) keep the previous (verbose) hook,
/// meaningful strict asserts are silenced so they unwind into `fuzz_run`.
fn install_fuzz_panic_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if cfg!(feature = "strict-validation") && is_raw_info(info) {
                prev(info);
            }
            // otherwise: silence — the payload is handled by fuzz_run.
        }));
    });
}

fn is_raw_info(info: &std::panic::PanicHookInfo<'_>) -> bool {
    let msg = info.payload_as_str().map(|s| s.to_string());
    match msg {
        Some(m) => RAW_PANIC_PATTERNS.iter().any(|p| m.contains(p)),
        None => true,
    }
}

/// Run `f` under the fuzz panic policy: swallow everything in default
/// builds; in `strict-validation` builds re-raise raw bounds/overflow
/// panics (finding) and swallow meaningful asserts.
pub fn fuzz_run(f: impl FnOnce()) {
    install_fuzz_panic_hook();
    let result = catch_unwind(AssertUnwindSafe(f));
    if let Err(payload) = result {
        if cfg!(feature = "strict-validation") && is_raw_panic(&payload) {
            // Re-raise: this is a finding (libFuzzer artifact / replay test
            // failure), not an expected strict assert.
            std::panic::resume_unwind(payload);
        }
    }
}

fn is_raw_panic(payload: &Box<dyn std::any::Any + Send>) -> bool {
    let msg: Option<String> = if let Some(s) = payload.downcast_ref::<&'static str>() {
        Some((*s).to_string())
    } else if let Some(s) = payload.downcast_ref::<String>() {
        Some(s.clone())
    } else {
        None
    };
    match msg {
        Some(m) => RAW_PANIC_PATTERNS.iter().any(|p| m.contains(p)),
        // Non-string payloads (assert_eq messages from unimplemented paths)
        // count as raw: strict asserts always carry string messages.
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Corpus types: one fat file format exercising every reader branch.
// ---------------------------------------------------------------------------

#[derive(MmapStruct, Debug, Clone, Copy, Pod, Zeroable, Default)]
#[repr(C)]
pub struct FuzzPoint {
    pub x: u32,
    pub y: u32,
}

#[derive(MmapStruct, Debug)]
pub struct FuzzItem {
    pub a: String,
    pub b: Option<u64>,
}

#[derive(MmapStruct, Debug)]
pub enum FuzzKind {
    Idle,
    Run { id: u32, label: String },
    Meta { code: u32 },
}

#[derive(MmapStruct, Debug)]
pub struct FuzzFile {
    pub id: u32,
    pub name: String,
    pub tags: Vec<u32>,
    pub items: Vec<FuzzItem>,
    pub notes: List<String>,
    pub kind: FuzzKind,
    pub pair: Option<FuzzPoint>,
    pub point: FuzzPoint,
}

// ---------------------------------------------------------------------------
// Targets
// ---------------------------------------------------------------------------

/// Arbitrary bytes as a headerless `FuzzFile` payload: exercise the
/// derive-generated readers (fixed section, offset table, List chains,
/// Option bytes, enum discriminants) plus the container cursor APIs.
pub fn fuzz_typed_read(data: &[u8]) {
    fuzz_run(|| {
        let buf = data.to_vec();
        let br = BufReader::new(buf);
        let reader = br.read::<FuzzFile>();
        // Walk every field: cursor APIs must stay in-bounds for any input.
        let _ = reader.id;
        let _ = reader.name;
        let _ = reader.tags.len();
        for i in 0..reader.tags.len() {
            let _ = reader.tags[i];
        }
        let _ = reader.items.len();
        for i in 0..reader.items.len() {
            if let Some(item) = reader.items.get(i) {
                let _ = item.a;
                let _ = item.b;
            }
        }
        let _ = reader.notes.len();
        for note in reader.notes.iter() {
            let _ = note;
        }
        if let Some(i) = reader.notes.get(0) {
            let _ = i;
        }
        match &reader.kind {
            FuzzKindReader::Idle => {}
            FuzzKindReader::Run { id, label } => {
                let _ = (*id, *label);
            }
            FuzzKindReader::Meta { code } => {
                let _ = *code;
            }
        }
        if let Some(p) = reader.pair {
            let _ = (p.x, p.y);
        }
        let _ = (reader.point.x, reader.point.y);
    })
}

/// Arbitrary payload through the static `SchemaCtx` interpreter with a
/// compile-time valid schema (the root value type is `&'static`, so the
/// "schema" half needs no fuzzing here — see `fuzz_runtime_schema` in
/// mmapstruct-schema for the corrupted-schema target).
pub fn fuzz_schema_interpreter(payload: &[u8]) {
    fuzz_run(|| {
        let ctx = crate::SchemaCtx::new(payload);
        let root = FuzzFile::root_value(payload.len() as u64);
        for name in [
            "id", "name", "tags", "items", "notes", "kind", "pair", "point",
        ] {
            if let Some(v) = ctx.field_get(&root, name) {
                let _ = ctx.array_len(&v);
                for i in 0..8 {
                    if let Some(e) = ctx.array_get(&v, i) {
                        for f in ["a", "b", "x", "y"] {
                            let _ = ctx.field_get(&e, f);
                        }
                        let _ = ctx.read_u32(&e);
                        let _ = ctx.read_u64(&e);
                    }
                }
            }
        }
    })
}
