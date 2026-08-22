//! JSONL rendering of [`SchemaValue`]s via [`RuntimeSchemaCtx`].
//!
//! Designed for AI-facing CLI output: nested containers are summarized by
//! default (`{"$len", "$elem", "$preview"}`), strings are truncated, and a
//! byte budget stops emission before flooding the consumer. `--full`
//! disables summarization (budget still applies).

use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::{ArrayLayoutDefReader, PrimitiveDef, TypeDefReader};
use mmapstruct_schema::SchemaValue;
use serde_json::{json, Map, Value as Json};

/// Rendering options (from CLI flags).
#[derive(Clone, Copy, Debug)]
pub struct RenderOpts {
    /// Expand containers fully instead of summarizing.
    pub full: bool,
    /// Truncate strings longer than this many chars.
    pub max_string: usize,
    /// Max expansion depth for nested structures.
    pub depth_cap: usize,
}

impl Default for RenderOpts {
    fn default() -> Self {
        Self {
            full: false,
            max_string: 120,
            depth_cap: 8,
        }
    }
}

/// Byte-budget tracking across emitted lines.
#[derive(Default)]
pub struct OutState {
    /// Serialized bytes written so far.
    pub bytes: usize,
    /// Set when the budget would be exceeded by the next line.
    pub truncated: bool,
}

impl OutState {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            bytes: 0,
            truncated: max_bytes == 0 && false,
            // max_bytes == 0 means unlimited; nothing else needed here —
            // the budget check lives in `fits`.
        }
    }

    /// Returns `true` when a serialized chunk of `len` bytes fits the budget
    /// (`max_bytes == 0` = unlimited). Records the chunk when it fits.
    pub fn fits(&mut self, len: usize, max_bytes: usize) -> bool {
        if max_bytes == 0 || self.bytes + len <= max_bytes {
            self.bytes += len;
            true
        } else {
            self.truncated = true;
            false
        }
    }
}

/// Render an aggregate/scalar result as a JSON value.
pub fn scalar_to_json<V: Copy>(r: &crate::EvalResult<V>) -> Json {
    match r {
        crate::EvalResult::Buf(_) => Json::Null, // handled by value_to_json
        crate::EvalResult::U64(n) => json!(n),
        crate::EvalResult::I64(n) => json!(n),
        crate::EvalResult::F64(n) => json!(n),
    }
}

/// Render a buffer-backed value as JSON, summarizing large containers.
pub fn value_to_json(
    ctx: &RuntimeSchemaCtx<'_>,
    val: &SchemaValue,
    opts: &RenderOpts,
) -> Json {
    let mut depth = 0usize;
    value_to_json_inner(ctx, val, opts, &mut depth)
}

fn value_to_json_inner(
    ctx: &RuntimeSchemaCtx<'_>,
    val: &SchemaValue,
    opts: &RenderOpts,
    depth: &mut usize,
) -> Json {
    // Lazy is resolved eagerly at schema-serialization time, so the runtime
    // type table never contains Lazy entries.
    let t = ctx.type_reader(val.type_idx);
    match t {
        TypeDefReader::Primitive(p) => primitive_json(ctx, val, p),
        TypeDefReader::Str { .. } => Json::String(truncate_str(ctx.read_str(val), opts.max_string)),
        TypeDefReader::PodStruct { size } => json!({ "$pod": { "size": **size } }),
        TypeDefReader::Enum { .. } => enum_json(ctx, val, opts, depth),
        TypeDefReader::Struct { .. } => struct_json(ctx, val, opts, depth),
        TypeDefReader::Array { .. } => array_json(ctx, val, opts, depth),
        TypeDefReader::HashMap { .. } => hashmap_json(ctx, val, opts, depth),
        TypeDefReader::Option { .. } => {
            if val.mmap_begin < val.mmap_end {
                let inner = ctx.unwrap_type(val.type_idx);
                let inner_val = SchemaValue::new(val.mmap_begin, val.mmap_end, inner);
                value_to_json_inner(ctx, &inner_val, opts, depth)
            } else {
                Json::Null
            }
        }
        TypeDefReader::ByRef { .. } => {
            let inner = ctx.unwrap_type(val.type_idx);
            let inner_val = SchemaValue::new(val.mmap_begin, val.mmap_end, inner);
            value_to_json_inner(ctx, &inner_val, opts, depth)
        }
    }
}

