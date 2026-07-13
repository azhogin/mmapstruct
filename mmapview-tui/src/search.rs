/// Search state for the TUI viewer.
///
/// Tracks an active pattern (`/text` mode) and the index of the most
/// recently found node.  Scanning and overscan rebuild is handled by
/// the caller (`App`) — this struct only stores state.
pub struct SearchState {
    pub pattern: String,
    pub match_idx: Option<usize>,
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
}
