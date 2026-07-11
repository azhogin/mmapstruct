/// Scrollable viewport state for navigating a flat list of `ViewNode`s.
pub struct Viewport {
    pub selected: usize,
    pub scroll: usize,
    pub display_height: usize,
}

impl Viewport {
    /// Adjust `selected` after `inserted` nodes were inserted
    /// just after index `idx` (e.g. expand loaded children).
    /// `scroll` is in visible-space and is unaffected by hidden children.
    pub fn fixup_for_insertion(&mut self, idx: usize, inserted: usize) {
        if self.selected > idx {
            self.selected += inserted;
        }
    }
}