fn primitive_json(ctx: &RuntimeSchemaCtx<'_>, val: &SchemaValue, p: &PrimitiveDef) -> Json {
    match p {
        PrimitiveDef::U8 => json!(ctx.read_u8(val)),
        PrimitiveDef::U16 => json!(ctx.read_u16(val)),
        PrimitiveDef::U32 => json!(ctx.read_u32(val)),
        PrimitiveDef::U64 => json!(ctx.read_u64(val)),
        PrimitiveDef::I8 => json!(ctx.read_i8(val)),
        PrimitiveDef::I16 => json!(ctx.read_i16(val)),
        PrimitiveDef::I32 => json!(ctx.read_i32(val)),
        PrimitiveDef::I64 => json!(ctx.read_i64(val)),
        PrimitiveDef::F32 => json!(ctx.read_f32(val)),
        PrimitiveDef::F64 => json!(ctx.read_f64(val)),
        PrimitiveDef::Bool => json!(ctx.read_bool(val)),
        PrimitiveDef::Usize => json!(ctx.read_usize(val)),
        PrimitiveDef::Isize => json!(ctx.read_isize(val)),
    }
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max_chars).collect();
        format!("{cut}…")
    }
}

fn struct_json(
    ctx: &RuntimeSchemaCtx<'_>,
    val: &SchemaValue,
    opts: &RenderOpts,
    depth: &mut usize,
) -> Json {
    let name = match ctx.type_reader(val.type_idx) {
        TypeDefReader::Struct { name, .. } => (**name).to_string(),
        _ => String::new(),
    };
    if !opts.full && *depth >= opts.depth_cap {
        return json!({ "$type": name });
    }
    *depth += 1;
    let mut obj = Map::new();
    if let TypeDefReader::Struct { fields, .. } = ctx.type_reader(val.type_idx) {
        for i in 0..fields.len() {
            let fd = fields.get(i).unwrap();
            let fname: &str = &fd.name;
            match ctx.field_get(val, fname) {
                Some(fv) => {
                    obj.insert(fname.to_string(), value_to_json_inner(ctx, &fv, opts, depth));
                }
                None => {
                    obj.insert(fname.to_string(), Json::Null);
                }
            }
        }
    }
    *depth -= 1;
    Json::Object(obj)
}

fn enum_json(
    ctx: &RuntimeSchemaCtx<'_>,
    val: &SchemaValue,
    opts: &RenderOpts,
    depth: &mut usize,
) -> Json {
    let Some((variant_idx, data_off)) = ctx.enum_variant(val) else {
        return Json::Null;
    };
    let vname = match ctx.type_reader(variant_idx) {
        TypeDefReader::Struct { name, fields, .. } => {
            let n = (**name).to_string();
            if fields.is_empty() {
                return Json::String(n);
            }
            n
        }
        _ => return Json::Null,
    };
    if !opts.full && *depth >= opts.depth_cap {
        return json!({ "$variant": vname });
    }
    *depth += 1;
    let shifted = SchemaValue::new(val.mmap_begin + data_off, val.mmap_end, variant_idx);
    let mut obj = Map::new();
    obj.insert("$variant".into(), json!(vname));
    if let TypeDefReader::Struct { fields, .. } = ctx.type_reader(variant_idx) {
        for i in 0..fields.len() {
            let fd = fields.get(i).unwrap();
            let fname: &str = &fd.name;
            match ctx.field_get(&shifted, fname) {
                Some(fv) => {
                    obj.insert(fname.to_string(), value_to_json_inner(ctx, &fv, opts, depth));
                }
                None => {
                    obj.insert(fname.to_string(), Json::Null);
                }
            }
        }
    }
    *depth -= 1;
    Json::Object(obj)
}

