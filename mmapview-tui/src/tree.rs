use ratatui::text::Line;
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::{PrimitiveDef, TypeDefReader};
use mmapstruct_schema::SchemaValue;

/// A single node in the tree data structure.
#[derive(Clone)]
pub struct TreeNode {
    /// Pre-formatted display line (prefix + name + type + value).
    pub line: Line<'static>,
    /// Nesting depth (0 = root).
    pub depth: usize,
    /// Whether children are currently visible.
    pub expanded: bool,
    /// Whether this node can have children (struct/enum/array/map).
    pub has_children: bool,
    /// Index of parent node, or None for root.
    #[allow(dead_code)]
    pub parent: Option<usize>,
    /// Indices of direct child nodes in the flat list.
    pub children: Vec<usize>,
}

/// Build the full flat tree from a root schema value.
pub fn build_tree(
    ctx: &RuntimeSchemaCtx,
    root: &SchemaValue,
    root_name: &str,
    depth_limit: usize,
) -> Vec<TreeNode> {
    let mut nodes = Vec::new();
    nodes.push(TreeNode {
        line: Line::from(root_name.to_string()),
        depth: 0,
        expanded: true,
        has_children: true,
        parent: None,
        children: Vec::new(),
    });
    build_children(ctx, root, 0, &mut nodes, 1, depth_limit, "");
    nodes
}

fn build_children(
    ctx: &RuntimeSchemaCtx,
    val: &SchemaValue,
    parent_idx: usize,
    nodes: &mut Vec<TreeNode>,
    depth: usize,
    depth_limit: usize,
    prefix: &str,
) {
    let t = ctx.type_reader(val.type_idx);
    match t {
        TypeDefReader::Struct { fields, .. } => {
            let n = fields.len();
            for fi in 0..n {
                let field = fields.get(fi).unwrap();
                let is_last = fi + 1 == n;
                let connector = if is_last { "└── " } else { "├── " };
                let child_prefix = if is_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                };

                let child_val = ctx.field_get(val, field.name);
                let Some(child_val) = child_val else { continue };

                let has_grandchildren = type_can_expand(ctx, &child_val);
                let expanded = depth < depth_limit && has_grandchildren;

                let line = build_node_line(prefix, connector, &field.name, ctx, &child_val, has_grandchildren, expanded);
                let child_idx = nodes.len();
                nodes[parent_idx].children.push(child_idx);
                nodes.push(TreeNode {
                    line,
                    depth,
                    expanded,
                    has_children: has_grandchildren,
                    parent: Some(parent_idx),
                    children: Vec::new(),
                });

                    build_children(ctx, &child_val, child_idx, nodes, depth + 1, depth_limit, &child_prefix);
            }
        }

        TypeDefReader::Enum { .. } => {
            if let Some((variant_type_idx, _data_off)) = ctx.enum_variant(val) {
                let vt = ctx.type_reader(variant_type_idx);
                if let TypeDefReader::Struct { fields, .. } = vt {
                    let n = fields.len();
                    for fi in 0..n {
                        let field = fields.get(fi).unwrap();
                        let is_last = fi + 1 == n;
                        let connector = if is_last { "└── " } else { "├── " };
                        let child_prefix = if is_last {
                            format!("{}    ", prefix)
                        } else {
                            format!("{}│   ", prefix)
                        };

                        let child_val = ctx.field_get(val, field.name);
                        let Some(child_val) = child_val else { continue };

                        let has_grandchildren = type_can_expand(ctx, &child_val);
                        let expanded = depth < depth_limit && has_grandchildren;

                        let line = build_node_line(prefix, connector, &field.name, ctx, &child_val, has_grandchildren, expanded);
                        let child_idx = nodes.len();
                        nodes[parent_idx].children.push(child_idx);
                        nodes.push(TreeNode {
                            line,
                            depth,
                            expanded,
                            has_children: has_grandchildren,
                            parent: Some(parent_idx),
                            children: Vec::new(),
                        });

                            build_children(ctx, &child_val, child_idx, nodes, depth + 1, depth_limit, &child_prefix);
                    }
                }
            }
        }

        TypeDefReader::Array { .. } => {
            if val.limit as usize > ctx.data().len() {
                return;
            }
            let len = ctx.array_len(val) as usize;
            for i in 0..len {
                let is_last = i + 1 == len;
                let connector = if is_last { "└── " } else { "├── " };
                let child_prefix = if is_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                };

                if let Some(elem) = ctx.array_get(val, i as u64) {
                    let has_grandchildren = type_can_expand(ctx, &elem);
                    let expanded = depth < depth_limit && has_grandchildren;

                    let name = format!("[{}]", i);
                    let line = build_node_line(prefix, connector, &name, ctx, &elem, has_grandchildren, expanded);
                    let child_idx = nodes.len();
                    nodes[parent_idx].children.push(child_idx);
                    nodes.push(TreeNode {
                        line,
                        depth,
                        expanded,
                        has_children: has_grandchildren,
                        parent: Some(parent_idx),
                        children: Vec::new(),
                    });

                        build_children(ctx, &elem, child_idx, nodes, depth + 1, depth_limit, &child_prefix);
                }
            }
        }

        TypeDefReader::HashMap { .. } => {
            if val.limit as usize > ctx.data().len() {
                return;
            }
            let len = ctx.hashmap_len(val);
            for i in 0..len {
                let is_last = i + 1 == len;
                let connector = if is_last { "└── " } else { "├── " };
                let child_prefix = if is_last {
                    format!("{}    ", prefix)
                } else {
                    format!("{}│   ", prefix)
                };

                if let Some((key, entry_val)) = ctx.hashmap_get_index(val, i) {
                    let key_text = inline_value_text(ctx, &key);
                    let has_grandchildren = type_can_expand(ctx, &entry_val);
                    let expanded = depth < depth_limit && has_grandchildren;

                    let name = &key_text;
                    let line = build_node_line(prefix, connector, name, ctx, &entry_val, has_grandchildren, expanded);
                    let child_idx = nodes.len();
                    nodes[parent_idx].children.push(child_idx);
                    nodes.push(TreeNode {
                        line,
                        depth,
                        expanded,
                        has_children: has_grandchildren,
                        parent: Some(parent_idx),
                        children: Vec::new(),
                    });

                        build_children(ctx, &entry_val, child_idx, nodes, depth + 1, depth_limit, &child_prefix);
                }
            }
        }

        TypeDefReader::Option { inner_type } => {
            if val.offset < val.limit {
                let inner_val = SchemaValue::new(val.offset, val.limit, **inner_type);
                build_children(ctx, &inner_val, parent_idx, nodes, depth, depth_limit, prefix);
            }
        }

        TypeDefReader::ByRef { inner_type } => {
            let inner_val = SchemaValue::new(val.offset, val.limit, **inner_type);
            build_children(ctx, &inner_val, parent_idx, nodes, depth, depth_limit, prefix);
        }

        _ => {}
    }
}

