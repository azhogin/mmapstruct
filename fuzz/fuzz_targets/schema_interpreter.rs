#![no_main]
//! Arbitrary payload through the static SchemaCtx (fixed valid schema).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    mmapstruct::fuzzing::fuzz_schema_interpreter(data);
});
