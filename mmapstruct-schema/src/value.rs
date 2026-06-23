//! Lightweight value types for schema-based navigation.

/// Lightweight value descriptor for schema-based navigation.
/// Analogous to `mmapstruct::schema::Value` but carries a `type_idx` into
/// the schema's type table instead of a `&'static DynValueType`.
#[derive(Clone, Copy, Debug)]
pub struct SchemaValue {
    pub offset: u64,
    pub limit: u64,
    pub type_idx: u32,
}

impl SchemaValue {
    pub fn new(offset: u64, limit: u64, type_idx: u32) -> Self {
        Self {
            offset,
            limit,
            type_idx,
        }
    }
}


#[derive(Clone, Copy)]
pub struct HashMapDef {
    pub key_is_pod: bool,
    pub val_is_pod: bool,
}
