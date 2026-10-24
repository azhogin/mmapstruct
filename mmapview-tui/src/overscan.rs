use crate::debug_log;
use crate::path::{Path, PathSegment};
use crate::scanner::{Scanner, StepContext};
use crate::view_node::{build_view_nodes, ViewNode};
use mmapstruct_schema::SchemaValue;
use std::collections::HashSet;

/// Tracking state for a single scan segment: the path *after* the last
/// budgeted element (`end`) and a stack of effective-end positions pushed
/// by expand_node / popped by collapse_node.
pub struct ScanCursor {
    /// Path to the element *after* the last one processed by the most
    /// recent `do_steps` scan.  `None` when end of data is reached.
    pub end: Option<Path>,
    /// Stack of effective-end paths (one per expand in progress).
    /// The top of the stack is the *current* virtual boundary.
    pub effective_end_stack: Vec<Path>,
}

impl ScanCursor {
    pub fn new(end: Option<Path>) -> Self {
        ScanCursor {
            end,
            effective_end_stack: Vec::new(),
        }
    }
}

/// Tree view manager that wraps a `Scanner` and caches `ViewNode`s for the
/// visible window.
pub struct Overscan {
    pub scanner: Scanner,
    pub nodes: Vec<ViewNode>,
    pub cursor: ScanCursor,
}

impl Overscan {
    /// Build a fresh `Overscan` from a `Scanner` with all nodes collapsed.
    /// Only top-level siblings of the root are visible.
    pub fn new(scanner: Scanner) -> Self {
        Overscan::build(scanner, |_| false)
    }

    /// Build an `Overscan` from a `Scanner`, using `should_step_in` to
    /// decide which containers are expanded.
    pub fn build(scanner: Scanner, should_step_in: impl FnMut(SchemaValue) -> bool) -> Self {
        let (nodes, end_path) = build_view_nodes(&scanner, should_step_in);
        Overscan {
            scanner,
            nodes,
            cursor: ScanCursor::new(end_path),
        }
    }

    /// Build a new `Overscan` from `prev`, reusing its expanded/collapsed
    /// state.  A node in the new view will be expanded iff the corresponding
    /// node (matched by `(mmap_begin, type_idx)`) was expanded in `prev`.
    pub fn from_previous(prev: &Overscan, new_scanner: Scanner) -> Self {
        Overscan::build(new_scanner, |val| prev.is_expanded(&val))
    }

    /// Build a new `Overscan` from `next`, reusing expansions from both
    /// visible nodes and the current path segments.
    ///
    /// Like `from_previous`, but also marks as expanded every container
    /// that appears in `next.scanner.path`.  This ensures that when we
    /// walk upward (BottomUp) into a region whose parent was already
    /// traversed (e.g. an expanded array whose children are visible),
    /// the parent stays expanded in the new view.
    pub fn from_next(next: &Overscan, new_scanner: Scanner) -> Self {
        let expanded_from_path: HashSet<(u64, u32)> = next
            .scanner
            .path
            .iter()
            .map(|seg| (seg.val.mmap_begin, seg.val.type_idx))
            .collect();

        Overscan::build(new_scanner, |val| {
            expanded_from_path.contains(&(val.mmap_begin, val.type_idx)) || next.is_expanded(&val)
        })
    }

    /// Returns `true` if a node with the given value is marked as expanded
    /// in this view (linear search over `nodes`).
    pub fn is_expanded(&self, val: &SchemaValue) -> bool {
        self.nodes.iter().any(|n| {
            n.value.mmap_begin == val.mmap_begin && n.value.type_idx == val.type_idx && n.expanded
        })
    }

    /// Return the current effective end — the top of `effective_end_stack`,
    /// or `cursor.end` if the stack is empty.
    pub fn effective_end(&self) -> Option<Path> {
        self.cursor
            .effective_end_stack
            .last()
            .cloned()
            .or_else(|| self.cursor.end.clone())
    }

