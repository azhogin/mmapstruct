use mmapstruct::{DynContext, DynValueType, Value};
use mmapstruct_schema::context::{SchemaDynContext, SchemaValue};

/// Trait abstracting over `DynContext` and `SchemaDynContext` for the
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
}

impl<'a> QueryContext<'a> for DynContext<'a> {
    type Val = Value;

    fn field_get(&self, val: &Value, name: &str) -> Option<Value> {
        DynContext::field_get(self, val, name)
    }

    fn array_len(&self, val: &Value) -> usize {
        DynContext::array_len(self, val)
    }

    fn array_get(&self, val: &Value, idx: usize) -> Option<Value> {
        DynContext::array_get(self, val, idx)
    }

    fn read_as_i64(&self, val: &Value) -> Option<i64> {
        DynContext::read_as_i64(self, val)
    }

    fn read_as_f64(&self, val: &Value) -> Option<f64> {
        DynContext::read_as_f64(self, val)
    }

    fn read_str(&self, val: &Value) -> &'a str {
        DynContext::read_str(self, val)
    }

    fn read_str_opt(&self, val: &Value) -> Option<&'a str> {
        DynContext::read_str_opt(self, val)
    }

    fn is_array_type(&self, val: &Value) -> bool {
        matches!(val.schema, DynValueType::Array(_))
    }

    fn is_str_type(&self, val: &Value) -> bool {
        matches!(val.schema, DynValueType::Str(_))
    }
}

impl<'a> QueryContext<'a> for SchemaDynContext<'a> {
    type Val = SchemaValue;

    fn field_get(&self, val: &SchemaValue, name: &str) -> Option<SchemaValue> {
        SchemaDynContext::field_get(self, val, name)
    }

    fn array_len(&self, val: &SchemaValue) -> usize {
        SchemaDynContext::array_len(self, val)
    }

    fn array_get(&self, val: &SchemaValue, idx: usize) -> Option<SchemaValue> {
        SchemaDynContext::array_get(self, val, idx)
    }

    fn read_as_i64(&self, val: &SchemaValue) -> Option<i64> {
        SchemaDynContext::read_as_i64(self, val)
    }

    fn read_as_f64(&self, val: &SchemaValue) -> Option<f64> {
        SchemaDynContext::read_as_f64(self, val)
    }

    fn read_str(&self, val: &SchemaValue) -> &'a str {
        SchemaDynContext::read_str(self, val)
    }

    fn read_str_opt(&self, val: &SchemaValue) -> Option<&'a str> {
        SchemaDynContext::read_str_opt(self, val)
    }

    fn is_array_type(&self, val: &SchemaValue) -> bool {
        SchemaDynContext::is_array_type(self, val)
    }

    fn is_str_type(&self, val: &SchemaValue) -> bool {
        SchemaDynContext::is_str_type(self, val)
    }
}
