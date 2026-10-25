//! Runtime-schema corpus replay (see mmapstruct/tests/fuzz_regression.rs
//! for the policy):
//!
//! ```bash
//! cargo test -p mmapstruct-schema --features fuzzing,strict-validation fuzz_regression
//! ```

#![cfg(feature = "fuzzing")]

#[test]
fn fuzz_regression_runtime_schema() {
    let dir = std::path::Path::new("../fuzz/seeds/runtime_schema");
    let mut found = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let data = std::fs::read(&p).unwrap();
                mmapstruct::fuzzing::fuzz_run(|| {
                    mmapstruct_schema::fuzzing::fuzz_runtime_schema(&data);
                });
                found += 1;
            }
        }
    }
    assert!(found > 0, "no seeds found in {}", dir.display());
}
