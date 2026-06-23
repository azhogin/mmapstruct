use ratatui::text::Line;
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::{PrimitiveDef, TypeDefReader};
use mmapstruct_schema::SchemaValue;

pub const WINDOW_SIZE: usize = 600;
pub const MARGIN: usize = 150;

#[derive(Clone)]
pub struct TreeNode {
    pub line: Line<'static>,
    pub depth: usize,
    pub expanded: bool,
    pub has_children: bool,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub value: Option<SchemaValue>,
}

/// State for one expanded array/map window.
#[derive(Clone)]
pub struct WindowState {
    pub val_offset: u64,
    pub val_type_idx: u32,
    pub total: usize,
    pub window_start: usize,
    pub window_count: usize,
    pub element_expanded: Vec<bool>,
    pub expanded: bool,
}

impl WindowState {
    pub fn matches_val(&self, val: &SchemaValue) -> bool {
        self.val_offset == val.offset && self.val_type_idx == val.type_idx
    }
}

pub fn build_tree(
    ctx: &RuntimeSchemaCtx,
    root: &SchemaValue,
    root_name: &str,
    depth_limit: usize,
    window_state: Option<&WindowState>,
) -> Vec<TreeNode> {
    let mut nodes = Vec::new();
    nodes.push(TreeNode {
        line: Line::from(root_name.to_string()),
        depth: 0,
        expanded: true,
        has_children: true,
        parent: None,
        children: Vec::new(),
        value: None,
    });
    build_children(ctx, root, 0, &mut nodes, 1, depth_limit, "", window_state);
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
    window_state: Option<&WindowState>,
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
                let is_array = matches!(
                    ctx.type_reader(child_val.type_idx),
                    TypeDefReader::Array { .. } | TypeDefReader::HashMap { .. }
                );

                let ws = if is_array {
                    window_state.filter(|ws| ws.matches_val(&child_val))
                } else {
                    None
                };
                let expanded = if let Some(ws) = ws {
                    ws.expanded
                } else if is_array {
                    false
                } else {
                    depth < depth_limit && has_grandchildren
                };

                let line = build_node_line(prefix, connector, field.name, ctx, &child_val, has_grandchildren, expanded);
                let child_idx = nodes.len();
                nodes[parent_idx].children.push(child_idx);
                nodes.push(TreeNode {
                    line,
                    depth,
                    expanded,
                    has_children: has_grandchildren,
                    parent: Some(parent_idx),
                    children: Vec::new(),
                    value: if has_grandchildren { Some(child_val) } else { None },
                });

                if expanded {
                    if let Some(ws) = ws {
                        build_array_window(ctx, &child_val, child_idx, nodes, depth + 1, &child_prefix, ws, depth_limit);
                    } else {
                        build_children(ctx, &child_val, child_idx, nodes, depth + 1, depth_limit, &child_prefix, window_state);
                    }
                }
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
                        let is_array = matches!(
                            ctx.type_reader(child_val.type_idx),
                            TypeDefReader::Array { .. } | TypeDefReader::HashMap { .. }
                        );
                        let ws = if is_array {
                            window_state.filter(|ws| ws.matches_val(&child_val))
                        } else {
                            None
                        };
                        let expanded = if let Some(ws) = ws {
                            ws.expanded
                        } else if is_array {
                            false
                        } else {
                            depth < depth_limit && has_grandchildren
                        };

                        let line = build_node_line(prefix, connector, field.name, ctx, &child_val, has_grandchildren, expanded);
                        let child_idx = nodes.len();
                        nodes[parent_idx].children.push(child_idx);
                        nodes.push(TreeNode {
                            line,
                            depth,
                            expanded,
                            has_children: has_grandchildren,
                            parent: Some(parent_idx),
                            children: Vec::new(),
                            value: if has_grandchildren { Some(child_val) } else { None },
                        });

                        if expanded {
                            if let Some(ws) = ws {
                                build_array_window(ctx, &child_val, child_idx, nodes, depth + 1, &child_prefix, ws, depth_limit);
                            } else {
                                build_children(ctx, &child_val, child_idx, nodes, depth + 1, depth_limit, &child_prefix, window_state);
                            }
                        }
                    }
                }
            }
        }

        TypeDefReader::Array { .. } | TypeDefReader::HashMap { .. } => {
            // Only reached when this node itself has a window_state.
            // The container node was already created by the parent's struct/enum handler.
            // This arm handles the case where the root itself is an array.
        }

        TypeDefReader::Option { inner_type } => {
            if val.offset < val.limit {
                let inner_val = SchemaValue::new(val.offset, val.limit, **inner_type);
                build_children(ctx, &inner_val, parent_idx, nodes, depth, depth_limit, prefix, window_state);
            }
        }

        TypeDefReader::ByRef { inner_type } => {
            let inner_val = SchemaValue::new(val.offset, val.limit, **inner_type);
            build_children(ctx, &inner_val, parent_idx, nodes, depth, depth_limit, prefix, window_state);
        }

        _ => {}
    }
}

fn build_array_window(
    ctx: &RuntimeSchemaCtx,
    val: &SchemaValue,
    parent_idx: usize,
    nodes: &mut Vec<TreeNode>,
    depth: usize,
    prefix: &str,
    ws: &WindowState,
    depth_limit: usize,
) {
    let total = ws.total;
    let win_start = ws.window_start;
    let win_end = (win_start + ws.window_count).min(total);

    let is_array = matches!(ctx.type_reader(val.type_idx), TypeDefReader::Array { .. });

    for i in win_start..win_end {
        let is_last = i + 1 == total;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            format!("{}    ", prefix)
        } else {
            format!("{}│   ", prefix)
        };

        let elem = if is_array {
            ctx.array_get(val, i as u64)
        } else {
            ctx.hashmap_get_index(val, i).map(|(_key, entry)| entry)
        };
        let Some(elem_val) = elem else { continue };

        let local_idx = i - win_start;
        let elem_expanded = local_idx < ws.element_expanded.len() && ws.element_expanded[local_idx];
        let has_grandchildren = type_can_expand(ctx, &elem_val);

        let name = if is_array {
            format!("[{}]", i)
        } else {
            if let Some((key, _)) = ctx.hashmap_get_index(val, i) {
                inline_value_text(ctx, &key)
            } else {
                format!("[{}]", i)
            }
        };
        let line = build_node_line(prefix, connector, &name, ctx, &elem_val, has_grandchildren, elem_expanded);
        let child_idx = nodes.len();
        nodes[parent_idx].children.push(child_idx);
        nodes.push(TreeNode {
            line,
            depth,
            expanded: elem_expanded,
            has_children: has_grandchildren,
            parent: Some(parent_idx),
            children: Vec::new(),
            value: if has_grandchildren { Some(elem_val) } else { None },
        });

        if elem_expanded {
            build_children(ctx, &elem_val, child_idx, nodes, depth + 1, depth_limit, &child_prefix, None);
        }
    }
}

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
    let conn = connector.trim_end();
    let indicator = if !has_children { " " } else if expanded { "▼" } else { "▶" };
    let text = format!("{}{}{} {}{} = {}", prefix, conn, indicator, name, type_ann, inline);
    Line::from(text)
}

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
