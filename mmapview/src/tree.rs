//! Tree-formatted viewer — renders a mmapstruct file as an indented tree.
//!
//! Designed for use as an F3 external viewer in Total Commander / Midnight Commander.

use mmapstruct::BufReader;

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

/// Result of loading a data file: embedded schema bytes and the payload range.
pub struct SchemaLoadResult {
    pub schema_bytes: Vec<u8>,
    pub payload_range: std::ops::Range<usize>,
}

/// Load embedded schema from a data buffer, falling back to registry.
pub fn load_schema_for_data(data: &[u8]) -> Result<SchemaLoadResult, String> {
    let reader = BufReader::new(data.to_vec());
    if let Some(embedded) = reader.embedded_schema() {
        let (payload, _, _, _) = mmapstruct::file_reader::parse_header(data);
        let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
        return Ok(SchemaLoadResult {
            schema_bytes: embedded.to_vec(),
            payload_range: payload_start..payload_start + payload.len(),
        });
    }
    let (payload, _schema, hash, _crc) = mmapstruct::file_reader::parse_header(data);
    if let Some(h) = hash {
        if let Some(s) = crate::registry::load_schema(h) {
            let payload_start = payload.as_ptr() as usize - data.as_ptr() as usize;
            return Ok(SchemaLoadResult {
                schema_bytes: s,
                payload_range: payload_start..payload_start + payload.len(),
            });
        }
    }
    Err(
        "No embedded schema found and no schema in registry.\n\
         Write the file with write_to_file_with_embedded_schema() or\n\
         save the schema to ~/.config/mmapstruct/schemas/<layout_hash>.mms."
            .to_string(),
    )
}

/// Render the entire file as a tree string.
pub fn format_file(data: &[u8], opts: &TreeOptions) -> Result<String, String> {
    let loaded = load_schema_for_data(data)?;
    let ctx =
        mmapstruct_schema::context::SchemaDynContext::new(data, &loaded.schema_bytes);
    let schema_reader = <mmapstruct_schema::Schema as mmapstruct::traits::MmapRead>::read(
        &loaded.schema_bytes,
        0..loaded.schema_bytes.len(),
    );
    let root_type = *schema_reader.root_type;

    use mmapstruct_schema::context::SchemaValue;
    let root = SchemaValue::new(
        loaded.payload_range.start as u64,
        loaded.payload_range.end as u64,
        root_type,
    );

    let type_info = ctx.type_reader(root_type);
    let root_name = match type_info {
        mmapstruct_schema::SchemaTypeDefReader::Struct { def } => def.name.to_string(),
        mmapstruct_schema::SchemaTypeDefReader::Enum { def } => def.name.to_string(),
        _ => "Root".to_string(),
    };

    let mut out = String::new();
    out.push_str(&root_name);
    out.push('\n');

    match type_info {
        mmapstruct_schema::SchemaTypeDefReader::Struct { def } => {
            write_struct_children(&ctx, &root, &def, &mut out, 0, opts);
        }
        mmapstruct_schema::SchemaTypeDefReader::Enum { .. } => {
            write_value_tree(&ctx, &root, &mut out, 0, true, opts);
        }
        _ => {
            write_value_tree(&ctx, &root, &mut out, 0, false, opts);
        }
    }

    Ok(out)
}

fn write_struct_children(
    ctx: &mmapstruct_schema::context::SchemaDynContext<'_>,
    val: &mmapstruct_schema::context::SchemaValue,
    def: &mmapstruct_schema::StructDefReader<'_>,
    out: &mut String,
    depth: usize,
    opts: &TreeOptions,
) {
    let field_count = def.fields.len();
    for i in 0..field_count {
        let is_last = i + 1 == field_count;
        let field = def.fields.get(i).unwrap();

        let prefix = if is_last { "└── " } else { "├── " };

        out.push_str(&format!("{}{}", "    ".repeat(depth), prefix));
        out.push_str(field.name);

        let field_val = ctx.field_get(val, field.name);
        if let Some(fv) = field_val {
            let ft = ctx.type_reader(fv.type_idx);
            write_type_annotation(ft, out);
            if opts.no_values {
                out.push('\n');
            } else {
                out.push_str(" = ");
                write_value_inline(ctx, &fv, out, opts);
                out.push('\n');
            }
            write_value_tree(ctx, &fv, out, depth + 1, is_last, opts);
        } else {
            out.push_str(" = <error>\n");
        }
    }
}

fn write_type_annotation(ft: &mmapstruct_schema::SchemaTypeDefReader<'_>, out: &mut String) {
    use mmapstruct_schema::SchemaTypeDefReader;
    match ft {
        SchemaTypeDefReader::U32 => out.push_str(": u32"),
        SchemaTypeDefReader::I32 => out.push_str(": i32"),
        SchemaTypeDefReader::U64 => out.push_str(": u64"),
        SchemaTypeDefReader::I64 => out.push_str(": i64"),
        SchemaTypeDefReader::F32 => out.push_str(": f32"),
        SchemaTypeDefReader::F64 => out.push_str(": f64"),
        SchemaTypeDefReader::Bool => out.push_str(": bool"),
        SchemaTypeDefReader::U16 => out.push_str(": u16"),
        SchemaTypeDefReader::I16 => out.push_str(": i16"),
        SchemaTypeDefReader::U8 => out.push_str(": u8"),
        SchemaTypeDefReader::I8 => out.push_str(": i8"),
        SchemaTypeDefReader::Usize => out.push_str(": usize"),
        SchemaTypeDefReader::Isize => out.push_str(": isize"),
        SchemaTypeDefReader::Str { .. } => out.push_str(": str"),
        SchemaTypeDefReader::PodStruct { .. } => out.push_str(": [pod]"),
        SchemaTypeDefReader::Struct { def } => {
            out.push_str(&format!(": {}", def.name));
        }
        SchemaTypeDefReader::Enum { def } => {
            out.push_str(&format!(": {}", def.name));
        }
        SchemaTypeDefReader::Array { .. } => out.push_str(": Array"),
        SchemaTypeDefReader::Option { .. } => out.push_str(": Option"),
        SchemaTypeDefReader::ByRef { .. } => out.push_str(": byref"),
        SchemaTypeDefReader::HashMap { .. } => out.push_str(": HashMap"),
    }
}

