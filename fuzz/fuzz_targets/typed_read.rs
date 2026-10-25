#![no_main]
//! Arbitrary bytes as a headerless FuzzFile payload (see mmapstruct/src/fuzzing.rs).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    mmapstruct::fuzzing::fuzz_typed_read(data);
});
