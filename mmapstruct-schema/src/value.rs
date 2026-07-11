//! Lightweight value types for schema-based navigation.

/// Lightweight value descriptor for schema-based navigation.
/// Analogous to `mmapstruct::schema::Value` but carries a `type_idx` into
/// the schema's type table instead of a `&'static DynValueType`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SchemaValue {
    pub mmap_begin: u64,
    pub mmap_end: u64,
    pub type_idx: u32,
}

impl SchemaValue {
    pub fn new(mmap_begin: u64, mmap_end: u64, type_idx: u32) -> Self {
        Self {
            mmap_begin,
            mmap_end,
            type_idx,
        }
    }

    pub fn begin(&self) -> usize {
        self.mmap_begin as usize
    }

    pub fn end(&self) -> usize {
        self.mmap_end as usize
    }
}


#[derive(Clone, Copy)]
pub struct HashMapDef {
    pub key_is_pod: bool,
    pub val_is_pod: bool,
}
