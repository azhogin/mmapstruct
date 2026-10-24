use mmapstruct_schema::SchemaValue;
use std::ops::{Deref, DerefMut};

/// Describes the sibling offset within a PathSegment's value.
/// For intermediate segments: index of the child to descend into.
/// For the last segment: cursor offset within the parent's content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathShift {
    Field(usize), // field index in struct (or current enum variant struct)
    Entry(usize), // element index in array / sorted HashMap entry
}

impl PathShift {
    pub fn index(self) -> usize {
        match self {
            PathShift::Field(idx) | PathShift::Entry(idx) => idx,
        }
    }
}

/// One level in a path from root to the current focus.
#[derive(Clone, Debug, PartialEq)]
pub struct PathSegment {
    pub val: SchemaValue,
    /// Sibling offset within this segment's value.
    /// For the last segment: cursor offset within the parent's content.
    pub shift: PathShift,
}

/// Path from root to the current focus (parent-relative, with cursor offset).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Path(pub Vec<PathSegment>);

impl Path {
    pub fn new() -> Self {
        Path(Vec::new())
    }

    /// Absolute line of the cursor: path headers + sum of all shift offsets.
    pub fn lines_begin(&self) -> usize {
        self.len() + self.iter().map(|seg| seg.shift.index()).sum::<usize>()
    }

    /// Returns `true` if `self` points to the very first DFS element in the file.
    ///
    /// When every segment has a zero shift and the path has length 1, there is
    /// no data before this position — `do_steps_back` would return 0 steps.
    pub fn is_file_begin(&self) -> bool {
        self.len() == 1 && self[0].shift.index() == 0
    }

    /// Returns `true` if `self` is a proper prefix of `other`.
    ///
    /// Both `PathSegment` fields (val + shift) must match for the first
    /// `self.len()` entries.
    pub fn is_path_prefix(&self, other: &Path) -> bool {
        self.len() < other.len()
            && self
                .iter()
                .zip(other.iter())
                .all(|(a, b)| a.val == b.val && a.shift.index() == b.shift.index())
    }
}

impl Deref for Path {
    type Target = Vec<PathSegment>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Path {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
