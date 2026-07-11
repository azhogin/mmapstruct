//! Formatting schema values for debugging / tree view.

use crate::context::RuntimeSchemaCtx;
use crate::value::SchemaValue;
use crate::schema::{FieldDef, PrimitiveDef, TypeDefReader};
use mmapstruct::indexed_array::IndexedArrayReader;

/// Format a schema value as a human-readable string (for debugging / tree view).
pub fn format_schema_value(ctx: &RuntimeSchemaCtx, val: &SchemaValue) -> String {
    let mut out = String::new();
    write_value(ctx, val, 0, &mut out);
    out
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_value(ctx: &RuntimeSchemaCtx, val: &SchemaValue, depth: usize, out: &mut String) {
    let t = ctx.type_reader(val.type_idx);
    write_value_inner(ctx, val, depth, out, t)
}

fn write_struct_fields(
    ctx: &RuntimeSchemaCtx,
    root: &SchemaValue,
    fields: &IndexedArrayReader<'_, FieldDef>,
    depth: usize,
    out: &mut String,
) {
    let field_count = fields.len();
    for i in 0..field_count {
        let field = fields.get(i).unwrap();
        let is_last = i + 1 == field_count;
        indent(out, depth + 1);
        out.push('"');
        out.push_str(field.name);
        out.push_str("\": ");
        let fv = ctx.field_get(root, field.name);
        if let Some(fv) = fv {
            write_value(ctx, &fv, depth + 1, out);
        } else {
            out.push_str("null");
        }
        if !is_last {
            out.push(',');
        }
        out.push('\n');
    }
}

fn write_value_inner(
    ctx: &RuntimeSchemaCtx,
    val: &SchemaValue,
    depth: usize,
    out: &mut String,
    t: &TypeDefReader,
) {
    match t {
        TypeDefReader::Primitive(p) => match p {
            PrimitiveDef::U32 => out.push_str(&ctx.read_u32(val).to_string()),
            PrimitiveDef::I32 => out.push_str(&ctx.read_i32(val).to_string()),
            PrimitiveDef::U64 => out.push_str(&ctx.read_u64(val).to_string()),
            PrimitiveDef::I64 => out.push_str(&ctx.read_i64(val).to_string()),
            PrimitiveDef::F32 => out.push_str(&ctx.read_f32(val).to_string()),
            PrimitiveDef::F64 => out.push_str(&ctx.read_f64(val).to_string()),
            PrimitiveDef::Bool => {
                out.push_str(if ctx.read_bool(val) { "true" } else { "false" });
            }
            PrimitiveDef::U16 => out.push_str(&ctx.read_u16(val).to_string()),
            PrimitiveDef::I16 => out.push_str(&ctx.read_i16(val).to_string()),
            PrimitiveDef::U8 => out.push_str(&ctx.read_u8(val).to_string()),
            PrimitiveDef::I8 => out.push_str(&ctx.read_i8(val).to_string()),
            PrimitiveDef::Usize => out.push_str(&ctx.read_usize(val).to_string()),
            PrimitiveDef::Isize => out.push_str(&ctx.read_isize(val).to_string()),
        },
        TypeDefReader::Str { .. } => {
            out.push('"');
            out.push_str(ctx.read_str(val));
            out.push('"');
        }
        TypeDefReader::PodStruct { .. } => {
            out.push_str("[pod]");
        }
        TypeDefReader::Struct {
            name: _,
            fields,
            ..
        } => {
            out.push_str("{\n");
            write_struct_fields(ctx, val, fields, depth, out);
            indent(out, depth);
            out.push('}');
        }
        TypeDefReader::Array { .. } => {
            let count = ctx.array_len(val);
            out.push_str("[\n");
            for i in 0..count {
                let is_last = i + 1 == count;
                if let Some(elem) = ctx.array_get(val, i) {
                    indent(out, depth + 1);
                    write_value(ctx, &elem, depth + 1, out);
                    if !is_last {
                        out.push(',');
                    }
                    out.push('\n');
                }
            }
            indent(out, depth);
            out.push(']');
        }
        TypeDefReader::Option { inner_type: _ } => {
            // Convention from resolve_field_value: None → offset == limit
            if val.mmap_begin < val.mmap_end {
                let inner = ctx.unwrap_type(val.type_idx);
                let inner_val = SchemaValue {
                    mmap_begin: val.mmap_begin,
                    mmap_end: val.mmap_end,
                    type_idx: inner,
                };
                out.push_str("Some(");
                write_value(ctx, &inner_val, depth, out);
                out.push(')');
            } else {
                out.push_str("None");
            }
        }
        TypeDefReader::ByRef { inner_type } => {
            let inner_val = SchemaValue {
                mmap_begin: val.mmap_begin,
                mmap_end: val.mmap_end,
                type_idx: **inner_type,
            };
            write_value(ctx, &inner_val, depth, out);
        }
        TypeDefReader::Enum { .. } => {
            if let Some((variant_type_idx, _)) = ctx.enum_variant(val) {
                let variant_t = ctx.type_reader(variant_type_idx);
                if let TypeDefReader::Struct {
                    name: variant_name,
                    fields,
                    ..
                } = variant_t
                {
                    out.push_str(variant_name);
                    out.push_str(" {\n");
                    write_struct_fields(ctx, val, fields, depth, out);
                    indent(out, depth);
                    out.push('}');
                }
            }
        }
        TypeDefReader::HashMap { .. } => {
            let count = ctx.hashmap_len(val);
            out.push_str(&format!("{{{} entries", count));
            for i in 0..count {
                out.push('\n');
                indent(out, depth + 1);
                out.push_str(&format!("[{}]: ", i));
                if let Some((key, val_entry)) = ctx.hashmap_get_index(val, i) {
                    out.push_str("key = ");
                    write_value(ctx, &key, depth + 1, out);
                    out.push_str(", value = ");
                    write_value(ctx, &val_entry, depth + 1, out);
                } else {
                    out.push_str("<error>");
                }
            }
            out.push('\n');
            indent(out, depth);
            out.push('}');
        }
    }
}
