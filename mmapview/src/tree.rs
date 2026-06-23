//! Tree-formatted viewer — renders a mmapstruct file as an indented tree.
//!
//! Designed for use as an F3 external viewer in Total Commander / Midnight Commander.

/// Options for tree formatting.
#[derive(Clone)]
pub struct TreeOptions {
    pub depth: usize,
    pub max_array: usize,
    pub max_string: usize,
    pub no_values: bool,
}

impl Default for TreeOptions {
    fn default() -> Self {
        Self { depth: 4, max_array: 20, max_string: 80, no_values: false }
    }
}

// Re-export shared schema types for backward compatibility
pub use crate::schema::SchemaResult as SchemaLoadResult;
pub use crate::schema::resolve_schema as load_schema_for_data;

/// Render the entire file as a tree string.
pub fn format_file(data: &[u8], opts: &TreeOptions) -> Result<String, String> {
    let loaded = load_schema_for_data(data)?;
    let ctx =
        mmapstruct_schema::context::RuntimeSchemaCtx::new(data, &loaded.schema_bytes);
    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        &loaded.schema_bytes,
        0..loaded.schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    use mmapstruct_schema::SchemaValue;
    let root = SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );

    let type_info = ctx.type_reader(root_type);
    let root_name = match type_info {
        mmapstruct_schema::TypeDefReader::Struct { name, .. } => name.to_string(),
        mmapstruct_schema::TypeDefReader::Enum { name, .. } => name.to_string(),
        _ => "Root".to_string(),
    };

    let mut out = String::new();
    out.push_str(&root_name);
    out.push('\n');

    write_value_tree(&ctx, &root, &mut out, 0, opts);

    Ok(out)
}

fn write_type_annotation(ft: &mmapstruct_schema::TypeDefReader<'_>, out: &mut String) {
    match ft {
        mmapstruct_schema::TypeDefReader::Primitive(p) => match p {
            mmapstruct_schema::PrimitiveDef::U32 => out.push_str(": u32"),
            mmapstruct_schema::PrimitiveDef::I32 => out.push_str(": i32"),
            mmapstruct_schema::PrimitiveDef::U64 => out.push_str(": u64"),
            mmapstruct_schema::PrimitiveDef::I64 => out.push_str(": i64"),
            mmapstruct_schema::PrimitiveDef::F32 => out.push_str(": f32"),
            mmapstruct_schema::PrimitiveDef::F64 => out.push_str(": f64"),
            mmapstruct_schema::PrimitiveDef::Bool => out.push_str(": bool"),
            mmapstruct_schema::PrimitiveDef::U16 => out.push_str(": u16"),
            mmapstruct_schema::PrimitiveDef::I16 => out.push_str(": i16"),
            mmapstruct_schema::PrimitiveDef::U8 => out.push_str(": u8"),
            mmapstruct_schema::PrimitiveDef::I8 => out.push_str(": i8"),
            mmapstruct_schema::PrimitiveDef::Usize => out.push_str(": usize"),
            mmapstruct_schema::PrimitiveDef::Isize => out.push_str(": isize"),
        },
        mmapstruct_schema::TypeDefReader::Str { .. } => out.push_str(": str"),
        mmapstruct_schema::TypeDefReader::PodStruct { .. } => out.push_str(": [pod]"),
        mmapstruct_schema::TypeDefReader::Struct { name, .. } => {
            out.push_str(&format!(": {}", name));
        }
        mmapstruct_schema::TypeDefReader::Enum { name, .. } => {
            out.push_str(&format!(": {}", name));
        }
        mmapstruct_schema::TypeDefReader::Array { .. } => out.push_str(": Array"),
        mmapstruct_schema::TypeDefReader::Option { .. } => out.push_str(": Option"),
        mmapstruct_schema::TypeDefReader::ByRef { .. } => out.push_str(": byref"),
        mmapstruct_schema::TypeDefReader::HashMap { .. } => out.push_str(": HashMap"),
    }
}