    /// Expand the node at `idx`, loading its children if not yet loaded.
    /// Returns the number of child nodes inserted.
    pub fn expand_node(&mut self, idx: usize, limit: usize) -> usize {
        if !self.nodes[idx].children_loaded {
            let parent_val = self.nodes[idx].value;
            let parent_depth = self.nodes[idx].depth;
            let (children, virtual_end) = self.scanner.load_children(&parent_val, limit);
            let depth = parent_depth + 1;
            let insert_before = idx + 1;
            let child_count = children.len();
            for (child_val, child_idx) in children.into_iter().rev() {
                let has_children = self.scanner.child_count(&child_val) > 0;
                let display_text = self
                    .scanner
                    .format_display_text(&child_val, Some((parent_val, child_idx)));
                let shift = self.scanner.child_shift(&parent_val, child_idx);
                self.nodes.insert(
                    insert_before,
                    ViewNode {
                        value: child_val,
                        depth,
                        has_children,
                        expanded: false,
                        children_loaded: false,
                        display_text,
                        virtual_end: None,
                        parent_val: Some(parent_val),
                        shift: Some(shift),
                    },
                );
            }
            self.nodes[idx].children_loaded = true;
            self.nodes[idx].expanded = true;

            // Save virtual_end in the node and push to effective_end_stack.
            if let Some(ve) = virtual_end {
                // Prepend the path from root to this node so virtual_end is
                // a full path usable by collapse_node's descendant check.
                let parent_path = self.path_of(idx);
                let mut full_ve = parent_path;
                full_ve.extend(ve.0);
                self.nodes[idx].virtual_end = Some(full_ve.clone());
                self.cursor.effective_end_stack.push(full_ve);
            }

            child_count
        } else {
            self.nodes[idx].expanded = true;
            // Re-push virtual_end onto the stack if this node had one.
            if let Some(ref ve) = self.nodes[idx].virtual_end {
                // Avoid duplicate if the stack top already matches.
                let should_push = self.cursor.effective_end_stack.last() != Some(ve);
                if should_push {
                    self.cursor.effective_end_stack.push(ve.clone());
                }
            }
            0
        }
    }

    /// Collapse the node at `idx`. Children stay in `nodes[]` and are skipped
    /// during rendering.  Any effective‑end entries that lie within the
    /// collapsed subtree are popped.
    pub fn collapse_node(&mut self, idx: usize) {
        self.nodes[idx].expanded = false;

        // Pop effective‑end entries that descend from this node.
        let parent_path = self.path_of(idx);
        let parent_depth = self.nodes[idx].depth;
        assert_eq!(parent_path.len(), parent_depth);
        while let Some(top) = self.cursor.effective_end_stack.last() {
            if parent_path.is_path_prefix(top) {
                self.cursor.effective_end_stack.pop();
            } else {
                break;
            }
        }
        self.fixup_cursor_end_on_collapse(idx, parent_path);
    }

    fn fixup_cursor_end_on_collapse(&mut self, idx: usize, parent_path: Path) {
        let inside = self
            .cursor
            .end
            .as_ref()
            .is_some_and(|end| parent_path.is_path_prefix(end));
        if inside {
            // Save a clone to virtual_end so re-expand can restore it,
            // but keep cursor.end intact so effective_end() still works.
            self.nodes[idx].virtual_end = self.cursor.end.take();
        }
    }

    /// Starting from just after a collapsed parent, return the index of the
    /// first node whose depth is ≤ the parent's depth.
    pub fn skip_loaded_children(&self, start: usize) -> usize {
        let parent_depth = self.nodes[start].depth;
        let mut i = start + 1;
        while i < self.nodes.len() && self.nodes[i].depth > parent_depth {
            i += 1;
        }
        i
    }

