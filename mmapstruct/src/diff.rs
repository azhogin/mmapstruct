// TODO: Structural diff based on MmapRead / MmapWrite

use std::collections::HashMap;

/// Represents a difference between two values.
#[derive(Debug, Clone, PartialEq)]
pub enum DiffValue {
    /// A simple value change
    Changed {
        old: Vec<u8>,
        new: Vec<u8>,
    },
    /// An element added to a collection
    Added(Vec<u8>),
    /// An element removed from a collection
    Removed(Vec<u8>),
    /// Nested diff for structs
    Nested(HashMap<String, DiffValue>),
    /// Recursive diff for arrays
    ArrayDiff(Vec<(usize, Box<DiffValue>)>),
}

/// Compute a binary diff between two byte slices.
/// Returns offsets and descriptions of changes.
pub fn binary_diff(old: &[u8], new: &[u8]) -> Vec<(usize, DiffValue)> {
    let mut diffs = Vec::new();

    let min_len = old.len().min(new.len());

    // Find differing bytes
    let mut i = 0;
    while i < min_len {
        if old[i] != new[i] {
            let start = i;
            let mut end = i + 1;
            while end < min_len && old[end] != new[end] {
                end += 1;
            }
            diffs.push((
                start,
                DiffValue::Changed {
                    old: old[start..end].to_vec(),
                    new: new[start..end].to_vec(),
                },
            ));
            i = end;
        } else {
            i += 1;
        }
    }

    if old.len() > new.len() {
        diffs.push((
            new.len(),
            DiffValue::Removed(old[new.len()..].to_vec()),
        ));
    } else if new.len() > old.len() {
        diffs.push((
            old.len(),
            DiffValue::Added(new[old.len()..].to_vec()),
        ));
    }

    diffs
}

/// Format diff output for display.
pub fn format_diff(diffs: &[(usize, DiffValue)]) -> String {
    let mut out = String::new();
    for (offset, diff) in diffs {
        match diff {
            DiffValue::Changed { old, new } => {
                out.push_str(&format!(
                    "  @{:#x}: {} bytes changed\n    old: {:02x?}\n    new: {:02x?}\n",
                    offset,
                    old.len(),
                    old,
                    new
                ));
            }
            DiffValue::Added(data) => {
                out.push_str(&format!(
                    "  @{:#x}: {} bytes added: {:02x?}\n",
                    offset,
                    data.len(),
                    data
                ));
            }
            DiffValue::Removed(data) => {
                out.push_str(&format!(
                    "  @{:#x}: {} bytes removed: {:02x?}\n",
                    offset,
                    data.len(),
                    data
                ));
            }
            DiffValue::Nested(_) | DiffValue::ArrayDiff(_) => {}
        }
    }
    out
}