fn write_value_inline(
    ctx: &mmapstruct_schema::context::RuntimeSchemaCtx<'_>,
    val: &mmapstruct_schema::SchemaValue,
    out: &mut String,
    opts: &TreeOptions,
) {
    let t = ctx.type_reader(val.type_idx);
    match t {
        mmapstruct_schema::TypeDefReader::Primitive(p) => match p {
            mmapstruct_schema::PrimitiveDef::U32 => out.push_str(&ctx.read_u32(val).to_string()),
            mmapstruct_schema::PrimitiveDef::I32 => out.push_str(&ctx.read_i32(val).to_string()),
            mmapstruct_schema::PrimitiveDef::U64 => out.push_str(&ctx.read_u64(val).to_string()),
            mmapstruct_schema::PrimitiveDef::I64 => out.push_str(&ctx.read_i64(val).to_string()),
            mmapstruct_schema::PrimitiveDef::F32 => out.push_str(&ctx.read_f32(val).to_string()),
            mmapstruct_schema::PrimitiveDef::F64 => out.push_str(&ctx.read_f64(val).to_string()),
            mmapstruct_schema::PrimitiveDef::Bool => {
                out.push_str(if ctx.read_bool(val) { "true" } else { "false" })
            }
            mmapstruct_schema::PrimitiveDef::U16 => out.push_str(&ctx.read_u16(val).to_string()),
            mmapstruct_schema::PrimitiveDef::I16 => out.push_str(&ctx.read_i16(val).to_string()),
            mmapstruct_schema::PrimitiveDef::U8 => out.push_str(&ctx.read_u8(val).to_string()),
            mmapstruct_schema::PrimitiveDef::I8 => out.push_str(&ctx.read_i8(val).to_string()),
            mmapstruct_schema::PrimitiveDef::Usize => out.push_str(&ctx.read_usize(val).to_string()),
            mmapstruct_schema::PrimitiveDef::Isize => out.push_str(&ctx.read_isize(val).to_string()),
        },
        mmapstruct_schema::TypeDefReader::Str { .. } => {
            let s = if val.limit as usize <= ctx.data().len() {
                ctx.read_str(val).to_string()
            } else {
                "<bad str>".to_string()
            };
            if s.len() > opts.max_string {
                out.push_str(&format!("\"{}...\"", &s[..opts.max_string.saturating_sub(3)]));
            } else {
                out.push_str(&format!("\"{}\"", s));
            }
        }
        mmapstruct_schema::TypeDefReader::Struct { .. } => out.push_str("{ ... }"),
        mmapstruct_schema::TypeDefReader::Enum { .. } => {
            if let Some((variant_type_idx, _)) = ctx.enum_variant(val) {
                let vt = ctx.type_reader(variant_type_idx);
                if let mmapstruct_schema::TypeDefReader::Struct { name, .. } = vt {
                    out.push_str(name);
                } else {
                    out.push_str("<enum>");
                }
            } else {
                out.push_str("<enum>");
            }
        }
        mmapstruct_schema::TypeDefReader::Array { .. } => {
            if val.limit as usize <= ctx.data().len() {
                out.push_str(&format!("[{} elements]", ctx.array_len(val)));
            } else {
                out.push_str("[?]");
            }
        }
        mmapstruct_schema::TypeDefReader::Option { inner_type } => {
            if val.offset == val.limit {
                out.push_str("None");
            } else {
                let inner_val = mmapstruct_schema::SchemaValue::new(
                    val.offset, val.limit, **inner_type,
                );
                out.push_str("Some(");
                write_value_inline(ctx, &inner_val, out, opts);
                out.push_str(")");
            }
        }
        mmapstruct_schema::TypeDefReader::ByRef { inner_type } => {
            let inner_val = mmapstruct_schema::SchemaValue::new(
                val.offset, val.limit, **inner_type,
            );
            write_value_inline(ctx, &inner_val, out, opts);
        }
        mmapstruct_schema::TypeDefReader::HashMap { .. } => {
            if val.limit as usize <= ctx.data().len() {
                out.push_str(&format!("[{} entries]", ctx.hashmap_len(val)));
            } else {
                out.push_str("[?]");
            }
        }
        mmapstruct_schema::TypeDefReader::PodStruct { .. } => out.push_str("[pod]"),
    }
}

