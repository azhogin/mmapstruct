//! Replay the committed fuzz seeds through the shared fuzz-target functions.
//!
//! Discovery lives in `cargo +nightly fuzz run <target> fuzz/seeds/<target>`;
//! this test is the regression harness: every artifact added to
//! `fuzz/seeds/` must keep passing.
//!
//! ```bash
//! cargo test -p mmapstruct --features fuzzing,strict-validation fuzz_regression
//! ```
//!
//! Under `strict-validation` the shared targets re-raise raw bounds/overflow
//! panics (findings) and swallow meaningful mmapstruct asserts — so this
//! test fails exactly when strict validation lets a raw panic through.
//! Under default features every panic is swallowed (trusted-data policy):
//! the replay then only exercises the code paths (no UB/hangs).

#![cfg(feature = "fuzzing")]

use std::path::Path;

fn replay_dir(target: &str, f: impl Fn(&[u8])) {
    let dir = Path::new("../fuzz/seeds").join(target);
    let mut found = 0;
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let data = std::fs::read(&p).unwrap();
                f(&data);
                found += 1;
            }
        }
    }
    assert!(found > 0, "no seeds found in {}", dir.display());
}

#[test]
fn fuzz_regression_typed_read() {
    replay_dir("typed_read", mmapstruct::fuzzing::fuzz_typed_read);
}

#[test]
fn fuzz_regression_schema_interpreter() {
    replay_dir(
        "schema_interpreter",
        mmapstruct::fuzzing::fuzz_schema_interpreter,
    );
}