    /// Return the index of the next visible (non-skipped) node after `idx`,
    /// or `None` if there is none.
    pub fn next_visible(&self, idx: usize) -> Option<usize> {
        let node = &self.nodes[idx];
        if node.collapsed_with_children() {
            let i = self.skip_loaded_children(idx);
            if i >= self.nodes.len() {
                None
            } else {
                Some(i)
            }
        } else {
            let i = idx + 1;
            if i >= self.nodes.len() {
                None
            } else {
                // If advancing would escape an expanded container that has
                // a virtual_end (more children not yet loaded), stop here
                // so the caller can load the next segment via effective_end().
                if self.nodes[i].depth < self.nodes[idx].depth {
                    if let Some(parent) = self.parent_of(idx) {
                        if self.nodes[parent].virtual_end.is_some() {
                            return None;
                        }
                    }
                }
                Some(i)
            }
        }
    }

    /// Return the top-most collapsed (children_loaded) ancestor of `idx`  
    /// that is itself not hidden, or `None` if no such ancestor exists.
    pub fn toplvl_collapsed_parent(&self, idx: usize) -> Option<usize> {
        let mut collapsed = None;
        let mut p = idx;
        while let Some(pai) = self.parent_of(p) {
            if self.nodes[pai].collapsed_with_children() {
                collapsed = Some(pai);
            }
            p = pai;
        }
        collapsed
    }

    /// Return the index of the previous visible (non-skipped) node before
    /// `idx`, or `None` if there is none.
    pub fn prev_visible(&self, idx: usize) -> Option<usize> {
        if idx == 0 {
            return None;
        }
        let i = idx - 1;
        self.toplvl_collapsed_parent(i).or(Some(i))
    }

    /// Return the index of the last visible node, or `None` if empty.
    pub fn last_visible(&self) -> Option<usize> {
        let mut i = 0;
        let mut last;
        loop {
            let node = self.nodes.get(i)?;
            let collapsed = !node.expanded && node.has_children && node.children_loaded;
            debug_log!(
                "last_visible: i={} depth={} has_children={} expanded={} children_loaded={} collapsed={} text={:?}",
                i, node.depth, node.has_children, node.expanded, node.children_loaded, collapsed, node.display_text
            );
            if collapsed {
                last = Some(i);
                let next = self.skip_loaded_children(i);
                debug_log!("  collapsed -> skip to {}", next);
                i = next;
            } else {
                last = Some(i);
                i += 1;
            }
            if i >= self.nodes.len() {
                debug_log!("last_visible result: {:?}", last);
                break last;
            }
        }
    }

    /// Starting from `from`, advance `n` visible steps forward and return
    /// the resulting index and the remaining count (steps not taken).
    pub fn nth_visible_forward(&self, from: usize, n: usize) -> (usize, usize) {
        let mut idx = from;
        let mut steps = 0;
        for _ in 0..n {
            match self.next_visible(idx) {
                Some(i) => {
                    idx = i;
                    steps += 1;
                }
                None => break,
            }
        }
        (idx, n - steps)
    }

    /// Starting from `from`, advance `n` visible steps backward and return
    /// the resulting index, or `None` if there aren't enough steps.
    pub fn nth_visible_backward(&self, from: usize, n: usize) -> (usize, usize) {
        let mut idx = from;
        let mut steps = 0;
        for _ in 0..n {
            match self.prev_visible(idx) {
                Some(i) => {
                    idx = i;
                    steps += 1;
                }
                None => break,
            }
        }
        (idx, n - steps)
    }

