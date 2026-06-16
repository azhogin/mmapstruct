use mmapstruct::{DynContext, Value, DynValueType};

mod tokenizer;
mod parser;
mod interpreter;
#[cfg(feature = "json")]
pub mod json_conv;

pub use tokenizer::tokenize;
pub use parser::parse;
pub use interpreter::interpret;

pub use tokenizer::Token;
pub use parser::Expr;

/// Result of evaluating a query expression — either a buffer-backed value
/// or a computed value from an aggregation function.
#[derive(Clone, Copy)]
pub enum EvalResult {
    /// Value backed by data in the buffer.
    Buf(Value),
    /// Unsigned integer (e.g. `count()`).
    U64(u64),
    /// Signed integer (e.g. `min(.field)` on signed types).
    I64(i64),
    /// Float (e.g. `avg(.field)`, or `min/max` on float types).
    F64(f64),
}

/// Render a `Value` as a JSON-like string using a `DynContext`.
///
/// ```
/// # use mmapstruct::schema::{DynValueType, Value};
/// # let ctx = mmapstruct::DynContext::new(&[]);
/// // A Value for a simple u32 is read as a number:
/// let dummy = Value::new(0, 4, &DynValueType::U32);
/// // We can't easily construct a context with data in a doctest here.
/// ```
pub fn format_value<'a>(ctx: &DynContext<'a>, val: &Value) -> String {
    let mut out = String::new();
    write_val(ctx, val, &mut out);
    out
}

/// Render an `EvalResult` as a JSON-like string.
pub fn format_result(ctx: &DynContext<'_>, result: &EvalResult) -> String {
    match result {
        EvalResult::Buf(v) => format_value(ctx, v),
        EvalResult::U64(n) => n.to_string(),
        EvalResult::I64(n) => n.to_string(),
        EvalResult::F64(n) => {
            if *n == n.trunc() && !n.is_infinite() {
                (n.trunc() as i64).to_string()
            } else {
                n.to_string()
            }
        }
    }
}

fn write_val<'a>(ctx: &DynContext<'a>, val: &Value, out: &mut String) {
    match val.schema {
        DynValueType::U32 => out.push_str(&ctx.read_u32(val).to_string()),
        DynValueType::I32 => out.push_str(&ctx.read_i32(val).to_string()),
        DynValueType::U64 => out.push_str(&ctx.read_u64(val).to_string()),
        DynValueType::I64 => out.push_str(&ctx.read_i64(val).to_string()),
        DynValueType::F32 => out.push_str(&ctx.read_f32(val).to_string()),
        DynValueType::F64 => out.push_str(&ctx.read_f64(val).to_string()),
        DynValueType::Bool => {
            out.push_str(if ctx.read_bool(val) { "true" } else { "false" });
        }
        DynValueType::Str(_) => {
            out.push('"');
            out.push_str(ctx.read_str(val));
            out.push('"');
        }
        DynValueType::U16 => out.push_str(&ctx.read_u16(val).to_string()),
        DynValueType::I16 => out.push_str(&ctx.read_i16(val).to_string()),
        DynValueType::U8 => out.push_str(&ctx.read_u8(val).to_string()),
        DynValueType::I8 => out.push_str(&ctx.read_i8(val).to_string()),
        DynValueType::Usize => out.push_str(&ctx.read_usize(val).to_string()),
        DynValueType::Isize => out.push_str(&ctx.read_isize(val).to_string()),
        DynValueType::PodStruct { .. } => {
            out.push_str("[pod]");
        }
        DynValueType::Struct(_) => {
            let schema = ctx.get_struct_schema(val);
            out.push('{');
            for (i, field) in schema.fields.iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                out.push('"');
                out.push_str(field.name);
                out.push_str("\": ");
                if let Some(fv) = ctx.field_get(val, field.name) {
                    write_val(ctx, &fv, out);
                } else {
                    out.push_str("null");
                }
            }
            out.push('}');
        }
        DynValueType::Array(_) => {
            out.push('[');
            let len = ctx.array_len(val);
            for i in 0..len {
                if i > 0 { out.push_str(", "); }
                if let Some(elem) = ctx.array_get(val, i) {
                    write_val(ctx, &elem, out);
                } else {
                    out.push_str("null");
                }
            }
            out.push(']');
        }
        DynValueType::Option(_) => {
            out.push_str("null");
        }
        DynValueType::ByRef(inner) => {
            // Re-wrap with the inner schema and render
            let inner_val = Value::new(val.offset, val.limit, inner);
            write_val(ctx, &inner_val, out);
        }
        DynValueType::HashMap(_) => {
            out.push('[');
            let len = ctx.hashmap_len(val);
            for i in 0..len {
                if i > 0 { out.push_str(", "); }
                if let Some((k, v)) = ctx.hashmap_get_index(val, i) {
                    out.push('[');
                    write_val(ctx, &k, out);
                    out.push_str(", ");
                    write_val(ctx, &v, out);
                    out.push(']');
                }
            }
            out.push(']');
        }
    }
}