fn write_value_tree(
    ctx: &mmapstruct_schema::context::RuntimeSchemaCtx<'_>,
    val: &mmapstruct_schema::SchemaValue,
    out: &mut String,
    depth: usize,
    opts: &TreeOptions,
) {
    if depth > opts.depth {
        return;
    }
    write_value_tree_prefix(ctx, val, out, "", depth, opts);
}

fn write_value_tree_prefix(
    ctx: &mmapstruct_schema::context::RuntimeSchemaCtx<'_>,
    val: &mmapstruct_schema::SchemaValue,
    out: &mut String,
    prefix: &str,
    depth: usize,
    opts: &TreeOptions,
) {
    if depth > opts.depth {
        return;
    }
    let t = ctx.type_reader(val.type_idx);
    match t {
        mmapstruct_schema::TypeDefReader::Struct { fields, .. } => {
            let field_count = fields.len();
            for fi in 0..field_count {
                let field = fields.get(fi).unwrap();
                let is_last = fi + 1 == field_count;
                let conn = if is_last { "└── " } else { "├── " };
                let child_prefix = if is_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                };
                out.push_str(&format!("{}{}", prefix, conn));
                out.push_str(field.name);
                let fv = ctx.field_get(val, field.name);
                if let Some(fv) = fv {
                    let ft = ctx.type_reader(fv.type_idx);
                    write_type_annotation(ft, out);
                    let can_expand = fv.offset < fv.limit && fv.limit as usize <= ctx.data().len();
                    if can_expand && depth + 1 <= opts.depth {
                        match ft {
                            mmapstruct_schema::TypeDefReader::Struct { .. } => {
                                out.push('\n');
                                write_value_tree_prefix(ctx, &fv, out, &child_prefix, depth + 1, opts);
                                continue;
                            }
                            mmapstruct_schema::TypeDefReader::Enum { .. } => {
                                out.push_str(" = ");
                                write_value_inline(ctx, &fv, out, opts);
                                out.push('\n');
                                write_value_tree_prefix(ctx, &fv, out, &child_prefix, depth + 1, opts);
                                continue;
                            }
                            mmapstruct_schema::TypeDefReader::Array { .. } => {
                                out.push_str(" = ");
                                write_value_inline(ctx, &fv, out, opts);
                                out.push('\n');
                                write_value_tree_prefix(ctx, &fv, out, &child_prefix, depth + 1, opts);
                                continue;
                            }
                            _ => {}
                        }
                    }
                    out.push_str(" = ");
                    write_value_inline(ctx, &fv, out, opts);
                    out.push('\n');
                    if can_expand {
                        write_value_tree_prefix(ctx, &fv, out, &child_prefix, depth + 1, opts);
                    }
                } else {
                    out.push_str(" = <error>\n");
                }
            }
        }
        mmapstruct_schema::TypeDefReader::Enum { .. } => {
            if let Some((variant_type_idx, _data_off)) = ctx.enum_variant(val) {
                let vt = ctx.type_reader(variant_type_idx);
                if let mmapstruct_schema::TypeDefReader::Struct { fields, .. } = vt {
                    let field_count = fields.len();
                    for fi in 0..field_count {
                        let field = fields.get(fi).unwrap();
                        let is_last = fi + 1 == field_count;
                        let conn = if is_last { "└── " } else { "├── " };
                        let child_prefix = if is_last {
                            format!("{}    ", prefix)
                        } else {
                            format!("{}│   ", prefix)
                        };
                        out.push_str(&format!("{}{}", prefix, conn));
                        out.push_str(field.name);
                        let fv = ctx.field_get(val, field.name);
                        if let Some(fv) = fv {
                            let ft = ctx.type_reader(fv.type_idx);
                            write_type_annotation(ft, out);
                            let can_expand = fv.offset < fv.limit && fv.limit as usize <= ctx.data().len();
                            if can_expand && depth + 1 <= opts.depth {
                                match ft {
                                    mmapstruct_schema::TypeDefReader::Struct { .. }
                                    | mmapstruct_schema::TypeDefReader::Enum { .. }
                                    | mmapstruct_schema::TypeDefReader::Array { .. }
                                    | mmapstruct_schema::TypeDefReader::Option { .. } => {
                                        out.push('\n');
                                        write_value_tree_prefix(ctx, &fv, out, &child_prefix, depth + 1, opts);
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                            out.push_str(" = ");
                            write_value_inline(ctx, &fv, out, opts);
                            out.push('\n');
                            if can_expand {
                                write_value_tree_prefix(ctx, &fv, out, &child_prefix, depth + 1, opts);
                            }
                        } else {
                            out.push_str(" = <error>\n");
                        }
                    }
                }
            }
        }
        mmapstruct_schema::TypeDefReader::Array { .. } => {
            if val.limit as usize > ctx.data().len() {
                return;
            }
            let len = ctx.array_len(val) as usize;
            let show = len.min(opts.max_array);
            for i in 0..show {
                let elem_last = i + 1 == show && show == len;
                let conn = if elem_last { "└── " } else { "├── " };
                let child_prefix = if elem_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                };
                out.push_str(&format!("{}{}[{}]", prefix, conn, i));
                if let Some(elem) = ctx.array_get(val, i as u64) {
                    let et = ctx.type_reader(elem.type_idx);
                    write_type_annotation(et, out);
                    out.push('\n');
                    write_value_tree_prefix(ctx, &elem, out, &child_prefix, depth + 1, opts);
                } else {
                    out.push_str(" = <error>\n");
                }
            }
            if len > opts.max_array {
                let remaining = len - opts.max_array;
                out.push_str(&format!(
                    "{}    ... ({} more)\n",
                    prefix, remaining
                ));
            }
        }
        mmapstruct_schema::TypeDefReader::HashMap { .. } => {
            if val.limit as usize > ctx.data().len() {
                return;
            }
            let len = ctx.hashmap_len(val);
            let show = len.min(opts.max_array);
            for i in 0..show {
                let is_last = i + 1 == show && show == len;
                let conn = if is_last { "└── " } else { "├── " };
                let child_prefix = if is_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                };
                out.push_str(&format!("{}{}", prefix, conn));
                if let Some((key, val_entry)) = ctx.hashmap_get_index(val, i) {
                    // key inline
                    write_value_inline(ctx, &key, out, opts);
                    let vt = ctx.type_reader(val_entry.type_idx);
                    write_type_annotation(vt, out);
                    let can_expand = val_entry.offset < val_entry.limit
                        && val_entry.limit as usize <= ctx.data().len();
                    if can_expand && depth + 1 <= opts.depth {
                        match vt {
                            mmapstruct_schema::TypeDefReader::Struct { .. }
                            | mmapstruct_schema::TypeDefReader::Enum { .. }
                            | mmapstruct_schema::TypeDefReader::Array { .. } => {
                                out.push('\n');
                                write_value_tree_prefix(ctx, &val_entry, out, &child_prefix, depth + 1, opts);
                                continue;
                            }
                            _ => {}
                        }
                    }
                    out.push_str(" = ");
                    write_value_inline(ctx, &val_entry, out, opts);
                    out.push('\n');
                    if can_expand {
                        write_value_tree_prefix(ctx, &val_entry, out, &child_prefix, depth + 1, opts);
                    }
                } else {
                    out.push_str(" = <error>\n");
                }
            }
            if len > opts.max_array {
                let remaining = len - opts.max_array;
                out.push_str(&format!("{}    ... ({} more)\n", prefix, remaining));
            }
        }
        mmapstruct_schema::TypeDefReader::Option { inner_type } => {
            if val.offset < val.limit {
                let inner_val = mmapstruct_schema::SchemaValue::new(
                    val.offset, val.limit, **inner_type,
                );
                write_value_tree_prefix(ctx, &inner_val, out, prefix, depth, opts);
            }
        }
        _ => {}
    }
}
