use crate::tree::{self, TreeNode, WindowState, WINDOW_SIZE, MARGIN};
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::SchemaValue;

pub struct App {
    pub nodes: Vec<TreeNode>,
    pub visible: Vec<usize>,
    pub selected: usize,
    pub scroll: usize,
    pub depth: usize,
    pub should_quit: bool,
    pub ctx: &'static RuntimeSchemaCtx<'static>,
    pub root_val: SchemaValue,
    pub root_name: String,
    pub window_state: Option<WindowState>,
}

impl App {
    pub fn new(
        nodes: Vec<TreeNode>,
        depth: usize,
        ctx: &'static RuntimeSchemaCtx<'static>,
        root_val: SchemaValue,
        root_name: String,
    ) -> Self {
        let visible = compute_visible(&nodes);
        App {
            nodes,
            visible,
            selected: 0,
            scroll: 0,
            depth,
            should_quit: false,
            ctx,
            root_val,
            root_name,
            window_state: None,
        }
    }

    pub fn navigate_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.check_window_boundary();
        }
        self.ensure_selected_visible();
    }

    pub fn navigate_down(&mut self) {
        if self.selected + 1 < self.visible.len() {
            self.selected += 1;
            self.check_window_boundary();
        }
        self.ensure_selected_visible();
    }

    pub fn navigate_page_up(&mut self) {
        let page_size: usize = 20;
        if self.selected >= page_size {
            self.selected -= page_size;
        } else {
            self.selected = 0;
        }
        self.check_window_boundary();
        self.ensure_selected_visible();
    }

    pub fn navigate_page_down(&mut self) {
        let page_size: usize = 20;
        let last = self.visible.len().saturating_sub(1);
        self.selected = (self.selected + page_size).min(last);
        self.check_window_boundary();
        self.ensure_selected_visible();
    }

    pub fn toggle_expand(&mut self) {
        if self.selected >= self.visible.len() {
            return;
        }
        let node_idx = self.visible[self.selected];
        let node = &self.nodes[node_idx];
        if !node.has_children {
            return;
        }

        // If this is an array/map container, toggle via window state
        if let Some(ref val) = node.value {
            let t = self.ctx.type_reader(val.type_idx);
            if matches!(t, mmapstruct_schema::TypeDefReader::Array { .. } | mmapstruct_schema::TypeDefReader::HashMap { .. }) {
                self.toggle_array_window(node_idx, *val);
                return;
            }
        }

        // If this is a child of an expanded array element, toggle via window state
        if let Some(mut ws) = self.window_state.take() {
            if let Some(local_idx) = self.find_window_child(node_idx, &ws) {
                if local_idx < ws.element_expanded.len() {
                    ws.element_expanded[local_idx] = !ws.element_expanded[local_idx];
                    self.window_state = Some(ws);
                    self.rebuild();
                    return;
                }
            }
            self.window_state = Some(ws);
        }

        // Normal struct/enum expand (non-window)
        let node = &mut self.nodes[node_idx];
        node.expanded = !node.expanded;
        let text = format!("{}", node.line);
        let new_text = if text.contains('▶') {
            text.replacen('▶', "▼", 1)
        } else {
            text.replacen('▼', "▶", 1)
        };
        node.line = ratatui::text::Line::from(new_text);
        self.visible = compute_visible(&self.nodes);
        self.ensure_selected_visible();
    }

    fn toggle_array_window(&mut self, _node_idx: usize, val: SchemaValue) {
        if let Some(ref ws) = self.window_state {
            if ws.matches_val(&val) && ws.expanded {
                self.window_state = None;
                self.rebuild();
                return;
            }
        }
        let t = self.ctx.type_reader(val.type_idx);
        let total = match t {
            mmapstruct_schema::TypeDefReader::Array { .. } => self.ctx.array_len(&val) as usize,
            mmapstruct_schema::TypeDefReader::HashMap { .. } => self.ctx.hashmap_len(&val),
            _ => 0,
        };
        let count = total.min(WINDOW_SIZE);
        self.window_state = Some(WindowState {
            val_offset: val.offset,
            val_type_idx: val.type_idx,
            total,
            window_start: 0,
            window_count: count,
            element_expanded: vec![false; count],
            expanded: true,
        });
        self.rebuild();
    }

    fn find_window_child(&self, node_idx: usize, ws: &WindowState) -> Option<usize> {
        let mut current = node_idx;
        loop {
            let node = &self.nodes[current];
            if let Some(p) = node.parent {
                let parent = &self.nodes[p];
                if let Some(ref val) = parent.value {
                    if ws.matches_val(val) {
                        return parent.children.iter().position(|&c| c == current);
                    }
                }
                current = p;
            } else {
                return None;
            }
        }
    }

    fn check_window_boundary(&mut self) {
        let Some(ws) = self.window_state.clone() else { return };
        if !ws.expanded || ws.total <= WINDOW_SIZE { return; }
        if self.selected >= self.visible.len() {
            return;
        }

        let node_idx = self.visible[self.selected];
        let element_local = self.find_window_child(node_idx, &ws);
        let Some(local_idx) = element_local else { return };
        let element_idx = ws.window_start + local_idx;

        if element_idx >= ws.window_start + MARGIN &&
           element_idx < ws.window_start + ws.window_count - MARGIN
        {
            return;
        }

        let saved = Some(element_idx);

        let new_start = if element_idx < ws.window_start + MARGIN {
            ws.window_start.saturating_sub(WINDOW_SIZE / 2)
        } else {
            (ws.window_start + WINDOW_SIZE / 2).min(ws.total.saturating_sub(WINDOW_SIZE))
        };
        let new_start = new_start.min(ws.total.saturating_sub(WINDOW_SIZE));
        let new_count = (ws.total - new_start).min(WINDOW_SIZE);

        self.window_state = Some(WindowState {
            element_expanded: vec![false; new_count],
            window_start: new_start,
            window_count: new_count,
            ..ws
        });
        self.rebuild_with_saved(saved);
    }

    /// Rebuild tree and restore a previously-saved conceptual element index.
    fn rebuild_with_saved(&mut self, saved: Option<usize>) {
        let nodes = tree::build_tree(
            self.ctx,
            &self.root_val,
            &self.root_name,
            self.depth,
            self.window_state.as_ref(),
        );
        self.nodes = nodes;
        self.visible = compute_visible(&self.nodes);

        self.selected = self.restore_conceptual_selected(saved)
            .unwrap_or(0)
            .min(self.visible.len().saturating_sub(1));
        let height: usize = 40;
        self.scroll = self.selected.saturating_sub(height / 2);
        self.ensure_selected_visible();
    }

    fn rebuild(&mut self) {
        let saved = self.save_conceptual_selected();
        self.rebuild_with_saved(saved);
    }

    fn save_conceptual_selected(&self) -> Option<usize> {
        if self.selected >= self.visible.len() {
            return None;
        }
        let ws = self.window_state.as_ref()?;
        if !ws.expanded {
            return None;
        }
        let node_idx = self.visible[self.selected];
        let local_idx = self.find_window_child(node_idx, ws)?;
        Some(ws.window_start + local_idx)
    }

    fn restore_conceptual_selected(&self, target: Option<usize>) -> Option<usize> {
        let element_idx = target?;
        let ws = self.window_state.as_ref()?;
        if !ws.expanded {
            return None;
        }
        let clamped = element_idx.clamp(ws.window_start, ws.window_start + ws.window_count - 1);
        let local_idx = clamped - ws.window_start;

        let array_idx = self.nodes.iter().position(|n| {
            n.value.as_ref().map_or(false, |v| ws.matches_val(v))
        })?;
        let element_node_idx = *self.nodes[array_idx].children.get(local_idx)?;
        self.visible.iter().position(|&v| v == element_node_idx)
    }

    fn ensure_selected_visible(&mut self) {
        let height: usize = 40;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height.saturating_sub(1) {
            self.scroll = self.selected.saturating_sub(height).saturating_add(1);
        }
    }
}

fn compute_visible(nodes: &[TreeNode]) -> Vec<usize> {
    let mut visible = Vec::with_capacity(nodes.len());
    let mut i = 0;
    while i < nodes.len() {
        visible.push(i);
        if !nodes[i].expanded {
            i = skip_subtree(nodes, i);
        } else {
            i += 1;
        }
    }
    visible
}

fn skip_subtree(nodes: &[TreeNode], idx: usize) -> usize {
    let depth = nodes[idx].depth;
    let mut i = idx + 1;
    while i < nodes.len() && nodes[i].depth > depth {
        i += 1;
    }
    i
}
