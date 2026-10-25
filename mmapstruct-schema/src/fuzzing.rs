//! The runtime-schema fuzz target: arbitrary bytes split into a schema and
//! a payload, feeding [`RuntimeSchemaCtx`] — the doubly-corrupted input
//! fuzzes the schema parser (`SchemaReader`, itself mmapstruct-encoded) and
//! the runtime interpreter. Panic policy: see `mmapstruct/src/fuzzing.rs`.

#![cfg(feature = "fuzzing")]

use crate::context::RuntimeSchemaCtx;
use crate::value::SchemaValue;
use mmapstruct::fuzzing::fuzz_run;
use mmapstruct::traits::MmapRead;

/// The root type index comes from the schema itself; navigation mirrors
/// `mmapview q` (payload-relative coordinates).
pub fn fuzz_runtime_schema(data: &[u8]) {
    if data.len() < 8 {
        return;
    }
    let mid = data.len() / 2;
    let (schema, payload) = (&data[..mid], &data[mid..]);
    fuzz_run(|| {
        // Plain locals: the context borrows them for the duration of the
        // call — no leaks (LeakSanitizer runs the fuzz builds).
        let schema_bytes = schema.to_vec();
        let payload_vec = payload.to_vec();
        let ctx = RuntimeSchemaCtx::new(&payload_vec, &schema_bytes);
        let schema_reader =
            <crate::schema::Schema as MmapRead>::read(&schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;
        let root = SchemaValue::new(0, payload_vec.len() as u64, root_type);
        for name in [
            "id", "name", "tags", "items", "notes", "kind", "pair", "point",
        ] {
            if let Some(v) = ctx.field_get(&root, name) {
                let _ = ctx.array_len(&v);
                for i in 0..8 {
                    if let Some(e) = ctx.array_get(&v, i) {
                        let _ = ctx.read_u32(&e);
                        let _ = ctx.read_u64(&e);
                        for f in ["a", "b", "x", "y"] {
                            let _ = ctx.field_get(&e, f);
                        }
                    }
                }
            }
        }
    })
}
