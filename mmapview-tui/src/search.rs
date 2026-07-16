/// Search state for the TUI viewer.
///
/// Tracks an active pattern (`/text` mode) and the index of the most
/// recently found node.  Scanning and overscan rebuild is handled by
/// the caller (`App`) — this struct only stores state.
pub struct SearchState {
    pub pattern: String,
    pub match_idx: Option<usize>,
}

/// Parsed fast-path: `field:target` or `field=target`.
///
/// For arrays:
/// - `field` empty → `target_str` is the array index to jump to
/// - `field` set   → `target_str` is the expected numeric value of that field
///
/// For HashMaps:
/// - `field` empty → `target_str` is the entry index (numeric)
/// - `field` set   → `target_str` is the key value to binary-search for
pub struct FastPath {
    pub field: String,
    pub target_str: String,
}

impl FastPath {
    /// Parse `target_str` as a numeric `u64` for array-index jumps
    /// and field-value verification.
    pub fn target_as_num(&self) -> Option<u64> {
        self.target_str.parse().ok()
    }
}

impl SearchState {
    pub fn new() -> Self {
        SearchState {
            pattern: String::new(),
            match_idx: None,
        }
    }

    pub fn is_active(&self) -> bool {
        !self.pattern.is_empty()
    }

    /// Returns `Some(pattern)` when the search pattern is non-empty, `None` otherwise.
    pub fn get_pattern(&self) -> Option<String> {
        if self.pattern.is_empty() { None } else { Some(self.pattern.clone()) }
    }

    /// Start a new search with `pattern`. Clears previous match.
    pub fn start(&mut self, pattern: String) {
        self.pattern = pattern;
        self.match_idx = None;
    }

    /// Clear the search, resetting to inactive state.
    pub fn clear(&mut self) {
        self.pattern.clear();
        self.match_idx = None;
    }

    /// Try to parse the pattern as a fast-path `field:target` or `field=target`.
    pub fn parse_fast_path(&self) -> Option<FastPath> {
        let p = self.pattern.trim();
        let (field, target) = p.split_once(':').or_else(|| p.split_once('='))?;
        let field = field.trim().to_string();
        let target = target.trim().to_string();
        if target.is_empty() {
            return None;
        }
        Some(FastPath { field, target_str: target })
    }
}
