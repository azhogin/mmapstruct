use crate::EvalResult;
use mmapstruct::{ArrayLayout, DynValueType, SchemaCtx, StructSchema, Value};

/// Convert a runtime `Value` to a `serde_json::Value` for HTTP output.
pub fn to_json<'a>(ctx: &SchemaCtx<'a>, val: &Value) -> serde_json::Value {
    match val.schema {
        DynValueType::U8 => serde_json::json!(ctx.read_u8(val)),
        DynValueType::U16 => serde_json::json!(ctx.read_u16(val)),
        DynValueType::U32 => serde_json::json!(ctx.read_u32(val)),
        DynValueType::U64 => serde_json::json!(ctx.read_u64(val)),
        DynValueType::I8 => serde_json::json!(ctx.read_i8(val)),
        DynValueType::I16 => serde_json::json!(ctx.read_i16(val)),
        DynValueType::I32 => serde_json::json!(ctx.read_i32(val)),
        DynValueType::I64 => serde_json::json!(ctx.read_i64(val)),
        DynValueType::F32 => serde_json::json!(ctx.read_f32(val)),
        DynValueType::F64 => serde_json::json!(ctx.read_f64(val)),
        DynValueType::Bool => serde_json::json!(ctx.read_bool(val)),
        DynValueType::Usize => serde_json::json!(ctx.read_usize(val)),
        DynValueType::Isize => serde_json::json!(ctx.read_isize(val)),
        DynValueType::Str(_) => {
            let s = ctx.read_str(val);
            serde_json::Value::String(s.to_string())
        }
        DynValueType::PodStruct { .. } => serde_json::Value::Null,
        DynValueType::Struct(_) => {
            let schema = ctx.get_struct_schema(val);
            let mut map = serde_json::Map::new();
            for fd in schema.fields {
                if let Some(fv) = ctx.field_get(val, fd.name) {
                    map.insert(fd.name.to_string(), to_json(ctx, &fv));
                }
            }
            serde_json::Value::Object(map)
        }
        DynValueType::Array(_) => {
            let len = ctx.array_len(val);
            let mut items = Vec::with_capacity(len);
            for i in 0..len {
                if let Some(elem) = ctx.array_get(val, i) {
                    items.push(to_json(ctx, &elem));
                }
            }
            serde_json::Value::Array(items)
        }
        DynValueType::Option(_) => serde_json::Value::Null,
        DynValueType::ByRef(inner) => {
            let inner_val = Value::new(val.offset, val.limit, inner);
            to_json(ctx, &inner_val)
        }
        DynValueType::HashMap(_schema) => {
            let len = ctx.hashmap_len(val);
            let mut items = Vec::with_capacity(len);
            for i in 0..len {
                if let Some((key, val)) = ctx.hashmap_get_index(val, i) {
                    items.push(serde_json::json!([to_json(ctx, &key), to_json(ctx, &val)]));
                }
            }
            serde_json::Value::Array(items)
        }
        DynValueType::Lazy(resolve) => to_json(ctx, &Value::new(val.offset, val.limit, resolve())),
        DynValueType::Enum(_es) => {
            if let Some((vs, _data_off)) = ctx.enum_variant(val) {
                let mut map = serde_json::Map::new();
                map.insert("$variant".into(), serde_json::json!(vs.name));
                for fd in vs.fields {
                    if let Some(fv) = ctx.field_get(val, fd.name) {
                        map.insert(fd.name.to_string(), to_json(ctx, &fv));
                    }
                }
                serde_json::Value::Object(map)
            } else {
                serde_json::Value::Null
            }
        }
    }
}

/// Convert an `EvalResult` to a `serde_json::Value` for HTTP output.
pub fn result_to_json<'a>(ctx: &SchemaCtx<'a>, result: &EvalResult) -> serde_json::Value {
    match result {
        EvalResult::Buf(v) => to_json(ctx, v),
        EvalResult::U64(n) => serde_json::json!(n),
        EvalResult::I64(n) => serde_json::json!(n),
        EvalResult::F64(n) => serde_json::json!(n),
    }
}

/// Convert a static `DynValueType` to a JSON-schema-like description.
pub fn value_type_to_json(vt: &DynValueType) -> serde_json::Value {
    match vt {
        DynValueType::U8 => serde_json::json!({"type": "u8"}),
        DynValueType::U16 => serde_json::json!({"type": "u16"}),
        DynValueType::U32 => serde_json::json!({"type": "u32"}),
        DynValueType::U64 => serde_json::json!({"type": "u64"}),
        DynValueType::I8 => serde_json::json!({"type": "i8"}),
        DynValueType::I16 => serde_json::json!({"type": "i16"}),
        DynValueType::I32 => serde_json::json!({"type": "i32"}),
        DynValueType::I64 => serde_json::json!({"type": "i64"}),
        DynValueType::F32 => serde_json::json!({"type": "f32"}),
        DynValueType::F64 => serde_json::json!({"type": "f64"}),
        DynValueType::Bool => serde_json::json!({"type": "bool"}),
        DynValueType::Usize => serde_json::json!({"type": "usize"}),
        DynValueType::Isize => serde_json::json!({"type": "isize"}),
        DynValueType::Str(inline) => {
            if let Some(n) = inline {
                serde_json::json!({"type": "string", "inline": n})
            } else {
                serde_json::json!({"type": "string"})
            }
        }
        DynValueType::PodStruct { size } => {
            serde_json::json!({"type": "pod_struct", "size": size})
        }
        DynValueType::Struct(_) => struct_schema_to_json(vt),
        DynValueType::Array(arr_schema) => {
            serde_json::json!({
                "type": "array",
                "elem": value_type_to_json(arr_schema.elem_type),
                "layout": array_layout_to_json(&arr_schema.layout),
            })
        }
        DynValueType::Option(inner) => {
            serde_json::json!({
                "type": "option",
                "inner": value_type_to_json(inner),
            })
        }
        DynValueType::ByRef(inner) => {
            serde_json::json!({
                "type": "byref",
                "inner": value_type_to_json(inner),
            })
        }
        DynValueType::HashMap(schema) => {
            serde_json::json!({
                "type": "hashmap",
                "key_type": value_type_to_json(schema.key_type),
                "value_type": value_type_to_json(schema.value_type),
            })
        }
        DynValueType::Lazy(resolve) => value_type_to_json(resolve()),
        DynValueType::Enum(es) => {
            let variants: Vec<serde_json::Value> =
                es.variants.iter().map(|vs| fields_to_json(vs)).collect();
            serde_json::json!({
                "type": "enum",
                "name": es.name,
                "variants": variants,
            })
        }
    }
}

