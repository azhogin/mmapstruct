use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::overscan::Overscan;
use crate::viewport::Viewport;

/// Render a scrollable view of `Overscan` nodes with selection highlighting.
/// Children of collapsed nodes are skipped.
pub fn draw_viewport(frame: &mut Frame, viewport: &Viewport, overscan: &Overscan) {
    let area = frame.area();
    let [list_area, status_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);
    let height = list_area.height as usize;

    let mut items = Vec::new();
    let mut selected_vis = None;

    // Walk all nodes from 0, track visible-line index (`vis`), and build
    // `items` for the window starting at `viewport.scroll`.
    let mut i = 0;
    let mut vis = 0;
    while i < overscan.nodes.len() {
        let node = &overscan.nodes[i];

        if vis >= viewport.scroll && items.len() < height {
            let indent = "  ".repeat(node.depth);
            let arrow = if !node.has_children {
                " "
            } else if node.expanded {
                "\u{25bc}"
            } else {
                "\u{25b6}"
            };

            // Show inline preview for collapsed containers
            let display_text = if node.has_children {
                let prefix_len = indent.len() + 1 + 1; // indent + arrow + space
                let char_limit = list_area.width.saturating_sub(prefix_len as u16) as usize;
                if char_limit > 10 {
                    let parent = node.parent_val.map(|pv| {
                        let idx = node.shift.map(|s| s.index()).unwrap_or(0);
                        (pv, idx)
                    });
                    overscan.scanner.format_preview_text(&node.value, parent, char_limit)
                        .unwrap_or_else(|| node.display_text.clone())
                } else {
                    node.display_text.clone()
                }
            } else {
                node.display_text.clone()
            };

            let content = format!("{}{} {}", indent, arrow, display_text);
            items.push(ListItem::new(content));
        }

        if i == viewport.selected {
            selected_vis = Some(vis);
        }

        i += 1;
        vis += 1;

        // Skip descendants of a collapsed-but-loaded node (they occupy
        // `nodes[]` slots but are not visible).
        if node.collapsed_with_children() {
            i = overscan.skip_loaded_children(i - 1);
        }
    }

    let vis_selected = selected_vis
        .map(|v| v.saturating_sub(viewport.scroll))
        .filter(|&v| v < items.len());

    let mut state = ListState::default();
    state.select(vis_selected);

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(Style::new().reversed());

    frame.render_stateful_widget(list, list_area, &mut state);

    // Status bar — debug info only in debug builds
    let debug_info = if cfg!(debug_assertions) {
        format!("sel={} scroll={} nodes={}  ", viewport.selected, viewport.scroll, overscan.nodes.len())
    } else {
        String::new()
    };
    let status = format!(
        "{}q quit  \u{2191}\u{2193} navigate  Space expand",
        debug_info,
    );
    let status_line = Line::from(Span::raw(status));
    frame.render_widget(status_line, status_area);
}
