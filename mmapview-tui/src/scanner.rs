use crate::navigate::navigate_path_segments;
use crate::path::{Path, PathSegment, PathShift};
use crate::print::{format_size, inline_value_text, preview_text, type_annotation};
use mmapstruct_schema::context::RuntimeSchemaCtx;
use mmapstruct_schema::schema::TypeDefReader;
use mmapstruct_schema::SchemaValue;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Step {
    StepIn,
    StepLine,
    StepOut,
}

/// Context passed to the `do_steps` callback for each visited element.
pub struct StepContext {
    pub val: SchemaValue,
    pub step: Step,
    pub parent: Option<(SchemaValue, usize)>,
    pub depth: usize,
    pub expand: bool,
}

/// DFS walker over a structured binary buffer.
///
/// A `Scanner` holds a cursor position (`path`) into a data tree and can
/// walk forward (`do_steps`) or backward (`do_steps_back`) for a bounded
/// number of steps.  Each step visits one logical line — a struct field,
/// an array element, a hash-map entry, or an `Option` variant.
///
/// `do_steps` returns `Some(path)` where `path` points to the element
/// *after* the last processed line — this makes continuation scans
/// overlap‑free.  Returns `None` when the end of data is reached.
pub struct Scanner {
    pub ctx: &'static RuntimeSchemaCtx<'static>,
    pub path: Path,
    pub lines_begin: usize,
    pub steps: usize,
}

impl Scanner {
    pub fn new(
        ctx: &'static RuntimeSchemaCtx<'static>,
        root_val: SchemaValue,
        steps: usize,
    ) -> Self {
        let path = Path(vec![PathSegment { val: root_val, shift: PathShift::Field(0) }]);
        Scanner::from_path(ctx, path, steps)
    }

    pub fn from_path(
        ctx: &'static RuntimeSchemaCtx<'static>,
        path: Path,
        steps: usize,
    ) -> Self {
        let lb = path.lines_begin();
        Scanner { ctx, path, lines_begin: lb, steps }
    }

    /// Try to build a `Scanner` from a string path like `/items[0]/label`.
    /// Falls back to `Scanner::new(ctx, root_val, steps)` if the path is
    /// invalid or empty.
    pub fn navigate_or_new(
        ctx: &'static RuntimeSchemaCtx<'static>,
        root_val: SchemaValue,
        path_str: &str,
        steps: usize,
    ) -> Self {
        match navigate_path_segments(ctx, &root_val, path_str) {
            Some((_, path)) => Scanner::from_path(ctx, path, steps),
            None => Scanner::new(ctx, root_val, steps),
        }
    }

    pub(crate) fn child_shift(&self, val: &SchemaValue, idx: usize) -> PathShift {
        let t = self.ctx.type_reader(val.type_idx);
        match t {
            TypeDefReader::Array { .. } | TypeDefReader::HashMap { .. } => PathShift::Entry(idx),
            _ => PathShift::Field(idx),
        }
    }

    fn stack_to_path(stack: &[(SchemaValue, PathShift)]) -> Path {
        Path(stack.iter()
            .map(|(val, shift)| PathSegment { val: *val, shift: *shift })
            .collect())
    }