fn write_value_inline(
    ctx: &mmapstruct_schema::context::SchemaDynContext<'_>,
    val: &mmapstruct_schema::context::SchemaValue,
    out: &mut String,
    opts: &TreeOptions,
) {
    use mmapstruct_schema::SchemaTypeDefReader;
    let t = ctx.type_reader(val.type_idx);
    match t {
        SchemaTypeDefReader::U32 => out.push_str(&ctx.read_u32(val).to_string()),
        SchemaTypeDefReader::I32 => out.push_str(&ctx.read_i32(val).to_string()),
        SchemaTypeDefReader::U64 => out.push_str(&ctx.read_u64(val).to_string()),
        SchemaTypeDefReader::I64 => out.push_str(&ctx.read_i64(val).to_string()),
        SchemaTypeDefReader::F32 => out.push_str(&ctx.read_f32(val).to_string()),
        SchemaTypeDefReader::F64 => out.push_str(&ctx.read_f64(val).to_string()),
        SchemaTypeDefReader::Bool => {
            out.push_str(if ctx.read_bool(val) { "true" } else { "false" })
        }
        SchemaTypeDefReader::U16 => out.push_str(&ctx.read_u16(val).to_string()),
        SchemaTypeDefReader::I16 => out.push_str(&ctx.read_i16(val).to_string()),
        SchemaTypeDefReader::U8 => out.push_str(&ctx.read_u8(val).to_string()),
        SchemaTypeDefReader::I8 => out.push_str(&ctx.read_i8(val).to_string()),
        SchemaTypeDefReader::Usize => out.push_str(&ctx.read_usize(val).to_string()),
        SchemaTypeDefReader::Isize => out.push_str(&ctx.read_isize(val).to_string()),
        SchemaTypeDefReader::Str { .. } => {
            let s = ctx.read_str(val);
            if s.len() > opts.max_string {
                out.push_str(&format!("\"{}...\"", &s[..opts.max_string.saturating_sub(3)]));
            } else {
                out.push_str(&format!("\"{}\"", s));
            }
        }
        SchemaTypeDefReader::Struct { .. } => out.push_str("{ ... }"),
        SchemaTypeDefReader::Enum { .. } => out.push_str("<enum>"),
        SchemaTypeDefReader::Array { .. } => {
            out.push_str(&format!("[{} elements]", ctx.array_len(val)));
        }
        SchemaTypeDefReader::Option { .. } => out.push_str("?"),
        SchemaTypeDefReader::ByRef { .. } => {
            let inner = val.type_idx;
            let inner_val = mmapstruct_schema::context::SchemaValue::new(val.offset, val.limit, inner);
            write_value_inline(ctx, &inner_val, out, opts);
        }
        SchemaTypeDefReader::HashMap { .. } => {
            out.push_str(&format!("[{} entries]", ctx.hashmap_len(val)));
        }
        SchemaTypeDefReader::PodStruct { .. } => out.push_str("[pod]"),
    }
}

fn write_value_tree(
    ctx: &mmapstruct_schema::context::SchemaDynContext<'_>,
    val: &mmapstruct_schema::context::SchemaValue,
    out: &mut String,
    depth: usize,
    _is_last: bool,
    opts: &TreeOptions,
) {
    if depth > opts.depth {
        return;
    }

    use mmapstruct_schema::SchemaTypeDefReader;
    let t = ctx.type_reader(val.type_idx);
    match t {
        SchemaTypeDefReader::Struct { def } => {
            write_struct_children(ctx, val, def, out, depth, opts);
        }
        SchemaTypeDefReader::Enum { .. } => {
            if let Some((_variant, _data_off)) = ctx.enum_variant(val) {
                // variant children printing — skip for now
            }
        }
        SchemaTypeDefReader::Array { .. } => {
            let len = ctx.array_len(val);
            let show = len.min(opts.max_array);
            for i in 0..show {
                let elem_last = i + 1 == show && show == len;
                let prefix = if elem_last { "└── " } else { "├── " };
                out.push_str(&format!("{}    {}[{}]", "    ".repeat(depth), prefix, i));
                if let Some(elem) = ctx.array_get(val, i) {
                    let et = ctx.type_reader(elem.type_idx);
                    write_type_annotation(et, out);
                    if opts.no_values {
                        out.push('\n');
                    } else {
                        out.push_str(" = ");
                        write_value_inline(ctx, &elem, out, opts);
                        out.push('\n');
                    }
                    write_value_tree(ctx, &elem, out, depth + 1, elem_last, opts);
                } else {
                    out.push_str(" = <error>\n");
                }
            }
            if len > opts.max_array {
                let remaining = len - opts.max_array;
                out.push_str(&format!(
                    "{}    ... ({} more)\n",
                    "    ".repeat(depth),
                    remaining
                ));
            }
        }
        _ => {}
    }
}
