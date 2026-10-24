use mmapstruct_schema::context::{FieldInfo, RuntimeSchemaCtx};
use mmapstruct_schema::schema::{ArrayLayoutDefReader, PrimitiveDef, TypeDefReader};
use mmapstruct_schema::SchemaValue;

/// Trailing suffix appended when fields don't fit: `, ... }`
const TRUNC_SUFFIX: &str = ", ... }";

pub fn type_annotation(t: &TypeDefReader) -> &'static str {
    match t {
        TypeDefReader::Primitive(p) => match p {
            PrimitiveDef::U32 => ": u32",
            PrimitiveDef::I32 => ": i32",
            PrimitiveDef::U64 => ": u64",
            PrimitiveDef::I64 => ": i64",
            PrimitiveDef::F32 => ": f32",
            PrimitiveDef::F64 => ": f64",
            PrimitiveDef::Bool => ": bool",
            PrimitiveDef::U16 => ": u16",
            PrimitiveDef::I16 => ": i16",
            PrimitiveDef::U8 => ": u8",
            PrimitiveDef::I8 => ": i8",
            PrimitiveDef::Usize => ": usize",
            PrimitiveDef::Isize => ": isize",
        },
        TypeDefReader::Str { .. } => ": str",
        TypeDefReader::PodStruct { .. } => ": [pod]",
        TypeDefReader::Struct { .. } => "",
        TypeDefReader::Enum { .. } => "",
        TypeDefReader::Array { .. } => "",
        TypeDefReader::Option { .. } => "",
        TypeDefReader::ByRef { .. } => ": byref",
        TypeDefReader::HashMap { .. } => "",
    }
}

/// Generate a compact inline preview of `val`'s children that fits within
/// `char_limit`.  Returns `None` when the value type does not support
/// previews (scalars, arrays, maps, etc.) or when not even the opening
/// `Type { ` would fit.
pub fn preview_text(
    ctx: &RuntimeSchemaCtx,
    val: &SchemaValue,
    char_limit: usize,
) -> Option<String> {
    if char_limit < 4 {
        return None;
    }

    let t = ctx.type_reader(val.type_idx);
    match t {
        TypeDefReader::Struct { name, fields, .. } => {
            build_preview(ctx, val, name, fields.len(), |i| {
                let f = fields.get(i)?;
                let FieldInfo { val: child, is_byref } = ctx.field_get_with_info(val, f.name)?;
                Some((f.name.to_string(), child, is_byref))
            }, char_limit)
        }
        TypeDefReader::Enum { .. } => {
            let (variant_type_idx, data_off) = ctx.enum_variant(val)?;
            let vt = ctx.type_reader(variant_type_idx);
            if let TypeDefReader::Struct { name, fields, .. } = vt {
                let shifted = SchemaValue {
                    mmap_begin: val.mmap_begin + data_off,
                    mmap_end: val.mmap_end,
                    type_idx: variant_type_idx,
                };
                build_preview(ctx, &shifted, name, fields.len(), |i| {
                    let f = fields.get(i)?;
                    let FieldInfo { val: child, is_byref } = ctx.field_get_with_info(&shifted, f.name)?;
                    Some((f.name.to_string(), child, is_byref))
                }, char_limit)
            } else {
                None
            }
        }
        TypeDefReader::ByRef { inner_type } => {
            let inner = SchemaValue::new(val.mmap_begin, val.mmap_end, **inner_type);
            preview_text(ctx, &inner, char_limit).map(|s| format!("&{}", s))
        }
        _ => None,
    }
}

