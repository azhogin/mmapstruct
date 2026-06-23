use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};
use ratatui::Frame;

use crate::app::App;

/// Render the TUI.
pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let [list_area, status_area] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

    let items: Vec<ListItem> = app
        .visible
        .iter()
        .skip(app.scroll)
        .map(|&node_idx| {
            let node = &app.nodes[node_idx];
            ListItem::new(node.line.clone())
        })
        .collect();

    let mut state = ListState::default();
    state.select(Some(app.selected.saturating_sub(app.scroll)));

    let list = List::new(items)
        .block(Block::default().borders(Borders::NONE))
        .highlight_style(Style::new().reversed());

    frame.render_stateful_widget(list, list_area, &mut state);

    // Status bar
    let status = format!(
        " q quit  ↑↓ navigate  Space expand  depth: {}/{}  nodes: {}",
        app.depth,
        "∞",
        app.visible.len(),
    );
    let status_line = Line::from(Span::raw(status));
    frame.render_widget(status_line, status_area);
}
