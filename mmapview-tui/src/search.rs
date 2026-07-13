/// Search state for the TUI viewer.
///
/// Tracks an active pattern (`/text` mode) and the index of the most
/// recently found node.  Scanning and overscan rebuild is handled by
/// the caller (`App`) — this struct only stores state.
pub struct SearchState {
    pub pattern: String,
    pub match_idx: Option<usize>,
}

/// Parsed fast-path: `field: number` or `field=number`.
pub struct FastPath {
    pub field: String,
    pub target: u64,
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

    /// Try to parse the pattern as a fast-path field:number search.
    /// Returns `Some(FastPath)` for patterns like `id:42` or `id=42`.
    pub fn try_fast_path(&self) -> Option<FastPath> {
        let p = self.pattern.trim();
        let (field, num_str) = p.split_once(':').or_else(|| p.split_once('='))?;
        let field = field.trim().to_string();
        let num_str = num_str.trim();
        if field.is_empty() || num_str.is_empty() {
            return None;
        }
        let target: u64 = num_str.parse().ok()?;
        Some(FastPath { field, target })
    }
}
