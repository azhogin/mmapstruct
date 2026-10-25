pub mod builder;
pub mod context;
pub mod format;
pub mod helpers;
pub mod navigate;
pub mod registry;
pub mod schema;
pub mod value;

// Re-export key schema types for convenience
pub use schema::{
    ArrayLayoutDef, ArrayLayoutDefReader, FieldDef, FieldDefReader, PrimitiveDef, Schema,
    SchemaReader, TypeDef, TypeDefReader, VariantDef, VariantDefReader,
};

// Re-export context public API
pub use context::RuntimeSchemaCtx;

// Re-export value types
pub use value::{HashMapDef, SchemaValue};

// Re-export formatting
pub use format::format_schema_value;

// Re-export navigation
pub use navigate::navigate_path;

// Re-export builder public API
pub use builder::{
    build_serialized_schema, ensure_registry_schema, write_split, write_split_with,
    write_to_file_split, write_to_file_split_with, write_to_file_with_embedded_schema,
    write_with_schema,
};
