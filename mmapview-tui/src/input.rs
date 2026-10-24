use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};

use crate::app::App;

/// Handle keyboard input, returns `true` to continue.
pub fn handle_input(app: &mut App) -> Result<bool, String> {
    if !event::poll(std::time::Duration::from_millis(100)).unwrap_or(false) {
        return Ok(true);
    }
    let event = event::read().map_err(|e| format!("Event error: {}", e))?;
    // Any key press dismisses the search message
    if let Event::Key(_) = &event {
        app.search_message = None;
    }
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if app.search_mode {
                handle_search_key(key, app);
            } else {
                handle_normal_key(key, app);
            }
        }
        Event::Resize(_, h) => app.handle_resize(h as usize),
        _ => {}
    }
    Ok(!app.should_quit)
}

fn handle_normal_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.should_quit = true;
        }
        KeyCode::Char('/') => {
            app.search_mode = true;
            app.search_buffer.clear();
        }
        KeyCode::Char('n') => {
            app.search_next();
        }
        KeyCode::Char('N') => {
            app.search_prev();
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.navigate_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.navigate_down();
        }
        KeyCode::PageUp => {
            app.navigate_page_up();
        }
        KeyCode::PageDown => {
            app.navigate_page_down();
        }
        KeyCode::Home => {
            app.navigate_home();
        }
        KeyCode::End => {
            app.navigate_end();
        }
        KeyCode::Enter | KeyCode::Char(' ') => {
            if app.overscan.nodes[app.viewport.selected].expanded {
                app.collapse_selected();
            } else {
                app.expand_selected();
            }
        }
        _ => {}
    }
}

fn handle_search_key(key: KeyEvent, app: &mut App) {
    match key.code {
        KeyCode::Esc => {
            app.search_mode = false;
            app.search_buffer.clear();
        }
        KeyCode::Enter => {
            if !app.search_buffer.is_empty() {
                app.search.start(app.search_buffer.clone());
                app.search_first();
            }
            app.search_mode = false;
            app.search_buffer.clear();
        }
        KeyCode::Char(c) => {
            app.search_buffer.push(c);
        }
        KeyCode::Backspace => {
            app.search_buffer.pop();
        }
        _ => {}
    }
}
