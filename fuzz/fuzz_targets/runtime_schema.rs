#![no_main]
//! Arbitrary bytes split into schema + payload: RuntimeSchemaCtx over a
//! doubly-corrupted input (see mmapstruct-schema/src/fuzzing.rs).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    mmapstruct_schema::fuzzing::fuzz_runtime_schema(data);
});