    pub fn do_steps<F, P>(&self, mut callback: F, mut should_step_in: P) -> Option<Path>
    where
        F: FnMut(StepContext),
        P: FnMut(SchemaValue) -> bool,
    {
        let n = self.steps;
        if n == 0 {
            return Some(self.path.clone());
        }
        // Stack: [(parent, shift_into_parent)] — one entry per level
        let mut stack: Vec<(SchemaValue, PathShift)> = self.path
            .iter()
            .map(|seg| (seg.val, seg.shift))
            .collect();

        let Some((parent, shift)) = stack.last() else { return None; };
        let idx = shift.index();
        let mut current = match self.child_at(parent, idx) {
            Some(v) => v,
            None => return None,
        };
        let mut expand = should_step_in(current);
        callback(StepContext {
            val: current,
            step: Step::StepLine,
            parent: Some((*parent, idx)),
            depth: stack.len(),
            expand,
        });

        for _ in 1..n {
            if expand && self.child_count(&current) > 0 {
                let child = self.child_at(&current, 0).unwrap();
                stack.push((current, self.child_shift(&current, 0)));
                current = child;
                expand = should_step_in(current);
                callback(StepContext {
                    val: current,
                    step: Step::StepIn,
                    parent: Some((stack[stack.len() - 1].0, 0)),
                    depth: stack.len(),
                    expand,
                });
                continue;
            }

            // Advance to next element — step out if needed
            loop {
                let last = match stack.len().checked_sub(1) {
                    Some(i) => i,
                    None => return None,
                };
                let idx = stack[last].1.index();
                if idx + 1 < self.child_count(&stack[last].0) {
                    let new_shift = self.child_shift(&stack[last].0, idx + 1);
                    stack[last].1 = new_shift;
                    current = self.child_at(&stack[last].0, idx + 1).unwrap();
                    expand = should_step_in(current);
                    callback(StepContext {
                        val: current,
                        step: Step::StepLine,
                        parent: Some((stack[last].0, idx + 1)),
                        depth: stack.len(),
                        expand,
                    });
                    break;
                }
                // No more siblings at this level — pop up
                stack.pop();
                if stack.is_empty() {
                    return None;
                }
                // Try next sibling of the parent (now stack[last-1])
                let last = stack.len() - 1;
                let idx = stack[last].1.index();
                if idx + 1 < self.child_count(&stack[last].0) {
                    let new_shift = self.child_shift(&stack[last].0, idx + 1);
                    stack[last].1 = new_shift;
                    current = self.child_at(&stack[last].0, idx + 1).unwrap();
                    expand = should_step_in(current);
                    callback(StepContext {
                        val: current,
                        step: Step::StepOut,
                        parent: Some((stack[last].0, idx + 1)),
                        depth: stack.len(),
                        expand,
                    });
                    break;
                }
            }
        }
        // Silent step: advance past the last processed element so the
        // returned path points to the *next* element after the batch.
        // This makes continuation scans overlap‑free.
        self.silent_advance(&mut stack, &mut should_step_in)
    }

    /// Advance one step without calling `callback`.
    /// Returns `Some(path)` if there is a next element, `None` if exhausted.
    /// The `should_step_in` check is applied before descending into a child.
    fn silent_advance(
        &self,
        stack: &mut Vec<(SchemaValue, PathShift)>,
        should_step_in: &mut impl FnMut(SchemaValue) -> bool,
    ) -> Option<Path> {
        // Try to step into first child (only if should_step_in allows)
        if let Some(&(val, _)) = stack.last() {
            let idx = stack.last().unwrap().1.index();
            if let Some(cur) = self.child_at(&val, idx) {
                if self.child_count(&cur) > 0 && should_step_in(cur) {
                    stack.push((cur, self.child_shift(&cur, 0)));
                    return Some(Self::stack_to_path(stack));
                }
            }
        }

        loop {
            let last = match stack.len().checked_sub(1) {
                Some(i) => i,
                None => return None,
            };
            let idx = stack[last].1.index();
            if idx + 1 < self.child_count(&stack[last].0) {
                let new_shift = self.child_shift(&stack[last].0, idx + 1);
                stack[last].1 = new_shift;
                return Some(Self::stack_to_path(stack));
            }
            stack.pop();
            if stack.is_empty() {
                return None;
            }
            let last = stack.len() - 1;
            let idx = stack[last].1.index();
            if idx + 1 < self.child_count(&stack[last].0) {
                let new_shift = self.child_shift(&stack[last].0, idx + 1);
                stack[last].1 = new_shift;
                return Some(Self::stack_to_path(stack));
            }
        }
    }

    /// Walk `n` steps backward from a `start` path in DFS order.
    ///
    /// Semantics mirror the forward `do_steps`:
    /// - decrement last segment's shift when possible, then optionally
    ///   descend into the deepest expanded child
    /// - pop when shift is 0 (the parent becomes the next previous element)
    ///
    /// Returns `(path, steps)` where `steps` is the number of steps actually
    /// taken (may be less than `n` when the beginning of the data is reached).
    pub fn do_steps_back(
        &self,
        start: &Path,
        n: usize,
        should_step_in: &impl Fn(SchemaValue) -> bool,
    ) -> (Path, usize) {
        let mut path = start.clone();
        for step in 0..n {
            let (parent_val, old_shift) = match path.last() {
                Some(s) => (s.val, s.shift),
                None => return (start.clone(), step),
            };
            if old_shift.index() > 0 {
                let new_idx = old_shift.index() - 1;
                let new_shift = self.child_shift(&parent_val, new_idx);
                path.last_mut().unwrap().shift = new_shift;
                // If the new sibling was expanded, descend to its last child
                if let Some(sibling) = self.child_at(&parent_val, new_idx) {
                    if should_step_in(sibling) {
                        self.descend_to_last_child(&mut path, sibling, should_step_in);
                    }
                }
            } else {
                if path.len() == 1 {
                    return (path, step);
                }
                path.pop();
            }
        }
        (path, n)
    }

