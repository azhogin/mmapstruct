use crate::debug_log;
use crate::joiner::{BottomUpJoiner, TopDownJoiner};
use crate::overscan::Overscan;
use crate::scanner::Scanner;
use crate::viewport::Viewport;
use mmapstruct_schema::SchemaValue;
use std::collections::HashSet;

/// Application state built on `Overscan` + `Viewport`.
pub struct App {
    pub overscan: Overscan,
    pub viewport: Viewport,
    pub should_quit: bool,
    pub term_height: usize,
}

const EXPAND_LIMIT: usize = 100;
pub(crate) const OVERSCAN_STEPS: usize = 500;
const EXTENDED_WINDOW_MULTIPLIER: usize = 10;

impl App {
    pub fn new(overscan: Overscan, term_height: usize) -> Self {
        App {
            overscan,
            viewport: Viewport {
                selected: 0,
                scroll: 0,
                display_height: term_height.saturating_sub(1),
            },
            should_quit: false,
            term_height,
        }
    }

    pub fn handle_resize(&mut self, h: usize) {
        self.term_height = h;
        self.viewport.display_height = h.saturating_sub(1);
    }

    /// Expand the currently selected node (if collapsable).
    pub fn expand_selected(&mut self) {
        let idx = self.viewport.selected;
        if idx >= self.overscan.nodes.len() {
            return;
        }
        let has_children = self.overscan.nodes[idx].has_children;
        let expanded = self.overscan.nodes[idx].expanded;
        if !has_children || expanded {
            return;
        }
        let inserted = self.overscan.expand_node(idx, EXPAND_LIMIT);
        self.viewport.fixup_for_insertion(idx, inserted);
    }

    /// Collapse the currently selected node.
    pub fn collapse_selected(&mut self) {
        let idx = self.viewport.selected;
        if idx >= self.overscan.nodes.len() {
            return;
        }
        let has_children = self.overscan.nodes[idx].has_children;
        let expanded = self.overscan.nodes[idx].expanded;
        if !has_children || !expanded {
            return;
        }
        self.overscan.collapse_node(idx);
    }

    /// Navigate one line up.
    pub fn navigate_up(&mut self) {
        if let Some(prev) = self.overscan.prev_visible(self.viewport.selected) {
            self.viewport.selected = prev;
            self.ensure_selected_visible();
        } else if self.overscan.scanner.path.is_file_begin() {
            // Already at file beginning — nothing to do
        } else {
            let overlap_lines = self.term_height * EXTENDED_WINDOW_MULTIPLIER / 3;
            let (overscan, viewport) =
                BottomUpJoiner::join(
                    &self.overscan, overlap_lines, 1, self.viewport.display_height
                );
            self.overscan = overscan;
            self.viewport = viewport;
        }
    }

    /// Navigate one line down.
    pub fn navigate_down(&mut self) {
        if let Some(next) = self.overscan.next_visible(self.viewport.selected) {
            self.viewport.selected = next;
            self.ensure_selected_visible();
        } else {
            if self.overscan.effective_end().is_some() {
                let overlap_lines = self.term_height * EXTENDED_WINDOW_MULTIPLIER / 3;
                let (overscan, viewport) =
                    TopDownJoiner::join(
                        &self.overscan, overlap_lines, 1, self.viewport.display_height
                    );
                self.overscan = overscan;
                self.viewport = viewport;
            }
            // TODO: else we need to move cursor to the file end (?)
        }
    }

    fn ensure_selected_visible(&mut self) {
        let h = self.viewport.display_height;
        let vis = self.overscan.visible_line_count_up_to(self.viewport.selected);
        let vis_idx = vis.saturating_sub(1);
        if vis_idx < self.viewport.scroll {
            self.viewport.scroll = vis_idx;
        } else if vis_idx >= self.viewport.scroll + h {
            self.viewport.scroll = vis_idx.saturating_sub(h).saturating_add(1);
        }
    }

    pub fn navigate_page_up(&mut self) {
        let page_size = self.viewport.display_height;
        let (target, remaining) = self.overscan.nth_visible_backward(self.viewport.selected, page_size);
        if remaining == 0 {
            self.viewport.selected = target;
            self.ensure_selected_visible();
        } else if self.overscan.scanner.path.is_file_begin() {
            self.viewport.selected = 0;
            self.viewport.scroll = 0;
        } else {
            let overlap_lines = self.term_height * EXTENDED_WINDOW_MULTIPLIER / 3;
            let (overscan, viewport) =
                BottomUpJoiner::join(
                    &self.overscan, overlap_lines, remaining, page_size
                );
            self.overscan = overscan;
            self.viewport = viewport;
        }
    }