fn struct_schema_to_json(vt: &DynValueType) -> serde_json::Value {
    match vt {
        DynValueType::Struct(resolve) => {
            let s = resolve();
            fields_to_json(s)
        }
        _ => serde_json::Value::Null,
    }
}

fn fields_to_json(s: &StructSchema) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("name".into(), serde_json::json!(s.name));
    let fields: Vec<serde_json::Value> = s
        .fields
        .iter()
        .map(|fd| {
            serde_json::json!({
                "name": fd.name,
                "type": fd.value_type.name_hint(),
                "inline_dyn_index": fd.inline_dyn_index,
                "offset_in_fixed": fd.offset_in_fixed,
            })
        })
        .collect();
    map.insert("fields".into(), serde_json::Value::Array(fields));
    serde_json::Value::Object(map)
}

fn array_layout_to_json(layout: &ArrayLayout) -> serde_json::Value {
    match layout {
        ArrayLayout::Pod { stride } => serde_json::json!({"kind": "pod", "stride": stride}),
        ArrayLayout::FixedPod { stride, count } => {
            serde_json::json!({"kind": "fixedpod", "stride": stride, "count": count})
        }
        ArrayLayout::Indexed => serde_json::json!({"kind": "indexed"}),
        ArrayLayout::Fixed { stride } => serde_json::json!({"kind": "fixed", "stride": stride}),
    }
}

/// Name hint for user-friendly schema output (strips qualifiers).
trait NameHint {
    fn name_hint(&self) -> &'static str;
}

impl NameHint for DynValueType {
    fn name_hint(&self) -> &'static str {
        match self {
            DynValueType::U8 => "u8",
            DynValueType::U16 => "u16",
            DynValueType::U32 => "u32",
            DynValueType::U64 => "u64",
            DynValueType::I8 => "i8",
            DynValueType::I16 => "i16",
            DynValueType::I32 => "i32",
            DynValueType::I64 => "i64",
            DynValueType::F32 => "f32",
            DynValueType::F64 => "f64",
            DynValueType::Bool => "bool",
            DynValueType::Usize => "usize",
            DynValueType::Isize => "isize",
            DynValueType::Str(_) => "string",
            DynValueType::PodStruct { .. } => "pod",
            DynValueType::Struct(resolve) => resolve().name,
            DynValueType::Array(_arr) => "array",
            DynValueType::Option(_inner) => "option",
            DynValueType::ByRef(inner) => inner.name_hint(),
            DynValueType::HashMap(_) => "hashmap",
            DynValueType::Enum(es) => es.name,
            DynValueType::Lazy(resolve) => resolve().name_hint(),
        }
    }
}

/// Compute stats for the root struct: read each top-level field and describe it.
pub fn compute_stats<'a>(ctx: &SchemaCtx<'a>, root: &Value) -> serde_json::Value {
    let schema = ctx.get_struct_schema(root);
    let mut top_level = serde_json::Map::new();

    for fd in schema.fields {
        let fv = ctx.field_get(root, fd.name);
        let mut desc = serde_json::Map::new();

        match fd.value_type {
            DynValueType::Option(inner) => {
                desc.insert("type".into(), serde_json::json!(inner.name_hint()));
                // Check presence: Option layout = [present: u8, data...]
                let present = fv
                    .as_ref()
                    .map(|v| {
                        let off = v.offset as usize;
                        ctx.data().get(off).copied().unwrap_or(0) != 0
                    })
                    .unwrap_or(false);
                desc.insert("present".into(), serde_json::json!(present));
            }
            DynValueType::Array(_) => {
                desc.insert("type".into(), serde_json::json!("array"));
                if let Some(ref val) = fv {
                    let count = ctx.array_len(val);
                    desc.insert("count".into(), serde_json::json!(count));
                }
            }
            DynValueType::Enum(es) => {
                desc.insert("type".into(), serde_json::json!("enum"));
                desc.insert("name".into(), serde_json::json!(es.name));
                if let Some(ref val) = fv {
                    if let Some((vs, _)) = ctx.enum_variant(val) {
                        desc.insert("variant".into(), serde_json::json!(vs.name));
                    }
                }
            }
            _ => {
                desc.insert("type".into(), serde_json::json!(fd.value_type.name_hint()));
            }
        }

        top_level.insert(fd.name.to_string(), serde_json::Value::Object(desc));
    }

    serde_json::json!({
        "top_level": top_level,
        "root_type": schema.name,
    })
}