    /// Starting from `val` (already on `path`), keep descending into the
    /// last child while `should_step_in` holds.
    fn descend_to_last_child(
        &self,
        path: &mut Path,
        mut val: SchemaValue,
        should_step_in: &impl Fn(SchemaValue) -> bool,
    ) {
        loop {
            let count = self.child_count(&val);
            if count == 0 {
                break;
            }
            let last_idx = count - 1;
            let shift = self.child_shift(&val, last_idx);
            let child = match self.child_at(&val, last_idx) {
                Some(c) => c,
                None => break,
            };
            path.push(PathSegment { val, shift });
            if !should_step_in(child) {
                break;
            }
            val = child;
        }
    }

    /// Find the path of the last visible element in the file,
    /// respecting the given expansion criteria.
    ///
    /// Walks the type structure from root, picking the last child
    /// at each level.  If the last child is expanded (`should_step_in`
    /// returns `true`), descends into it recursively.  Stops at the
    /// first collapsed child (or the leaf of an expanded chain).
    /// Returns `None` if the root has no visible children.
    pub fn last_visible_path(
        &self,
        should_step_in: &impl Fn(SchemaValue) -> bool,
    ) -> Option<Path> {
        let root_val = self.path[0].val;
        self.last_visible_recursive(root_val, should_step_in)
    }

    fn last_visible_recursive(
        &self,
        val: SchemaValue,
        should_step_in: &impl Fn(SchemaValue) -> bool,
    ) -> Option<Path> {
        let count = self.child_count(&val);
        if count == 0 {
            return None;
        }
        for i in (0..count).rev() {
            let child = self.child_at(&val, i)?;
            let shift = self.child_shift(&val, i);
            if should_step_in(child) {
                if let Some(inner) = self.last_visible_recursive(child, should_step_in) {
                    let mut path = Path(vec![PathSegment { val, shift }]);
                    path.0.extend(inner.0);
                    return Some(path);
                }
                return Some(Path(vec![PathSegment { val, shift }]));
            }
            return Some(Path(vec![PathSegment { val, shift }]));
        }
        None
    }

    pub(crate) fn child_count(&self, val: &SchemaValue) -> usize {
        let t = self.ctx.type_reader(val.type_idx);
        match t {
            TypeDefReader::Struct { fields, .. } => fields.len(),
            TypeDefReader::Enum { .. } => {
                let (variant, _) = self.ctx.enum_variant(val).unwrap_or((0, 0));
                let vt = self.ctx.type_reader(variant);
                if let TypeDefReader::Struct { fields, .. } = vt {
                    fields.len()
                } else {
                    0
                }
            }
            TypeDefReader::Array { .. } => self.ctx.array_len(val) as usize,
            TypeDefReader::HashMap { .. } => self.ctx.hashmap_len(val),
            TypeDefReader::Option { .. } => {
                if val.mmap_begin < val.mmap_end { 1 } else { 0 }
            }
            _ => 0,
        }
    }