fn array_json(
    ctx: &RuntimeSchemaCtx<'_>,
    val: &SchemaValue,
    opts: &RenderOpts,
    depth: &mut usize,
) -> Json {
    let len = ctx.array_len(val);
    let (layout_tag, elem_type_idx, elem_is_scalar) = match ctx.type_reader(val.type_idx) {
        TypeDefReader::Array { elem_type, layout } => {
            let scalar = matches!(
                ctx.type_reader(**elem_type),
                TypeDefReader::Primitive(_) | TypeDefReader::Str { .. } | TypeDefReader::PodStruct { .. }
            );
            let tag = match layout {
                ArrayLayoutDefReader::Pod { .. } => "pod",
                ArrayLayoutDefReader::FixedPod { .. } => "fixedpod",
                ArrayLayoutDefReader::Indexed => "indexed",
                ArrayLayoutDefReader::Fixed { .. } => "fixed",
            };
            (tag, **elem_type, scalar)
        }
        _ => ("?", val.type_idx, false),
    };

    let summarize = !opts.full && (len > 8 || !elem_is_scalar);
    if summarize {
        let preview_len = len.min(3);
        let mut preview = Vec::with_capacity(preview_len as usize);
        *depth += 1;
        for i in 0..preview_len {
            match ctx.array_get(val, i as u64) {
                Some(ev) => preview.push(value_to_json_inner(ctx, &ev, opts, depth)),
                None => preview.push(Json::Null),
            }
        }
        *depth -= 1;
        let elem_ty = type_name(ctx, elem_type_idx);
        return json!({
            "$array": {
                "len": len,
                "elem": elem_ty,
                "layout": layout_tag,
                "preview": preview,
                "hint": format!("slice with [i:j] to fetch more"),
            }
        });
    }

    let mut arr = Vec::with_capacity(len.min(64) as usize);
    *depth += 1;
    for i in 0..len {
        match ctx.array_get(val, i as u64) {
            Some(ev) => arr.push(value_to_json_inner(ctx, &ev, opts, depth)),
            None => arr.push(Json::Null),
        }
    }
    *depth -= 1;
    Json::Array(arr)
}

fn hashmap_json(
    ctx: &RuntimeSchemaCtx<'_>,
    val: &SchemaValue,
    opts: &RenderOpts,
    depth: &mut usize,
) -> Json {
    let entries = ctx.hashmap_len(val);
    if !opts.full && entries > 4 {
        let mut preview = Vec::new();
        *depth += 1;
        for i in 0..entries.min(2) {
            if let Some((k, v)) = ctx.hashmap_get_index(val, i) {
                preview.push(json!([
                    value_to_json_inner(ctx, &k, opts, depth),
                    value_to_json_inner(ctx, &v, opts, depth)
                ]));
            }
        }
        *depth -= 1;
        return json!({ "$map": { "entries": entries, "preview": preview,
            "hint": "raise --full or slice via keys" } });
    }
    let mut arr = Vec::with_capacity(entries);
    *depth += 1;
    for i in 0..entries {
        if let Some((k, v)) = ctx.hashmap_get_index(val, i) {
            arr.push(json!([
                value_to_json_inner(ctx, &k, opts, depth),
                value_to_json_inner(ctx, &v, opts, depth)
            ]));
        }
    }
    *depth -= 1;
    Json::Array(arr)
}

/// Compact human/agent-readable type name for a type index.
pub fn type_name(ctx: &RuntimeSchemaCtx<'_>, type_idx: u32) -> String {
    match ctx.type_reader(type_idx) {
        TypeDefReader::Primitive(p) => match p {
            PrimitiveDef::U8 => "u8".into(),
            PrimitiveDef::U16 => "u16".into(),
            PrimitiveDef::U32 => "u32".into(),
            PrimitiveDef::U64 => "u64".into(),
            PrimitiveDef::I8 => "i8".into(),
            PrimitiveDef::I16 => "i16".into(),
            PrimitiveDef::I32 => "i32".into(),
            PrimitiveDef::I64 => "i64".into(),
            PrimitiveDef::F32 => "f32".into(),
            PrimitiveDef::F64 => "f64".into(),
            PrimitiveDef::Bool => "bool".into(),
            PrimitiveDef::Usize => "usize".into(),
            PrimitiveDef::Isize => "isize".into(),
        },
        TypeDefReader::Str { size } => match size {
            Some(n) => format!("str[{n}]"),
            None => "str".into(),
        },
        TypeDefReader::PodStruct { size } => format!("[pod {}B]", **size),
        TypeDefReader::Struct { name, .. } => (**name).to_string(),
        TypeDefReader::Enum { name, .. } => (**name).to_string(),
        TypeDefReader::Array { elem_type, layout } => {
            let inner = type_name(ctx, **elem_type);
            match layout {
                ArrayLayoutDefReader::Pod { .. } => format!("[{inner}]"),
                ArrayLayoutDefReader::FixedPod { count, .. } => {
                    format!("[{inner}; {}]", **count)
                }
                ArrayLayoutDefReader::Indexed => format!("vec<{inner}>"),
                ArrayLayoutDefReader::Fixed { .. } => format!("fixed<{inner}>"),
            }
        }
        TypeDefReader::Option { inner_type } => format!("Option<{}>", type_name(ctx, **inner_type)),
        TypeDefReader::ByRef { inner_type } => format!("&{}", type_name(ctx, **inner_type)),
        TypeDefReader::HashMap { key_type, val_type, .. } => {
            format!("map<{}, {}>", type_name(ctx, **key_type), type_name(ctx, **val_type))
        }
    }
}
