use crate::debug_log;
use crate::joiner::{BottomUpJoiner, TopDownJoiner};
use crate::overscan::Overscan;
use crate::path::{Path, PathSegment, PathShift};
use crate::scanner::Scanner;
use crate::search::SearchState;
use crate::view_node;
use crate::viewport::Viewport;
use mmapstruct_schema::schema::TypeDefReader;
use mmapstruct_schema::SchemaValue;
use std::collections::HashSet;

/// Application state built on `Overscan` + `Viewport`.
pub struct App {
    pub overscan: Overscan,
    pub viewport: Viewport,
    pub search: SearchState,
    pub search_mode: bool,
    pub search_buffer: String,
    pub search_message: Option<&'static str>,
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
            search: SearchState::new(),
            search_mode: false,
            search_buffer: String::new(),
            search_message: None,
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

        // If the collapsed subtree extends to the end of loaded nodes,
        // there are no visible siblings after it — continue scanning
        // to load them.
        if self.overscan.skip_loaded_children(idx) >= self.overscan.nodes.len() {
            self.overscan.fill_after_collapse(idx, OVERSCAN_STEPS);
        }
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

    /// Navigate one page up. Moves selection up by one display height.
    /// Loads a prior overscan segment via BottomUpJoiner if needed.
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

    /// Navigate one page down. Moves selection down by one display height.
    /// Loads a subsequent overscan segment via TopDownJoiner if needed.
    pub fn navigate_page_down(&mut self) {
        let page_size = self.viewport.display_height;
        let (target, remaining) = self.overscan.nth_visible_forward(self.viewport.selected, page_size);
        if remaining == 0 {
            self.viewport.selected = target;
            self.ensure_selected_visible();
        } else if self.overscan.effective_end().is_some() {
            let overlap_lines = self.term_height * EXTENDED_WINDOW_MULTIPLIER / 3;
            let (overscan, viewport) =
                TopDownJoiner::join(
                    &self.overscan, overlap_lines, remaining, page_size
                );
            self.overscan = overscan;
            self.viewport = viewport;
        } else if let Some(last) = self.overscan.last_visible() {
            self.viewport.selected = last;
            self.ensure_selected_visible();
        }
    }

    /// Try fast-path: pattern `field:number` jumps directly to array element
    /// at that index and verifies the field value matches.  Returns `true`
    /// if a match was found and the overscan was rebuilt.
    fn try_fast_path(&mut self, fp: &crate::search::FastPath) -> bool {
        let ctx = self.overscan.scanner.ctx;
        let (arr, prefix_to_arr_parent) = match self.locate_array_for_fastpath() {
            Some(v) => v,
            None => return false,
        };

        let len = ctx.array_len(&arr);
        if fp.target >= len {
            return false;
        }

        // Verify field value at index
        let elem = match ctx.array_get(&arr, fp.target) {
            Some(e) => e,
            None => return false,
        };
        let fv = match ctx.field_get(&elem, &fp.field) {
            Some(v) => v,
            None => return false,
        };
        if ctx.read_as_i64(&fv) != Some(fp.target as i64) {
            return false;
        }

        // Build path: prefix_to_arr_parent + (array, Entry(target))
        let mut path = prefix_to_arr_parent;
        path.push(PathSegment {
            val: arr,
            shift: PathShift::Entry(fp.target as usize),
        });
        let new_scanner = Scanner::from_path(ctx, path, OVERSCAN_STEPS);
        self.overscan = Overscan::from_next(&self.overscan, new_scanner);
        self.viewport.selected = 0;
        self.viewport.scroll = 0;
        self.search.match_idx = Some(0);
        self.search_message = None;
        true
    }

    /// Find the nearest array ancestor of the current position.
    ///
    /// Uses `path_of(selected)` which always reflects the user's actual
    /// position — works on first entry into an expanded array, after
    /// fast-path jumps, and after navigating to a different array.
    ///
    /// Returns `(array_value, prefix_to_arrays_parent)` — a path that stops
    /// **before** the array (the push of the array segment is done by the
    /// caller in `try_fast_path`).
    fn locate_array_for_fastpath(&self) -> Option<(SchemaValue, Path)> {
        let ctx = self.overscan.scanner.ctx;
        let cur_path = self.overscan.path_of(self.viewport.selected);
        let pos = cur_path.iter().rposition(|seg| {
            matches!(ctx.type_reader(seg.val.type_idx), TypeDefReader::Array { .. })
        })?;
        let arr = cur_path[pos].val;
        let prefix = Path(cur_path[..pos].to_vec());
        Some((arr, prefix))
    }

