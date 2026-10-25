//! Schema resolution — re-exports from `mmapstruct-schema::schema_source`
//! (the logic lives there so patching tools can resolve a file's schema
//! without this frontend crate).

pub use mmapstruct_schema::schema_source::{load_schema_full, resolve_schema, SchemaResult};
