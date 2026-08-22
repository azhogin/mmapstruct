use crate::path::{Path, PathShift};
use crate::scanner::{Scanner, StepContext};
use mmapstruct_schema::SchemaValue;

/// A single element in the logical tree view, built from `Scanner::do_steps`.
///
/// Eventually replaces `tree::TreeNode`.
pub struct ViewNode {
    pub value: SchemaValue,
    pub depth: usize,
    pub has_children: bool,
    pub expanded: bool,
    pub children_loaded: bool,
    /// Pre-formatted text after the arrow indicator.
    /// `draw_viewport` prepends the arrow.
    pub display_text: String,
    /// If this node was expanded with a limit < child_count,
    /// stores the virtual_end to restore on re-expand.
    pub virtual_end: Option<Path>,
    /// The parent's SchemaValue (for path reconstruction).
    pub parent_val: Option<SchemaValue>,
    /// The shift from parent_val to this node (for path reconstruction).
    pub shift: Option<PathShift>,
}

impl ViewNode {
    pub fn collapsed_with_children(&self) -> bool {
        !self.expanded && self.has_children && self.children_loaded
    }
}

/// Build a flat list of `ViewNode`s by scanning from the cursor position.
///
/// `should_step_in` decides whether to descend into a value (revealing
/// its children).  The resulting node's `expanded` mirrors this decision.
/// Returns `(nodes, Some(end_path))` where `end_path` is the path *after*
/// the last processed element (for continuation), or `None` when data is
/// exhausted.
pub fn build_view_nodes(
    scanner: &Scanner,
    should_step_in: impl FnMut(SchemaValue) -> bool,
) -> (Vec<ViewNode>, Option<Path>) {
    let mut nodes = Vec::new();
    let end_path = scanner.do_steps(
        |ctx: StepContext| {
            let has_children = scanner.child_count(&ctx.val) > 0;
            let display_text = scanner.format_display_text(&ctx.val, ctx.parent);
            let shift = ctx.parent.map(|(pv, ci)| scanner.child_shift(&pv, ci));
            nodes.push(ViewNode {
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
    (nodes, end_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scanner::Scanner;
    use mmapstruct::MmapStruct;
    use mmapstruct::schema::GetSchema;
    use mmapstruct::traits::{LayoutHash, MmapRead, MmapWrite};
    use mmapstruct_schema::{write_to_file_with_embedded_schema, Schema};
    use mmapstruct_schema::context::RuntimeSchemaCtx;
    use mmapstruct_schema::SchemaValue;
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
    fn test_build_view_nodes_simple() {
        let s = Simple { x: 42, y: 99 };
        let (ctx, root) = setup(&s);
        let scanner = Scanner::new(ctx, root, 10);
        let (nodes, _) = build_view_nodes(&scanner, |_| true);
        assert_eq!(nodes.len(), 2);
        assert!(!nodes[0].has_children); // u32 leaf
        assert_eq!(nodes[0].depth, 1);
        assert!(!nodes[1].has_children);
        assert_eq!(nodes[1].depth, 1);
    }

    #[test]
    fn test_build_view_nodes_nested() {
        let o = Outer { inner: Inner { a: 7 }, y: 99 };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let (nodes, _) = build_view_nodes(&scanner, |val| {
            // expand only struct-with-children, not primitives
            scanner.child_count(&val) > 0
        });
        assert_eq!(nodes.len(), 3);
        // inner: expanded Struct with 1 field
        assert!(nodes[0].has_children);
        assert!(nodes[0].expanded);
        assert_eq!(nodes[0].depth, 1);
        // a: leaf → not expanded
        assert!(!nodes[1].has_children);
        assert!(!nodes[1].expanded);
        assert_eq!(nodes[1].depth, 2);
        // y: leaf → not expanded
        assert!(!nodes[2].has_children);
        assert!(!nodes[2].expanded);
        assert_eq!(nodes[2].depth, 1);
    }

    #[test]
    fn test_build_view_nodes_flat_despite_children() {
        // should_step_in returns false for everything — all nodes appear at depth=1
        let o = Outer { inner: Inner { a: 7 }, y: 99 };
        let (ctx, root) = setup(&o);
        let scanner = Scanner::new(ctx, root, 10);
        let (nodes, _) = build_view_nodes(&scanner, |_| false);
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].has_children); // inner has 1 field
        assert!(!nodes[0].expanded);
        assert_eq!(nodes[0].depth, 1);
        assert!(!nodes[1].has_children); // y is a leaf
        assert!(!nodes[1].expanded);
        assert_eq!(nodes[1].depth, 1);
    }
}