    /// Execute a text search from the current selection + 1.
    /// First tries fast-path (`field:number`), then text search.
    pub fn execute_search(&mut self) {
        let pattern = self.search.pattern.clone();
        if pattern.is_empty() {
            return;
        }

        // Fast-path: /id:42 → direct jump
        if let Some(fp) = self.search.try_fast_path() {
            if self.try_fast_path(&fp) {
                return;
            }
        }

        // Phase 1: search within currently loaded nodes
        let start = self.viewport.selected + 1;
        for i in start..self.overscan.nodes.len() {
            if self.node_matches_search(&self.overscan.nodes[i], i, &pattern) {
                self.jump_to_match(i);
                self.search_message = None;
                return;
            }
        }

        // Phase 2: scan forward — always descend into all containers
        if !self.scan_forward(&pattern) {
            self.search_message = Some("Pattern not found");
        }
    }

    /// Move to the next match (forward from current selection + 1).
    pub fn search_next(&mut self) {
        let pattern = self.search.pattern.clone();
        if pattern.is_empty() {
            return;
        }

        // Search forward from selected + 1 in current nodes
        let start = self.viewport.selected + 1;
        for i in start..self.overscan.nodes.len() {
            if self.node_matches_search(&self.overscan.nodes[i], i, &pattern) {
                self.jump_to_match(i);
                self.search_message = None;
                return;
            }
        }

        // Phase 2: scan forward
        if !self.scan_forward(&pattern) {
            self.search_message = Some("Pattern not found");
        }
    }

    /// Scan forward from the next available path, descending into all
    /// containers (`should_step_in = true`).  Returns `true` if a match
    /// was found and the overscan was rebuilt.
    fn scan_forward(&mut self, pattern: &str) -> bool {
        let ctx = self.overscan.scanner.ctx;

        // Determine starting path: effective_end() first, then last node
        let start_path = match self.overscan.effective_end() {
            Some(p) => p,
            None => {
                let last = self.overscan.nodes.len().saturating_sub(1);
                self.overscan.path_of(last)
            }
        };

        let mut scanner = Scanner::from_path(ctx, start_path, OVERSCAN_STEPS);
        for _ in 0..100 {
            let (match_path, cont) = scanner.find_text_advance(pattern, |_| true);
            if let Some(mp) = match_path {
                let new_scanner = Scanner::from_path(ctx, mp, OVERSCAN_STEPS);
                self.overscan = Overscan::from_next(&self.overscan, new_scanner);
                self.viewport.selected = 0;
                self.viewport.scroll = 0;
                self.search.match_idx = Some(0);
                return true;
            }
            match cont {
                Some(next_path) => {
                    scanner = Scanner::from_path(ctx, next_path, OVERSCAN_STEPS);
                }
                None => return false,
            }
        }
        false
    }

    /// Check if a loaded `ViewNode` matches a search string.
    /// Uses `display_text` for leaves, preview text for containers.
    fn node_matches_search(&self, node: &view_node::ViewNode, _idx: usize, pat: &str) -> bool {
        if node.display_text.contains(pat) {
            return true;
        }
        // For containers, also check preview text (matches what the user sees)
        if node.has_children {
            let parent = node.parent_val.map(|pv| (pv, node.shift.map(|s| s.index()).unwrap_or(0)));
            if let Some(preview) = self.overscan.scanner.format_preview_text(&node.value, parent, 200) {
                if preview.contains(pat) {
                    return true;
                }
            }
        }
        false
    }

    fn jump_to_match(&mut self, idx: usize) {
        self.search.match_idx = Some(idx);
        self.viewport.selected = idx;
        self.ensure_selected_visible();
    }

    /// Navigate to the first visible line. If already at the beginning of
    /// the current overscan, jumps to viewport top. Otherwise rebuilds the
    /// overscan from root via `from_next`, preserving expansion state.
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

    /// Navigate to the last visible line. Uses `cursor.end` or
    /// `effective_end_stack` to locate the data boundary, then finds the
    /// structural last visible element via `last_visible_path` and builds
    /// a new overscan from `OVERSCAN_STEPS` lines before it.
    /// If no end marker exists (all data fits in one overscan), positions
    /// the viewport at `last_visible()` directly.
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