fn build_preview<F>(
    ctx: &RuntimeSchemaCtx,
    _val: &SchemaValue,
    type_name: &str,
    field_count: usize,
    mut field_at: F,
    char_limit: usize,
) -> Option<String>
where
    F: FnMut(usize) -> Option<(String, SchemaValue, bool)>,
{
    let mut out = format!("{} {{ ", type_name);
    if out.len() > char_limit {
        return None;
    }
    let mut remaining = char_limit - out.len();
    let mut first = true;

    for i in 0..field_count {
        let (field_name, child, is_byref) = field_at(i)?;
        let value_text = if is_byref {
            format!("&{}", inline_value_text(ctx, &child))
        } else {
            inline_value_text(ctx, &child)
        };
        let field_str = if first {
            format!("{}: {}", field_name, value_text)
        } else {
            format!(", {}: {}", field_name, value_text)
        };

        let overhead = if i + 1 < field_count { TRUNC_SUFFIX.len() } else { 2 };

        if field_str.len() + overhead <= remaining {
            out.push_str(&field_str);
            remaining -= field_str.len();
            first = false;
        } else if first {
            return None;
        } else {
            out.push_str(TRUNC_SUFFIX);
            return Some(out);
        }
    }

    out.push_str(" }");
    Some(out)
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

pub fn inline_value_text(ctx: &RuntimeSchemaCtx, val: &SchemaValue) -> String {
    let t = ctx.type_reader(val.type_idx);
    match t {
        TypeDefReader::Primitive(p) => match p {
            PrimitiveDef::U32 => ctx.read_u32(val).to_string(),
            PrimitiveDef::I32 => ctx.read_i32(val).to_string(),
            PrimitiveDef::U64 => ctx.read_u64(val).to_string(),
            PrimitiveDef::I64 => ctx.read_i64(val).to_string(),
            PrimitiveDef::F32 => ctx.read_f32(val).to_string(),
            PrimitiveDef::F64 => ctx.read_f64(val).to_string(),
            PrimitiveDef::Bool => {
                if ctx.read_bool(val) { "true".to_string() } else { "false".to_string() }
            }
            PrimitiveDef::U16 => ctx.read_u16(val).to_string(),
            PrimitiveDef::I16 => ctx.read_i16(val).to_string(),
            PrimitiveDef::U8 => ctx.read_u8(val).to_string(),
            PrimitiveDef::I8 => ctx.read_i8(val).to_string(),
            PrimitiveDef::Usize => ctx.read_usize(val).to_string(),
            PrimitiveDef::Isize => ctx.read_isize(val).to_string(),
        },
        TypeDefReader::Str { .. } => {
            let s = if val.mmap_end as usize <= ctx.data().len() {
                ctx.read_str(val).to_string()
            } else {
                "<bad str>".to_string()
            };
            if s.len() > 80 {
                format!("\"{}...\"", &s[..77])
            } else {
                format!("\"{}\"", s)
            }
        }
        TypeDefReader::Struct { name, .. } => format!("{} {{ ... }}", name),
        TypeDefReader::Enum { .. } => {
            if let Some((variant_type_idx, _)) = ctx.enum_variant(val) {
                let vt = ctx.type_reader(variant_type_idx);
                if let TypeDefReader::Struct { name, .. } = vt {
                    name.to_string()
                } else {
                    "<enum>".to_string()
                }
            } else {
                "<enum>".to_string()
            }
        }
        TypeDefReader::Array { elem_type, layout } => {
            if val.mmap_end as usize <= ctx.data().len() {
                let len = ctx.array_len(val);
                let layout_label = match layout {
                    ArrayLayoutDefReader::Pod { .. } => "pod",
                    ArrayLayoutDefReader::FixedPod { .. } => "fixedpod",
                    ArrayLayoutDefReader::Indexed => "indexed",
                    ArrayLayoutDefReader::Fixed { .. } => "fixed",
                };
                let elem_ann = type_annotation(ctx.type_reader(**elem_type));
                let mut s = format!("[{} {} (#{}{})]", len, layout_label, val.type_idx, elem_ann);
                // Pod arrays with ≤ 5 elements show inline values
                if len <= 5 {
                    if let ArrayLayoutDefReader::Pod { .. } | ArrayLayoutDefReader::FixedPod { .. } = layout {
                        s.push_str(" = [");
                        for i in 0..len {
                            if i > 0 { s.push_str(", "); }
                            if let Some(elem) = ctx.array_get(val, i) {
                                s.push_str(&inline_value_text(ctx, &elem));
                            }
                        }
                        s.push(']');
                    }
                }
                s
            } else {
                "[?]".to_string()
            }
        }
        TypeDefReader::Option { inner_type } => {
            if val.mmap_begin == val.mmap_end {
                "None".to_string()
            } else {
                let inner = SchemaValue::new(val.mmap_begin, val.mmap_end, **inner_type);
                format!("Some({})", inline_value_text(ctx, &inner))
            }
        }
        TypeDefReader::ByRef { inner_type } => {
            let inner = SchemaValue::new(val.mmap_begin, val.mmap_end, **inner_type);
            format!("&{}", inline_value_text(ctx, &inner))
        }
        TypeDefReader::HashMap { .. } => {
            if val.mmap_end as usize <= ctx.data().len() {
                format!("[{} entries (#{})]", ctx.hashmap_len(val), val.type_idx)
            } else {
                "[?]".to_string()
            }
        }
        TypeDefReader::PodStruct { .. } => "[pod]".to_string(),
    }
}