/// Parse and evaluate a query string against a root `Value`.
///
/// The caller provides a `DynContext` (wrapping the underlying byte buffer)
/// and a root `Value` (typically the top-level struct or array).
///
/// ```
/// # use mmapstruct::{MmapStruct, MmapRead, DynContext, GetSchema};
/// # use mmapstruct::schema::Value;
/// # #[derive(MmapStruct)] #[mmap(scope)] struct Rec { x: u32, y: String }
/// # let data = mmapstruct::write_to_buf_raw(&Rec { x: 42, y: "hello".into() });
/// let ctx = DynContext::new(&data);
/// let root = <Rec as GetSchema>::root_value(data.len() as u64);
/// let result = mmapview::query_from("/x", &ctx, &root).unwrap();
/// ```
pub fn query_from<'a>(q: &str, ctx: &DynContext<'a>, root: &Value) -> Result<Vec<EvalResult>, String> {
    let tokens = tokenize(q)?;
    let expr = parse(&tokens)?;
    interpret(&expr, ctx, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmapstruct::{MmapStruct, GetSchema};

    #[derive(MmapStruct, Debug)]
    #[mmap(scope)]
    struct Rec {
        id: u32,
        name: String,
    }

    #[test]
    fn test_format_value_struct_ref() {
        let r = Rec { id: 42, name: "hello".into() };
        let buf = mmapstruct::write_to_buf_raw(&r);
        let ctx = DynContext::new(&buf);
        let root = <Rec as GetSchema>::root_value(buf.len() as u64);
        let s = format_value(&ctx, &root);
        assert_eq!(s, "{\"id\": 42, \"name\": \"hello\"}");
    }

    #[test]
    fn test_format_value_array_of_structs() {
        #[derive(MmapStruct, Debug)]
        struct RecFile {
            items: Vec<Rec>,
        }
        let w = RecFile {
            items: vec![
                Rec { id: 1, name: "a".into() },
                Rec { id: 2, name: "b".into() },
            ],
        };
        let buf = mmapstruct::write_to_buf_raw(&w);
        let ctx = DynContext::new(&buf);
        let root = <RecFile as GetSchema>::root_value(buf.len() as u64);
        let items_val = ctx.field_get(&root, "items").unwrap();
        let s = format_value(&ctx, &items_val);
        assert_eq!(s, "[{\"id\": 1, \"name\": \"a\"}, {\"id\": 2, \"name\": \"b\"}]");
    }

    #[test]
    fn test_format_value_query_result() {
        #[derive(MmapStruct, Debug)]
        struct RecFile {
            items: Vec<Rec>,
        }
        let records = vec![
            Rec { id: 1, name: "sensor_a".into() },
            Rec { id: 5, name: "actuator_b".into() },
            Rec { id: 10, name: "sensor_c".into() },
        ];
        let w = RecFile { items: records };
        let buf = mmapstruct::write_to_buf_raw(&w);
        let ctx = DynContext::new(&buf);
        let root = <RecFile as GetSchema>::root_value(buf.len() as u64);
        let result = query_from("/items[?id>3]", &ctx, &root).unwrap();
        let mut out = String::new();
        out.push('[');
        for (i, v) in result.iter().enumerate() {
            if i > 0 { out.push_str(", "); }
            if let EvalResult::Buf(val) = v {
                write_val(&ctx, val, &mut out);
            }
        }
        out.push(']');
        assert_eq!(out, "[{\"id\": 5, \"name\": \"actuator_b\"}, {\"id\": 10, \"name\": \"sensor_c\"}]");
    }
}
