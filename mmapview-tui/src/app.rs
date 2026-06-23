use crate::tree::TreeNode;

/// Application state for the TUI.
pub struct App {
    /// All tree nodes (flat list, parent/child indices for navigation).
    pub nodes: Vec<TreeNode>,
    /// Indices into `nodes` that are currently visible (all ancestors expanded).
    pub visible: Vec<usize>,
    /// Currently selected row (index into `visible`).
    pub selected: usize,
    /// Vertical scroll offset.
    pub scroll: usize,
    /// Max expand depth.
    pub depth: usize,
    /// Whether to quit.
    pub should_quit: bool,
}

impl App {
    pub fn new(nodes: Vec<TreeNode>, depth: usize) -> Self {
        let visible = compute_visible(&nodes);
        App {
            nodes,
            visible,
            selected: 0,
            scroll: 0,
            depth,
            should_quit: false,
        }
    }

    /// Move selection up by one.
    pub fn navigate_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
        self.ensure_selected_visible();
    }

    /// Move selection down by one.
    pub fn navigate_down(&mut self) {
        if self.selected + 1 < self.visible.len() {
            self.selected += 1;
        }
        self.ensure_selected_visible();
    }

    /// Move selection up by one page.
    pub fn navigate_page_up(&mut self) {
        let page_size: usize = 20;
        if self.selected >= page_size {
            self.selected -= page_size;
        } else {
            self.selected = 0;
        }
        self.ensure_selected_visible();
    }

    /// Move selection down by one page.
    pub fn navigate_page_down(&mut self) {
        let page_size: usize = 20;
        let last = self.visible.len().saturating_sub(1);
        self.selected = (self.selected + page_size).min(last);
        self.ensure_selected_visible();
    }

    /// Toggle expand/collapse of the currently selected node.
    pub fn toggle_expand(&mut self) {
        if self.selected >= self.visible.len() {
            return;
        }
        let node_idx = self.visible[self.selected];
        let node = &mut self.nodes[node_idx];
        if !node.has_children {
            return;
        }
        node.expanded = !node.expanded;
        // Swap the expand indicator in the display line.
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

    fn ensure_selected_visible(&mut self) {
        let height: usize = 40;
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height.saturating_sub(1) {
            self.scroll = self.selected.saturating_sub(height).saturating_add(1);
        }
    }
}

/// Compute the list of visible node indices (all ancestors expanded).
fn compute_visible(nodes: &[TreeNode]) -> Vec<usize> {
    let mut visible = Vec::new();
    let mut i = 0;
    while i < nodes.len() {
        visible.push(i);
        if !nodes[i].expanded {
            // skip children
            i = skip_subtree(nodes, i);
        } else {
            i += 1;
        }
    }
    visible
}

/// Given a node index, return the index of the next node outside its subtree.
fn skip_subtree(nodes: &[TreeNode], idx: usize) -> usize {
    let depth = nodes[idx].depth;
    let mut i = idx + 1;
    while i < nodes.len() && nodes[i].depth > depth {
        i += 1;
    }
    i
}
