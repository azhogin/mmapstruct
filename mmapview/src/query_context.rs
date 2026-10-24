use mmapstruct::{SchemaCtx, DynValueType, Value};
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::TypeDefReader;
use mmapstruct_schema::SchemaValue;

/// Trait abstracting over `SchemaCtx` and `RuntimeSchemaCtx` for the
/// query interpreter. Allows the same filter/aggregate logic to work
/// with both compile-time and schema-based navigation.
pub trait QueryContext<'a> {
    type Val: Copy;

    fn field_get(&self, val: &Self::Val, name: &str) -> Option<Self::Val>;
    fn array_len(&self, val: &Self::Val) -> usize;
    fn array_get(&self, val: &Self::Val, idx: usize) -> Option<Self::Val>;
    fn read_as_i64(&self, val: &Self::Val) -> Option<i64>;
    fn read_as_f64(&self, val: &Self::Val) -> Option<f64>;
    fn read_str(&self, val: &Self::Val) -> &'a str;
    fn read_str_opt(&self, val: &Self::Val) -> Option<&'a str>;
    fn is_array_type(&self, val: &Self::Val) -> bool;
    fn is_str_type(&self, val: &Self::Val) -> bool;

    /// Name of the active enum variant for `?$variant=...` filters.
    ///
    /// Returns the full name as stored in the schema (`"Enum::Variant"`).
    /// `None` when the value is not an enum.
    fn variant_name(&self, val: &Self::Val) -> Option<&'a str>;
}

impl<'a> QueryContext<'a> for SchemaCtx<'a> {
    type Val = Value;

    fn field_get(&self, val: &Value, name: &str) -> Option<Value> {
        SchemaCtx::field_get(self, val, name)
    }

    fn array_len(&self, val: &Value) -> usize {
        SchemaCtx::array_len(self, val)
    }

    fn array_get(&self, val: &Value, idx: usize) -> Option<Value> {
        SchemaCtx::array_get(self, val, idx)
    }

    fn read_as_i64(&self, val: &Value) -> Option<i64> {
        SchemaCtx::read_as_i64(self, val)
    }

    fn read_as_f64(&self, val: &Value) -> Option<f64> {
        SchemaCtx::read_as_f64(self, val)
    }

    fn read_str(&self, val: &Value) -> &'a str {
        SchemaCtx::read_str(self, val)
    }

    fn read_str_opt(&self, val: &Value) -> Option<&'a str> {
        SchemaCtx::read_str_opt(self, val)
    }

    fn is_array_type(&self, val: &Value) -> bool {
        matches!(val.schema, DynValueType::Array(_))
    }

    fn is_str_type(&self, val: &Value) -> bool {
        matches!(val.schema, DynValueType::Str(_))
    }

    fn variant_name(&self, val: &Value) -> Option<&'a str> {
        SchemaCtx::enum_variant(self, val).map(|(schema, _)| schema.name)
    }
}

impl<'a> QueryContext<'a> for RuntimeSchemaCtx<'a> {
    type Val = SchemaValue;

    fn field_get(&self, val: &SchemaValue, name: &str) -> Option<SchemaValue> {
        RuntimeSchemaCtx::field_get(self, val, name)
    }

    fn array_len(&self, val: &SchemaValue) -> usize {
        RuntimeSchemaCtx::array_len(self, val) as usize
    }

    fn array_get(&self, val: &SchemaValue, idx: usize) -> Option<SchemaValue> {
        RuntimeSchemaCtx::array_get(self, val, idx as u64)
    }

    fn read_as_i64(&self, val: &SchemaValue) -> Option<i64> {
        RuntimeSchemaCtx::read_as_i64(self, val)
    }

    fn read_as_f64(&self, val: &SchemaValue) -> Option<f64> {
        RuntimeSchemaCtx::read_as_f64(self, val)
    }

    fn read_str(&self, val: &SchemaValue) -> &'a str {
        RuntimeSchemaCtx::read_str(self, val)
    }

    fn read_str_opt(&self, val: &SchemaValue) -> Option<&'a str> {
        if val.mmap_begin >= val.mmap_end {
            None
        } else {
            Some(self.read_str(val))
        }
    }

    fn is_array_type(&self, val: &SchemaValue) -> bool {
        RuntimeSchemaCtx::is_array_type(self, val.type_idx)
    }

    fn is_str_type(&self, val: &SchemaValue) -> bool {
        RuntimeSchemaCtx::is_str_type(self, val.type_idx)
    }

    fn variant_name(&self, val: &SchemaValue) -> Option<&'a str> {
        let (variant_idx, _) = RuntimeSchemaCtx::enum_variant(self, val)?;
        match self.type_reader(variant_idx) {
            TypeDefReader::Struct { name, .. } => Some(*name),
            _ => None,
        }
    }
}
