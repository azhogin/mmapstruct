//! `mmapview describe <file>` — compact structure map for AI agents.
//!
//! Walks the schema lazily via `RuntimeSchemaCtx`: array lengths are read
//! from count prefixes (O(1)), element types from a single `array_get(0)`.
//! No bulk data reads.

use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::{ArrayLayoutDefReader, PrimitiveDef, TypeDefReader};
use mmapstruct_schema::SchemaValue;

/// Options for [`describe`].
#[derive(Clone, Copy)]
pub struct DescribeOpts {
    /// Max nesting depth (default 5).
    pub depth: usize,
}

impl Default for DescribeOpts {
    fn default() -> Self {
        Self { depth: 5 }
    }
}

/// Produce a text tree of the data structure starting at `root`.
pub fn describe(
    ctx: &RuntimeSchemaCtx<'_>,
    root: &SchemaValue,
    opts: &DescribeOpts,
) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    walk(ctx, root, "/", &opts, &mut depth, &mut out);
    out
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn walk(
    ctx: &RuntimeSchemaCtx<'_>,
    val: &SchemaValue,
    path: &str,
    opts: &DescribeOpts,
    depth: &mut usize,
    out: &mut String,
) {
    if *depth > opts.depth {
        return;
    }
    // Lazy is resolved at serialization time; no Lazy variant exists here.
    match ctx.type_reader(val.type_idx) {
        TypeDefReader::Struct { name, fields, .. } => {
            let n = (**name).to_string();
            if fields.is_empty() {
                out.push_str(&format!("{path}: {n} {{}}\n"));
                return;
            }
            out.push_str(&format!("{path}: {n}\n"));
            *depth += 1;
            for i in 0..fields.len() {
                let fd = fields.get(i).unwrap();
                let fname: &str = &fd.name;
                indent(out, *depth);
                match ctx.field_get(val, fname) {
                    Some(fv) => walk(ctx, &fv, &format!("/{fname}"), opts, depth, out),
                    None => out.push_str(&format!("/{fname}: ?\n")),
                }
            }
            *depth -= 1;
        }
        TypeDefReader::Enum { name, variants, .. } => {
            let n = (**name).to_string();
            out.push_str(&format!("{path}: {n} (enum)\n"));
            // Active variant + its fields.
            if let Some((variant_idx, _off)) = ctx.enum_variant(val) {
                if let TypeDefReader::Struct { name: vname, fields, .. } =
                    ctx.type_reader(variant_idx)
                {
                    indent(out, *depth + 1);
                    out.push_str(&format!("active: {}\n", &**vname));
                    *depth += 2;
                    for i in 0..fields.len() {
                        let fd = fields.get(i).unwrap();
                        let fname: &str = &fd.name;
                        indent(out, *depth);
                        match ctx.field_get(val, fname) {
                            Some(fv) => walk(ctx, &fv, &format!("/{fname}"), opts, depth, out),
                            None => out.push_str(&format!("/{fname}: ?\n")),
                        }
                    }
                    *depth -= 2;
                }
            }
            let _ = variants;
        }
        TypeDefReader::Array { elem_type, layout } => {
            let len = ctx.array_len(val);
            let layout_tag = match layout {
                ArrayLayoutDefReader::Pod { .. } => "pod",
                ArrayLayoutDefReader::FixedPod { .. } => "fixedpod",
                ArrayLayoutDefReader::Indexed => "indexed",
                ArrayLayoutDefReader::Fixed { .. } => "fixed",
            };
            let elem_ty = type_name_of(ctx, **elem_type);
            out.push_str(&format!(
                "{path}: [{elem_ty}] ({layout_tag}) len={len}\n"
            ));
            // Show the first element's shape once.
            if len > 0 && *depth < opts.depth {
                if let Some(ev) = ctx.array_get(val, 0) {
                    *depth += 1;
                    indent(out, *depth);
                    walk(ctx, &ev, "[0]", opts, depth, out);
                    *depth -= 1;
                }
            }
        }
        TypeDefReader::HashMap { key_type, val_type, .. } => {
            let entries = ctx.hashmap_len(val);
            out.push_str(&format!(
                "{path}: map<{}, {}> entries={entries}\n",
                type_name_of(ctx, **key_type),
                type_name_of(ctx, **val_type)
            ));
        }
        TypeDefReader::Option { inner_type } => {
            let present = val.mmap_begin < val.mmap_end;
            let inner = type_name_of(ctx, **inner_type);
            if present {
                out.push_str(&format!("{path}: Option<{inner}> = some\n"));
            } else {
                out.push_str(&format!("{path}: Option<{inner}> = none\n"));
            }
        }
        TypeDefReader::ByRef { inner_type } => {
            out.push_str(&format!("{path}: &{}\n", type_name_of(ctx, **inner_type)));
        }
        TypeDefReader::Str { size } => match size {
            Some(n) => out.push_str(&format!("{path}: str[{n}]\n")),
            None => out.push_str(&format!("{path}: str\n")),
        },
        TypeDefReader::PodStruct { size } => {
            out.push_str(&format!("{path}: [pod {}B]\n", **size));
        }
        TypeDefReader::Primitive(p) => {
            let tn = match p {
                PrimitiveDef::U8 => "u8",
                PrimitiveDef::U16 => "u16",
                PrimitiveDef::U32 => "u32",
                PrimitiveDef::U64 => "u64",
                PrimitiveDef::I8 => "i8",
                PrimitiveDef::I16 => "i16",
                PrimitiveDef::I32 => "i32",
                PrimitiveDef::I64 => "i64",
                PrimitiveDef::F32 => "f32",
                PrimitiveDef::F64 => "f64",
                PrimitiveDef::Bool => "bool",
                PrimitiveDef::Usize => "usize",
                PrimitiveDef::Isize => "isize",
            };
            out.push_str(&format!("{path}: {tn}\n"));
        }
    }
}

fn type_name_of(ctx: &RuntimeSchemaCtx<'_>, type_idx: u32) -> String {
    crate::runtime_json::type_name(ctx, type_idx)
}