/// Build the display line for a single tree node.
fn build_node_line(
    prefix: &str,
    connector: &str,
    name: &str,
    ctx: &RuntimeSchemaCtx,
    val: &SchemaValue,
    has_children: bool,
    expanded: bool,
) -> Line<'static> {
    let t = ctx.type_reader(val.type_idx);
    let type_ann = type_annotation(t);
    let inline = inline_value_text(ctx, val);
    // connector is "├── " or "└── " — strip trailing space, add indicator
    let conn = connector.trim_end();
    let indicator = if !has_children { " " } else if expanded { "▼" } else { "▶" };
    let text = format!("{}{}{} {}{} = {}", prefix, conn, indicator, name, type_ann, inline);
    Line::from(text)
}

/// Text representation of a value's type annotation.
fn type_annotation(t: &TypeDefReader) -> &'static str {
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

/// Single-line text representation of a value.
fn inline_value_text(ctx: &RuntimeSchemaCtx, val: &SchemaValue) -> String {
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
            let s = if val.limit as usize <= ctx.data().len() {
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
        TypeDefReader::Struct { .. } => "{ ... }".to_string(),
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
        TypeDefReader::Array { .. } => {
            if val.limit as usize <= ctx.data().len() {
                format!("[{} elements]", ctx.array_len(val))
            } else {
                "[?]".to_string()
            }
        }
        TypeDefReader::Option { inner_type } => {
            if val.offset == val.limit {
                "None".to_string()
            } else {
                let inner = SchemaValue::new(val.offset, val.limit, **inner_type);
                format!("Some({})", inline_value_text(ctx, &inner))
            }
        }
        TypeDefReader::ByRef { inner_type } => {
            let inner = SchemaValue::new(val.offset, val.limit, **inner_type);
            inline_value_text(ctx, &inner)
        }
        TypeDefReader::HashMap { .. } => {
            if val.limit as usize <= ctx.data().len() {
                format!("[{} entries]", ctx.hashmap_len(val))
            } else {
                "[?]".to_string()
            }
        }
        TypeDefReader::PodStruct { .. } => "[pod]".to_string(),
    }
}

/// Whether a type can have expandable children.
fn type_can_expand(ctx: &RuntimeSchemaCtx, val: &SchemaValue) -> bool {
    let t = ctx.type_reader(val.type_idx);
    match t {
        TypeDefReader::Struct { fields, .. } => fields.len() > 0,
        TypeDefReader::Enum { .. } => {
            if let Some((variant_type_idx, _)) = ctx.enum_variant(val) {
                let vt = ctx.type_reader(variant_type_idx);
                if let TypeDefReader::Struct { fields, .. } = vt {
                    fields.len() > 0
                } else {
                    false
                }
            } else {
                false
            }
        }
        TypeDefReader::Array { .. } | TypeDefReader::HashMap { .. } => true,
        _ => false,
    }
}