    pub fn navigate_page_down(&mut self) {
        let page_size = self.viewport.display_height;
        let (target, remaining) = self.overscan.nth_visible_forward(self.viewport.selected, page_size);
        if remaining == 0 {
            self.viewport.selected = target;
            self.ensure_selected_visible();
        } else {
            if self.overscan.effective_end().is_some() {
                let overlap_lines = self.term_height * EXTENDED_WINDOW_MULTIPLIER / 3;
                let (overscan, viewport) =
                    TopDownJoiner::join(
                        &self.overscan, overlap_lines, remaining, page_size
                    );
                self.overscan = overscan;
                self.viewport = viewport;
            }
            // TODO: else we need to move cursor to the file end (?)
        }
    }

    pub fn navigate_home(&mut self) {
        if self.overscan.scanner.path.is_file_begin() {
            self.viewport.selected = 0;
            self.viewport.scroll = 0;
            return;
        }

        let ctx = self.overscan.scanner.ctx;
        let root_val = self.overscan.scanner.path[0].val;
        let new_scanner = Scanner::new(ctx, root_val, OVERSCAN_STEPS);

        self.overscan = Overscan::from_next(&self.overscan, new_scanner);
        self.viewport.selected = 0;
        self.viewport.scroll = 0;
    }

    pub fn navigate_end(&mut self) {
        let end_path = match self.overscan.cursor.end.clone()
            .or_else(|| self.overscan.cursor.effective_end_stack.last().cloned())
        {
            Some(p) => p,
            None => {
                debug_log!("navigate_end — cursor.end and effective_end_stack both empty, using last_visible fallback");
                if let Some(last) = self.overscan.last_visible() {
                    let vis = self.overscan.visible_line_count_up_to(last);
                    let vis_idx = vis.saturating_sub(1);
                    self.viewport.selected = last;
                    self.viewport.scroll = vis_idx.saturating_sub(self.viewport.display_height - 1);
                }
                return;
            }
        };

        debug_log!("navigate_end — cursor.end: {:?}", end_path);

        let ctx = self.overscan.scanner.ctx;

        // Pre-collect expanded-from-path and expanded-from-nodes
        // to avoid borrowing self.overscan inside the closure.
        let expanded_from_path: HashSet<(u64, u32)> = end_path
            .iter()
            .map(|seg| (seg.val.mmap_begin, seg.val.type_idx))
            .collect();
        let expanded_from_nodes: HashSet<(u64, u32)> = self
            .overscan
            .nodes
            .iter()
            .filter(|n| n.expanded)
            .map(|n| (n.value.mmap_begin, n.value.type_idx))
            .collect();

        debug_log!(
            "  expanded_from_path entries: {:?}",
            expanded_from_path.len()
        );
        debug_log!(
            "  expanded_from_nodes entries: {:?}",
            expanded_from_nodes.len()
        );

        let should_step_in = move |val: SchemaValue| -> bool {
            expanded_from_path.contains(&(val.mmap_begin, val.type_idx))
                || expanded_from_nodes.contains(&(val.mmap_begin, val.type_idx))
        };

        // Find the last visible element given current expansion state
        let last_path = match self.overscan.scanner.last_visible_path(&should_step_in) {
            Some(p) => p,
            None => {
                debug_log!("  last_visible_path returned None — fallback");
                if let Some(last) = self.overscan.last_visible() {
                    let vis = self.overscan.visible_line_count_up_to(last);
                    let vis_idx = vis.saturating_sub(1);
                    self.viewport.selected = last;
                    self.viewport.scroll = vis_idx.saturating_sub(self.viewport.display_height - 1);
                }
                return;
            }
        };

        debug_log!("  last_path: {:?}", last_path);

        // Step back from last_path by OVERSCAN_STEPS - 1
        let back_scanner = Scanner::from_path(ctx, last_path.clone(), 0);
        let (scan_start, steps) = back_scanner.do_steps_back(
            &last_path,
            OVERSCAN_STEPS - 1,
            &should_step_in,
        );

        debug_log!("  scan_start: {:?}  steps_back: {}", scan_start, steps);

        // Build new Overscan from scan_start forward with the same expansion
        let new_scanner = Scanner::from_path(ctx, scan_start, OVERSCAN_STEPS);
        self.overscan = Overscan::build(new_scanner, should_step_in);

        debug_log!(
            "  post-build — cursor.end: {:?}  effective_end_stack: {:?}  nodes: {}",
            self.overscan.cursor.end,
            self.overscan.cursor.effective_end_stack,
            self.overscan.nodes.len(),
        );

        // Position viewport at the last visible element
        let sel = self.overscan.last_visible().unwrap_or(0);
        let vis = self.overscan.visible_line_count_up_to(sel);
        let vis_idx = vis.saturating_sub(1);
        self.viewport.selected = sel;
        self.viewport.scroll = vis_idx.saturating_sub(self.viewport.display_height - 1);
        debug_log!("  done — selected: {}  scroll: {}", sel, self.viewport.scroll);
    }
}