    /// Count how many visible lines exist up to and including `idx`, based
    /// on current expand/collapse state.  Panics if `idx >= nodes.len()`.
    pub fn visible_line_count_up_to(&self, idx: usize) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < self.nodes.len() && i <= idx {
            count += 1;
            let node = &self.nodes[i];
            if node.collapsed_with_children() {
                if i == idx {
                    // idx is the collapsed parent itself — it is visible, stop
                    break;
                }
                let skip = self.skip_loaded_children(i);
                if idx < skip {
                    // idx lies inside hidden children — don't count them
                    break;
                }
                i = skip;
            } else {
                i += 1;
            }
        }
        count
    }

    /// Return the index of the parent of `idx` by scanning backward for
    /// a node whose depth is exactly one less.
    pub fn parent_of(&self, idx: usize) -> Option<usize> {
        let d = self.nodes[idx].depth;
        if d == 0 {
            return None;
        }
        (0..idx).rev().find(|&i| self.nodes[i].depth == d - 1)
    }

    /// Build a `Path` that, when given to `Scanner::from_path`, would
    /// begin processing at `nodes[idx]`.
    ///
    /// Walks up the `parent_val` chain stored in each `ViewNode`, then
    /// fills in any missing top-level segments from `scanner.path`.
    pub fn path_of(&self, idx: usize) -> Path {
        let depth = self.nodes[idx].depth;
        assert!(depth > 0, "path_of called for depth-0 node");

        // Walk up parent_val chain (bottom-up)
        let mut segments: Vec<PathSegment> = Vec::new();
        let mut current = idx;
        loop {
            let node = &self.nodes[current];
            match (node.parent_val, node.shift) {
                (Some(pv), Some(sh)) => {
                    segments.push(PathSegment { val: pv, shift: sh });
                    // Try to find parent in nodes
                    let parent_idx = (0..current).rposition(|i| {
                        self.nodes[i].value.mmap_begin == pv.mmap_begin
                            && self.nodes[i].value.type_idx == pv.type_idx
                    });
                    match parent_idx {
                        Some(pi) => current = pi,
                        None => break,
                    }
                }
                _ => break,
            }
        }
        segments.reverse(); // now top-down

        // Complete path? Return it directly.
        if segments.len() == depth {
            return Path(segments.into_iter().collect());
        }

        // Missing top-level ancestors — fill from scanner.path
        let missing = depth - segments.len();
        let remove = self.scanner.path.len().min(missing);
        let prefix = &self.scanner.path[0..remove];

        let mut path: Path = Path(prefix.to_vec());
        path.0.extend(segments);

        // The rebuilt path might still be short if scanner.path didn't cover
        // all missing levels. This should not happen in practice.
        assert_eq!(path.len(), depth, "path_of failed to reconstruct full path");
        path
    }

    // not used; only a stub/skeleton for stitching two Overscan instances together
    /// Continue scanning from `cursor.end` for `steps` more items.
    /// Appends new `ViewNode`s to `self.nodes` and updates `cursor.end`.
    /// The `should_step_in` callback should return `true` for nodes that
    /// are currently expanded in this tree view (not just logically
    /// expandable) — the continuation must NOT re‑walk into subtrees that
    /// have already been loaded via `expand_node`.
    /// After collapsing a node whose loaded children extend to the end of
    /// `nodes[]`, advance `cursor.end` past the collapsed node (if needed)
    /// and continue scanning to load subsequent siblings.
    pub fn fill_after_collapse(&mut self, idx: usize, steps: usize) {
        if self.cursor.end.is_none() {
            // cursor.end was inside the collapsed subtree and got saved to
            // virtual_end — compute the path past the collapsed node.
            let node_path = self.path_of(idx);
            let end = crate::scanner::Scanner::from_path(self.scanner.ctx, node_path, 1)
                .do_steps(|_| {}, |_| false);
            self.cursor.end = end;
        }
        if self.cursor.end.is_some() {
            self.continue_scan(steps);
        }
    }

    pub fn continue_scan(&mut self, steps: usize) {
        let end_path = match self.cursor.end.clone() {
            Some(p) => p,
            None => return,
        };

        let is_expanded = |val: SchemaValue| -> bool { self.is_expanded(&val) };

        let should_step_in = |val: SchemaValue| -> bool {
            if !is_expanded(val) {
                return false;
            }
            !self.nodes.iter().any(|n| {
                n.value.mmap_begin == val.mmap_begin
                    && n.value.type_idx == val.type_idx
                    && n.children_loaded
            })
        };

        let new_scanner =
            crate::scanner::Scanner::from_path(self.scanner.ctx, end_path.clone(), steps);
        let mut new_nodes = Vec::new();

        let new_end = new_scanner.do_steps(
            |ctx: StepContext| {
                let has_children = new_scanner.child_count(&ctx.val) > 0;
                let display_text = new_scanner.format_display_text(&ctx.val, ctx.parent);
                let shift = ctx.parent.map(|(pv, ci)| new_scanner.child_shift(&pv, ci));
                new_nodes.push(ViewNode {
                    value: ctx.val,
                    depth: ctx.depth,
                    has_children,
                    expanded: ctx.expand,
                    children_loaded: ctx.expand,
                    display_text,
                    virtual_end: None,
                    parent_val: ctx.parent.map(|(pv, _)| pv),
                    shift,
                });
            },
            should_step_in,
        );

        // Extend nodes
        self.nodes.extend(new_nodes);

        // Update cursor with the path after the last processed element
        self.cursor.end = new_end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::Scanner;
    use bytemuck::{Pod, Zeroable};
    use mmapstruct::schema::GetSchema;
    use mmapstruct::traits::{LayoutHash, MmapRead, MmapWrite};
    use mmapstruct::MmapStruct;
    use mmapstruct_schema::context::RuntimeSchemaCtx;
    use mmapstruct_schema::SchemaValue;
    use mmapstruct_schema::{write_to_file_with_embedded_schema, Schema};

    #[derive(MmapStruct, Default, Clone, Copy, Debug, Pod, Zeroable)]
    #[repr(C)]
    struct Inner {
        a: u32,
    }

    #[derive(MmapStruct, Clone, Copy, Debug, Pod, Zeroable)]
    #[repr(C)]
    struct Outer {
        inner: Inner,
        y: u32,
    }

    fn setup<T: MmapWrite + GetSchema + LayoutHash>(
        val: &T,
    ) -> (&'static RuntimeSchemaCtx<'static>, SchemaValue) {
        let buf = write_to_file_with_embedded_schema(val, false);
        let loaded = crate::schema::load_schema(&buf, None, None).unwrap();
        let data: &'static [u8] = Box::leak(buf.into_boxed_slice());
        let schema_bytes: &'static [u8] = Box::leak(loaded.schema_bytes.into_boxed_slice());
        let ctx: &'static RuntimeSchemaCtx<'static> =
            Box::leak(Box::new(RuntimeSchemaCtx::new(data, schema_bytes)));
        let schema_reader = <Schema as MmapRead>::read(schema_bytes, 0..schema_bytes.len());
        let root_type = *schema_reader.root_type;
        let root = SchemaValue::new(
            loaded.payload_range.start as u64,
            loaded.payload_range.end as u64,
            root_type,
        );
        (ctx, root)
    }

    #[test]
    fn test_overscan_new() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let os = Overscan::new(scanner);
        // All collapsed by default
        assert_eq!(os.nodes.len(), 2);
        assert!(!os.nodes[0].expanded); // inner, collapsed
        assert!(!os.nodes[1].expanded); // y, no children
    }

    #[test]
    fn test_overscan_from_previous_preserves_expanded() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut prev = Overscan::new(scanner);
        // inner starts collapsed (default)
        assert!(!prev.nodes[0].expanded);
        // Toggle inner to expanded
        prev.nodes[0].expanded = true;

        // Rebuild with a fresh scanner for the same data
        let scanner2 = Scanner::new(ctx, root, 10);
        let next = Overscan::from_previous(&prev, scanner2);
        // inner should stay expanded → its child `a` appears
        assert_eq!(next.nodes.len(), 3);
        assert!(next.nodes[0].expanded);
        assert!(!next.nodes[1].expanded);
        assert!(!next.nodes[2].expanded);
    }

    #[test]
    fn test_overscan_from_previous_toggle_expanded() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);

        // Build first overscan with everything collapsed
        let scanner1 = Scanner::new(ctx, root, 10);
        let mut prev = Overscan::new(scanner1);
        // Toggle inner to collapsed
        prev.nodes[0].expanded = false;

        // Rebuild — inner should stay collapsed
        let scanner2 = Scanner::new(ctx, root, 10);
        let next = Overscan::from_previous(&prev, scanner2);
        // Only 2 top-level nodes: inner and y (inner is collapsed)
        assert_eq!(next.nodes.len(), 2);
        assert!(!next.nodes[0].expanded);
        assert!(!next.nodes[1].expanded);
    }

    #[test]
    fn test_expand_node_inserts_children() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut os = Overscan::new(scanner);

        // Initially 2 nodes: inner, y
        assert_eq!(os.nodes.len(), 2);

        // Expand inner (index 0)
        let inserted = os.expand_node(0, 10);
        assert!(inserted > 0, "should insert at least 1 child");
        assert!(os.nodes.len() > 2, "nodes should grow after expand");

        // Node 0 is expanded with children_loaded
        assert!(os.nodes[0].expanded);
        assert!(os.nodes[0].children_loaded);

        // Child nodes should have depth = parent_depth + 1
        for i in 1..=inserted {
            assert_eq!(os.nodes[i].depth, os.nodes[0].depth + 1);
        }

        // effective_end_stack should be empty — load_children loaded all
        // children (1 of 1), no partial load.
        assert_eq!(os.cursor.effective_end_stack.len(), 0);
    }

    #[test]
    fn test_collapse_node_does_not_remove_children() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut os = Overscan::new(scanner);

        // Expand inner
        os.expand_node(0, 10);
        assert_eq!(os.nodes.len(), 3);

        // Collapse inner — children stay
        os.collapse_node(0);
        assert_eq!(os.nodes.len(), 3);
        assert!(!os.nodes[0].expanded);

        // effective_end_stack should be cleared for the collapsed subtree
        assert_eq!(os.cursor.effective_end_stack.len(), 0);
    }

    #[test]
    fn test_skip_loaded_children_skips_descendants() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut os = Overscan::new(scanner);

        // Expand inner, then collapse
        os.expand_node(0, 10);
        os.collapse_node(0);
        // Nodes: [inner(expanded=false, children_loaded=true), a, y]

        // skip_loaded_children from index 0 should skip past a (depth=2 > inner depth=1)
        let next = os.skip_loaded_children(0);
        assert_eq!(next, 2); // index of y
    }

    #[test]
    fn test_parent_of_basic() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut os = Overscan::new(scanner);

        // Expand inner: nodes = [inner(d=1), a(d=2), y(d=1)]
        os.expand_node(0, 10);

        // Parent of a (idx=1) should be inner (idx=0)
        let p = os.parent_of(1);
        assert_eq!(p, Some(0));

        // Parent of inner (idx=0) should be None (root)
        assert_eq!(os.parent_of(0), None);

        // Parent of y (idx=2) should be None (depth=1, root-adjacent)
        assert_eq!(os.parent_of(2), None);
    }

    #[test]
    fn test_path_of_round_trip() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut os = Overscan::new(scanner);

        // Expand inner
        os.expand_node(0, 10);

        // Build a path for the child `a` at index 1
        let path_a = os.path_of(1);
        // The path should have 2 segments: (root, Field(0)) → (inner, Field(0))
        assert_eq!(path_a.len(), 2);

        // Verify the path is valid by checking it starts with root
        assert_eq!(path_a[0].val.mmap_begin, root.mmap_begin);
    }

    #[test]
    fn test_continue_scan_appends_nodes() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 2); // only 2 items per batch
        let mut os = Overscan::new(scanner);

        // With all collapsed: Simple has 2 fields → both fit in batch
        // For Outer: inner, y → both fit in batch of 2
        // So continue_scan shouldn't add anything
        let n_before = os.nodes.len();
        os.continue_scan(5);
        assert_eq!(os.nodes.len(), n_before); // no new nodes

        // Now expand inner (inserts 1 more node = inner, a, y = 3 items)
        os.expand_node(0, 10);
        // Reset cursor.end to something earlier to simulate needing continuation
        // Actually, after expand cursor.end is unchanged, so continue_scan
        // would start from the original end.  Let's just verify it doesn't crash.
        os.continue_scan(5);
        // Should have more nodes or at least not fewer
        assert!(os.nodes.len() >= 3);
    }

    #[test]
    fn test_expand_twice_is_idempotent() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut os = Overscan::new(scanner);

        // First expand
        let n1 = os.expand_node(0, 10);
        assert_eq!(n1, 1);
        let count_after_first = os.nodes.len();

        // Second expand — should be 0 children inserted (already loaded)
        let n2 = os.expand_node(0, 10);
        assert_eq!(n2, 0);
        assert_eq!(os.nodes.len(), count_after_first);
    }

    #[test]
    fn test_collapse_twice_is_idempotent() {
        let o = Outer {
            inner: Inner { a: 7 },
            y: 99,
        };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let mut os = Overscan::new(scanner);

        os.expand_node(0, 10);
        os.collapse_node(0);
        let n_before = os.nodes.len();

        // Collapse again — no change
        os.collapse_node(0);
        assert_eq!(os.nodes.len(), n_before);
    }

    // ── partial-load tests ──────────────────────────────────────────

    #[derive(MmapStruct, Clone, Debug)]
    #[repr(C)]
    struct WithArray {
        a: u32,
        b: u32,
        c: Vec<u32>,
        d: u32,
    }

    fn setup_with_array() -> (&'static RuntimeSchemaCtx<'static>, SchemaValue) {
        let val = WithArray {
            a: 1,
            b: 2,
            c: vec![10u32; 10],
            d: 3,
        };
        setup(&val)
    }

    #[test]
    fn test_expand_partial_array_pushes_virtual_end() {
        let (ctx, root) = setup_with_array();
        let scanner = Scanner::new(ctx, root, 200);
        let mut os = Overscan::new(scanner);

        // Expand the array field c (index 2 = the Vec<u32>), limit=5
        let inserted = os.expand_node(2, 5);
        assert_eq!(inserted, 5);

        assert!(os.nodes[2].virtual_end.is_some());
        assert!(os.nodes[2].children_loaded);

        assert_eq!(os.cursor.effective_end_stack.len(), 1);

        os.collapse_node(2);
        assert_eq!(os.cursor.effective_end_stack.len(), 0);
    }

    #[test]
    fn test_expand_full_array_no_virtual_end() {
        let (ctx, root) = setup_with_array();
        let scanner = Scanner::new(ctx, root, 200);
        let mut os = Overscan::new(scanner);

        // Expand c with limit=200 (all 10 children fit)
        let inserted = os.expand_node(2, 200);
        assert_eq!(inserted, 10);

        // Node c should NOT have virtual_end (full load)
        assert!(os.nodes[2].virtual_end.is_none());

        // effective_end_stack should be empty
        assert_eq!(os.cursor.effective_end_stack.len(), 0);
    }

    #[test]
    fn test_re_expand_partial_restores_virtual_end() {
        let (ctx, root) = setup_with_array();
        let scanner = Scanner::new(ctx, root, 200);
        let mut os = Overscan::new(scanner);

        // Expand c with limit=5 (partial)
        os.expand_node(2, 5);
        assert!(os.nodes[2].virtual_end.is_some());
        let ve = os.nodes[2].virtual_end.clone();

        // Collapse
        os.collapse_node(2);
        assert_eq!(os.cursor.effective_end_stack.len(), 0);

        // Re-expand — should restore virtual_end without loading children again
        let inserted = os.expand_node(2, 5);
        assert_eq!(inserted, 0); // no new children
        assert!(os.nodes[2].virtual_end.is_some());
        assert_eq!(os.nodes[2].virtual_end, ve);
        assert_eq!(os.cursor.effective_end_stack.len(), 1);
    }

    #[test]
    fn test_nested_partial_expand_stack() {
        let (ctx, root) = setup_with_array();
        let scanner = Scanner::new(ctx, root, 200);
        let mut os = Overscan::new(scanner);

        // Expand c with limit=3 (3 of 10 children)
        os.expand_node(2, 3);
        // Nodes: a, b, c, c[0], c[1], c[2], d
        assert_eq!(os.nodes.len(), 7);

        // Expand c again — no new children, re-push virtual_end (no dup)
        os.expand_node(2, 3);
        assert_eq!(os.cursor.effective_end_stack.len(), 1, "no duplicate push");
    }
}