    pub(crate) fn child_at(&self, val: &SchemaValue, idx: usize) -> Option<SchemaValue> {
        let t = self.ctx.type_reader(val.type_idx);
        match t {
            TypeDefReader::Struct { fields, .. } => {
                let field = fields.get(idx)?;
                self.ctx.field_get(val, field.name)
            }
            TypeDefReader::Enum { .. } => {
                let (variant, data_off) = self.ctx.enum_variant(val)?;
                let vt = self.ctx.type_reader(variant);
                if let TypeDefReader::Struct { fields, .. } = vt {
                    let field = fields.get(idx)?;
                    let shifted = SchemaValue {
                        mmap_begin: val.mmap_begin + data_off,
                        mmap_end: val.mmap_end,
                        type_idx: variant,
                    };
                    self.ctx.field_get(&shifted, field.name)
                } else {
                    None
                }
            }
            TypeDefReader::Array { .. } => self.ctx.array_get(val, idx as u64),
            TypeDefReader::HashMap { .. } => {
                self.ctx.hashmap_get_index(val, idx).map(|(_, v)| v)
            }
            TypeDefReader::Option { inner_type } => {
                if idx == 0 && val.mmap_begin < val.mmap_end {
                    Some(SchemaValue::new(val.mmap_begin, val.mmap_end, **inner_type))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Print a debug tree of `steps` lines, starting from the cursor position.
    pub fn print_lines(&self) {
        println!("{}", self.format_lines());
    }

    /// Same as `print_lines` but returns the output as a `String`.
    pub fn format_lines(&self) -> String {
        let mut out = String::new();
        self.do_steps(
            |ctx: StepContext| {
                use std::fmt::Write;
                let indent = "  ".repeat(ctx.depth);
                let has_children = self.child_count(&ctx.val) > 0;
                let arrow = if !has_children { " " } else if ctx.expand { "▼" } else { "▶" };
                if let Some((ref p, idx)) = ctx.parent {
                    if matches!(self.ctx.type_reader(p.type_idx), TypeDefReader::Array { .. }) {
                        let inline = inline_value_text(self.ctx, &ctx.val);
                        writeln!(out, "{}{}[{}] = {}", indent, arrow, idx, inline).unwrap();
                        return;
                    }
                    if let Some(prefix) = self.hashmap_prefix(p) {
                        let name = self.field_name(p, idx).unwrap_or_default();
                        let inline = inline_value_text(self.ctx, &ctx.val);
                        writeln!(out, "{}{}{}{}: {}", indent, arrow, prefix, name, inline).unwrap();
                        return;
                    }
                }
                let name = ctx.parent.and_then(|(p, idx)| self.field_name(&p, idx)).unwrap_or_default();
                let type_ann = type_annotation(self.ctx.type_reader(ctx.val.type_idx));
                let inline = inline_value_text(self.ctx, &ctx.val);
                if name.is_empty() {
                    writeln!(out, "{}{} type={}{} = {}", indent, arrow, ctx.val.type_idx, type_ann, inline).unwrap();
                } else {
                    writeln!(out, "{}{} {}{} = {}", indent, arrow, name, type_ann, inline).unwrap();
                }
            },
            |val| self.child_count(&val) > 0,
        );
        out
    }

    /// Load up to `limit` direct children of `val` (one level, no recursion).
    /// Returns `(child_value, child_index)` pairs and an optional `virtual_end`
    /// path pointing to the first unloaded child (when `limit < child_count`).
    pub fn load_children(&self, val: &SchemaValue, limit: usize) -> (Vec<(SchemaValue, usize)>, Option<Path>) {
        let n = self.child_count(val).min(limit);
        let children: Vec<_> = (0..n).filter_map(|i| {
            let child = self.child_at(val, i)?;
            Some((child, i))
        }).collect();

        let virtual_end = if n < self.child_count(val) {
            let shift = self.child_shift(val, n);
            let path = Path(vec![PathSegment { val: *val, shift }]);
            Some(path)
        } else {
            None
        };

        (children, virtual_end)
    }

    pub(crate) fn format_display_text(&self, val: &SchemaValue, parent: Option<(SchemaValue, usize)>) -> String {
        if let Some((ref p, idx)) = parent {
            if matches!(self.ctx.type_reader(p.type_idx), TypeDefReader::Array { .. }) {
                let inline = inline_value_text(self.ctx, val);
                let type_ann = type_annotation(self.ctx.type_reader(val.type_idx));
                let text = format!("[{}]{} = {}", idx, type_ann, inline);
                if self.child_count(val) > 0 {
                    let bytes = val.mmap_end - val.mmap_begin;
                    return format!("{}  ({})", text, format_size(bytes));
                }
                return text;
            }
            if let Some(prefix) = self.hashmap_prefix(p) {
                let name = self.field_name(p, idx).unwrap_or_default();
                let inline = inline_value_text(self.ctx, val);
                let text = format!("{}{}: {}", prefix, name, inline);
                if self.child_count(val) > 0 {
                    let bytes = val.mmap_end - val.mmap_begin;
                    return format!("{}  ({})", text, format_size(bytes));
                }
                return text;
            }
        }
        let name = parent
            .as_ref()
            .and_then(|(p, idx)| self.field_name(p, *idx))
            .unwrap_or_default();
        let type_ann = type_annotation(self.ctx.type_reader(val.type_idx));
        let byref = parent.as_ref().map_or(false, |(p, idx)| self.ctx.is_field_byref(p, *idx));
        let inline = if byref {
            format!("&{}", inline_value_text(self.ctx, val))
        } else {
            inline_value_text(self.ctx, val)
        };
        let text = if name.is_empty() {
            format!("type={}{} = {}", val.type_idx, type_ann, inline)
        } else {
            format!("{}{} = {}", name, type_ann, inline)
        };
        if self.child_count(val) > 0 {
            let bytes = val.mmap_end - val.mmap_begin;
            format!("{}  ({})", text, format_size(bytes))
        } else {
            text
        }
    }

    /// Like `format_display_text` but replaces the inline value with a
    /// compact preview of the first fields that fits within `char_limit`.
    /// Returns `None` when this value type doesn't support previews or
    /// when there isn't enough room.
    pub(crate) fn format_preview_text(
        &self,
        val: &SchemaValue,
        parent: Option<(SchemaValue, usize)>,
        char_limit: usize,
    ) -> Option<String> {
        if let Some((ref p, idx)) = parent {
            if matches!(self.ctx.type_reader(p.type_idx), TypeDefReader::Array { .. }) {
                let prefix = format!("[{}] = ", idx);
                let preview = preview_text(self.ctx, val, char_limit.saturating_sub(prefix.len()))?;
                return Some(format!("{}{}", prefix, preview));
            }
            if let Some(prefix) = self.hashmap_prefix(p) {
                let name = self.field_name(p, idx).unwrap_or_default();
                let prefix_len = format!("{}{}: ", prefix, name).len();
                let preview = preview_text(self.ctx, val, char_limit.saturating_sub(prefix_len))?;
                return Some(format!("{}{}: {}", prefix, name, preview));
            }
        }
        let name = parent
            .as_ref()
            .and_then(|(p, idx)| self.field_name(p, *idx))
            .unwrap_or_default();
        let type_ann = type_annotation(self.ctx.type_reader(val.type_idx));
        let byref = parent.as_ref().map_or(false, |(p, idx)| self.ctx.is_field_byref(p, *idx));
        let prefix_len = if name.is_empty() {
            format!("type={}{} = ", val.type_idx, type_ann).len()
        } else {
            format!("{}{} = ", name, type_ann).len()
        };
        let extra = if byref { 1 } else { 0 };
        let preview = preview_text(self.ctx, val, char_limit.saturating_sub(prefix_len + extra))?;
        let value = if byref {
            format!("&{}", preview)
        } else {
            preview
        };
        if name.is_empty() {
            Some(format!("type={}{} = {}", val.type_idx, type_ann, value))
        } else {
            Some(format!("{}{} = {}", name, type_ann, value))
        }
    }

    /// Scan forward and find the first element whose display text contains
    /// `substr`.  Returns `(match_path, continuation)`:
    ///
    /// - `match_path` — `Some` if a matching element was found within
    ///   `self.steps` elements, `None` otherwise.
    /// - `continuation` — path right *after* the last processed element
    ///   (for overlap‑free continuation), or `None` when data is exhausted.
    ///
    /// The `should_step_in` closure controls which containers are descended
    /// into during the DFS walk.
    pub fn find_text_advance(
        &self,
        substr: &str,
        mut should_step_in: impl FnMut(SchemaValue) -> bool,
    ) -> (Option<Path>, Option<Path>) {
        let n = self.steps;
        if n == 0 {
            return (None, None);
        }

        let mut stack: Vec<(SchemaValue, PathShift)> = self.path
            .iter()
            .map(|seg| (seg.val, seg.shift))
            .collect();

        let Some((parent, shift)) = stack.last() else { return (None, None); };
        let idx = shift.index();
        let Some(current) = self.child_at(parent, idx) else { return (None, None); };
        let mut current = current;
        let mut expand = should_step_in(current);

        if self.matches_search(&current, Some((*parent, idx)), substr) {
            let match_path = self.path_for_match(&stack, idx);
            let cont = self.silent_advance(&mut stack, &mut should_step_in);
            return (Some(match_path), cont);
        }

        for _ in 1..n {
            if expand && self.child_count(&current) > 0 {
                let Some(child) = self.child_at(&current, 0) else { return (None, None); };
                stack.push((current, self.child_shift(&current, 0)));
                current = child;
                expand = should_step_in(current);

                if self.matches_search(&current, Some((stack[stack.len() - 1].0, 0)), substr) {
                    let match_path = self.path_for_match(&stack, 0);
                    let cont = self.silent_advance(&mut stack, &mut should_step_in);
                    return (Some(match_path), cont);
                }
                continue;
            }

            loop {
                let Some(last) = stack.len().checked_sub(1) else { return (None, None); };
                let idx = stack[last].1.index();
                if idx + 1 < self.child_count(&stack[last].0) {
                    let new_shift = self.child_shift(&stack[last].0, idx + 1);
                    stack[last].1 = new_shift;
                    let Some(child) = self.child_at(&stack[last].0, idx + 1) else { return (None, None); };
                    current = child;
                    expand = should_step_in(current);

                    if self.matches_search(&current, Some((stack[last].0, idx + 1)), substr) {
                        let match_path = self.path_for_match(&stack, idx + 1);
                        let cont = self.silent_advance(&mut stack, &mut should_step_in);
                        return (Some(match_path), cont);
                    }
                    break;
                }
                stack.pop();
                if stack.is_empty() {
                    let cont = self.silent_advance(&mut stack, &mut should_step_in);
                    return (None, cont);
                }
                let last = stack.len() - 1;
                let idx = stack[last].1.index();
                if idx + 1 < self.child_count(&stack[last].0) {
                    let new_shift = self.child_shift(&stack[last].0, idx + 1);
                    stack[last].1 = new_shift;
                    let Some(child) = self.child_at(&stack[last].0, idx + 1) else { return (None, None); };
                    current = child;
                    expand = should_step_in(current);

                    if self.matches_search(&current, Some((stack[last].0, idx + 1)), substr) {
                        let match_path = self.path_for_match(&stack, idx + 1);
                        let cont = self.silent_advance(&mut stack, &mut should_step_in);
                        return (Some(match_path), cont);
                    }
                    break;
                }
            }
        }
        // All n steps exhausted — advance past the last element
        let cont = self.silent_advance(&mut stack, &mut should_step_in);
        (None, cont)
    }

    /// Check if `val` matches a search substring using the same text
    /// the user sees in the UI — preview text for containers, plain
    /// display text for leaves.
    fn matches_search(
        &self,
        val: &SchemaValue,
        parent: Option<(SchemaValue, usize)>,
        substr: &str,
    ) -> bool {
        if self.child_count(val) > 0 {
            self.format_preview_text(val, parent, 200)
                .is_some_and(|t| t.contains(substr))
        } else {
            self.format_display_text(val, parent).contains(substr)
        }
    }

    /// Build the `Path` that, when used as a scanner start path, will
    /// visit `current` first (the element at `child_idx` of `stack`'s
    /// top-of-stack parent).
    fn path_for_match(
        &self,
        stack: &[(SchemaValue, PathShift)],
        child_idx: usize,
    ) -> Path {
        let mut segs: Vec<PathSegment> = stack.iter()
            .map(|(val, shift)| PathSegment { val: *val, shift: *shift })
            .collect();
        if let Some(last) = segs.last_mut() {
            last.shift = self.child_shift(&stack.last().unwrap().0, child_idx);
        }
        Path(segs)
    }

    pub(crate) fn field_name(&self, parent: &SchemaValue, idx: usize) -> Option<String> {
        let t = self.ctx.type_reader(parent.type_idx);
        match t {
            TypeDefReader::Struct { fields, .. } => {
                fields.get(idx).map(|f| f.name.to_string())
            }
            TypeDefReader::Enum { .. } => {
                let (variant_type_idx, _) = self.ctx.enum_variant(parent)?;
                let vt = self.ctx.type_reader(variant_type_idx);
                if let TypeDefReader::Struct { fields, .. } = vt {
                    fields.get(idx).map(|f| f.name.to_string())
                } else {
                    None
                }
            }
            TypeDefReader::HashMap { .. } => {
                let (key, _) = self.ctx.hashmap_get_index(parent, idx)?;
                Some(inline_value_text(self.ctx, &key))
            }
            _ => None,
        }
    }

    fn hashmap_prefix(&self, parent: &SchemaValue) -> Option<String> {
        let t = self.ctx.type_reader(parent.type_idx);
        if let TypeDefReader::HashMap { key_type, val_type, .. } = t {
            let kt = self.ctx.type_reader(**key_type);
            let vt = self.ctx.type_reader(**val_type);
            Some(format!("({}→{}) ", type_name_short(kt), type_name_short(vt)))
        } else {
            None
        }
    }
}

fn type_name_short<'a>(t: &'a TypeDefReader<'a>) -> &'a str {
    match t {
        TypeDefReader::Struct { name, .. } => name,
        TypeDefReader::Enum { name, .. } => name,
        TypeDefReader::Array { .. } => "[]",
        TypeDefReader::HashMap { .. } => "{}",
        _ => {
            let ann = type_annotation(t);
            if ann.len() > 2 { &ann[2..] } else { "?" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mmapstruct::MmapStruct;
    use mmapstruct::schema::GetSchema;
    use mmapstruct::traits::{LayoutHash, MmapRead, MmapWrite};
    use mmapstruct_schema::{write_to_file_with_embedded_schema, Schema};
    use bytemuck::{Pod, Zeroable};

    #[derive(MmapStruct, Default, Clone, Copy, Debug, Pod, Zeroable)]
    #[repr(C)]
    struct Simple {
        x: u32,
        y: u32,
    }

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
        let ctx: &'static RuntimeSchemaCtx<'static> = Box::leak(Box::new(
            RuntimeSchemaCtx::new(data, schema_bytes),
        ));
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
    fn test_do_steps_simple_struct() {
        let s = Simple { x: 42, y: 99 };
        let (ctx, root) = setup(&s);
        let overscan = Scanner::new(ctx, root, 10);
        let mut items = Vec::new();
        overscan.do_steps(
            |ctx: StepContext| items.push((ctx.val, ctx.step)),
            |_| false,
        );

        assert_eq!(items.len(), 2, "expected 2 items (2 fields), got {}", items.len());
        assert_eq!(items[0].1, Step::StepLine);
        assert_eq!(items[1].1, Step::StepLine);
    }

    #[test]
    fn test_do_steps_back_preserves_order() {
        let o = Outer { inner: Inner { a: 7 }, y: 99 };
        let (ctx, root) = setup(&o);
        let should_step_in = |v: SchemaValue| {
            let s = Scanner::new(ctx, root, 10);
            s.child_count(&v) > 0
        };

        // Use steps=3 so do_steps processes 3 items (inner, a, y) and then
        // the silent advance returns None (end of data).
        let forward_scanner = Scanner::new(ctx, root, 3);
        let mut forward = Vec::new();
        forward_scanner.do_steps(
            |ctx: StepContext| forward.push(ctx.val),
            should_step_in,
        );

        assert_eq!(forward.len(), 3);

        // Verify backward consistency by stepping back from the full scan.
        // Scann all 3 items with silent advance — we get None because there
        // is no element after y. Walk back from the continuation path
        // after 2 items (which is the path of y).
        let after_two = Scanner::new(ctx, root, 2)
            .do_steps(|_: StepContext| {}, should_step_in)
            .unwrap();
        // after_two is the continuation path pointing to y.
        // Walk back 1 step to get a's path, then another to get inner's path.
        let (path_a, _) = forward_scanner.do_steps_back(&after_two, 1, &should_step_in);
        let (path_inner, _) = forward_scanner.do_steps_back(&path_a, 1, &should_step_in);
        // Going back one more should return 0 steps
        let (_, steps) = forward_scanner.do_steps_back(&path_inner, 1, &should_step_in);
        assert_eq!(steps, 0, "should be at beginning of data");

        // Round-trip: do_steps_back from the path after scanning 1 item
        // (inner) should give the path of inner's predecessor — which is
        // the first element reached via from_previous.
        let after_one = Scanner::new(ctx, root, 1)
            .do_steps(|_: StepContext| {}, should_step_in)
            .unwrap();
        // after_one is continuation pointing to a (inner was expanded).
        // Walk back 1 gives inner's path.
        let (inner_again, _) = forward_scanner.do_steps_back(&after_one, 1, &should_step_in);
        // Walk back 1 more → beginning of data (0 steps)
        let (_, steps) = forward_scanner.do_steps_back(&inner_again, 1, &should_step_in);
        assert_eq!(steps, 0, "should be at beginning of data");

        // Verify that going back past the beginning returns 0 steps
        let first_path = Scanner::new(ctx, root, 1).do_steps(
            |_: StepContext| {},
            should_step_in,
        );
        // first_path is the continuation path after the first element
        // Walk back to get the first element's path, then try again
        let (actual_first, steps) = forward_scanner.do_steps_back(&first_path.unwrap(), 1, &should_step_in);
        assert_eq!(steps, 1, "should be able to go back to first element");
        let (_, steps) = forward_scanner.do_steps_back(&actual_first, 1, &should_step_in);
        assert_eq!(steps, 0, "should be at beginning of data");
    }

    /// Run `format_lines` on an overscan constructed via `navigate_or_new` and
    /// compare with the expected output.
    fn generic_print_test<T>(val: &T, path: &str, expected: &str)
    where
        T: MmapWrite + GetSchema + LayoutHash,
    {
        let (ctx, root) = setup(val);
        let overscan = Scanner::navigate_or_new(ctx, root, path, 20);
        assert_eq!(overscan.format_lines(), expected);
    }

    #[test]
    fn test_print_outer_via_new() {
        // Scanner::new starts at Field(0) — depth=1, cursor on `inner`.
        let o = Outer { inner: Inner { a: 7 }, y: 99 };
        let (ctx, root) = setup(&o);
        let overscan = Scanner::new(ctx, root, 10);
        assert_eq!(
            overscan.format_lines(),
            concat!(
                "  ▼ inner = Inner { ... }\n",
                "      a: u32 = 7\n",
                "    y: u32 = 99\n",
            )
        );
    }

    #[test]
    fn test_print_outer_via_navigate_root() {
        // "/" → auto-step into root → same as Scanner::new
        let o = Outer { inner: Inner { a: 7 }, y: 99 };
        generic_print_test(&o, "/", concat!(
            "  ▼ inner = Inner { ... }\n",
            "      a: u32 = 7\n",
            "    y: u32 = 99\n",
        ));
    }

    #[test]
    fn test_print_outer_via_navigate_y() {
        // "/y" → field(1) of root, primitive → no auto-step
        let o = Outer { inner: Inner { a: 7 }, y: 99 };
        generic_print_test(&o, "/y", concat!(
            "    y: u32 = 99\n",
        ));
    }

    #[test]
    fn test_print_outer_via_navigate_inner() {
        // "/inner" → field(0) of root, then auto-step into child 0
        let o = Outer { inner: Inner { a: 7 }, y: 99 };
        generic_print_test(&o, "/inner", concat!(
            "      a: u32 = 7\n",
            "    y: u32 = 99\n",
        ));
    }

    #[test]
    fn test_do_steps_returns_end_path() {
        let s = Simple { x: 42, y: 99 };
        let (ctx, root) = setup(&s);

        // Scan 1 element — end path should point AFTER element 0
        let sc = Scanner::new(ctx, root, 1);
        let end = sc.do_steps(|_: StepContext| {}, |_| false);
        assert!(end.is_some(), "1-step scan should have a next position");

        // Scan another from end — should get element 1 (the last one)
        let sc2 = Scanner::from_path(ctx, end.unwrap(), 1);
        let mut items = Vec::new();
        let end2 = sc2.do_steps(
            |ctx: StepContext| items.push(ctx.val),
            |_| false,
        );
        // After scanning y (last element), silent advance fails → end2 is None
        assert!(end2.is_none(), "end should be None after last element");
        assert_eq!(items.len(), 1, "should get 1 element");
    }

    #[test]
    fn test_do_steps_returns_none_at_end() {
        let s = Simple { x: 42, y: 99 };
        let (ctx, root) = setup(&s);

        // Scan all 2 elements — we used steps=2 which is enough for 2 items,
        // plus the silent advance will try to go past the end
        let sc = Scanner::new(ctx, root, 2);
        let mut items = Vec::new();
        let end = sc.do_steps(
            |ctx: StepContext| items.push(ctx.val),
            |_| false,
        );
        assert_eq!(items.len(), 2, "should get 2 elements");
        // end is None because after scanning 2 elements of a 2-item struct,
        // there's nowhere left to advance
        assert!(end.is_none(), "should be at end of data");
    }

    #[test]
    fn test_do_steps_contiguous_scan() {
        let s = Simple { x: 42, y: 99 };
        let (ctx, root) = setup(&s);

        // Scan 1 element
        let sc = Scanner::new(ctx, root, 1);
        let mut first = Vec::new();
        let end = sc.do_steps(|ctx: StepContext| first.push(ctx.val), |_| false);
        let end = end.expect("should have continuation path");
        assert_eq!(first.len(), 1);

        // Continue from end: scan 1 more
        let sc2 = Scanner::from_path(ctx, end, 1);
        let mut second = Vec::new();
        let _ = sc2.do_steps(|ctx: StepContext| second.push(ctx.val), |_| false);
        assert_eq!(second.len(), 1);
        assert!(
            first[0].type_idx != second[0].type_idx
                || first[0].mmap_begin != second[0].mmap_begin,
            "first and second batch should be different elements"
        );
    }
}
