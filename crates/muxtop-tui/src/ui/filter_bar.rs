// Filter input bar.
//
// One implementation for all four tabs. 0.4 had four near-identical copies with
// four slightly different behaviours, and none of them showed how many rows the
// filter had kept — so a filter matching nothing looked exactly like a machine
// with nothing to show.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::Render;

/// Draw the filter input for the active tab.
pub fn draw(frame: &mut Frame, area: Rect, r: &Render<'_>, label: &str) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let app = r.app;
    let text = app.filter_text();

    let mut spans = vec![
        Span::styled(format!(" {label} "), r.theme.accent_fill()),
        Span::styled(" ", r.theme.body()),
        Span::styled(text.to_string(), r.theme.body()),
        Span::styled(r.glyphs.cursor.to_string(), r.theme.accent()),
    ];

    // The match count is the whole point: it distinguishes "nothing matches"
    // from "nothing exists".
    if !text.is_empty() {
        spans.push(Span::styled(
            format!("   {} match", app.item_count()),
            r.theme.dim(),
        ));
        if app.item_count() != 1 {
            spans.push(Span::styled("es", r.theme.dim()));
        }
    }
    spans.push(Span::styled(
        format!("   Enter apply {} Esc leave", r.glyphs.sep),
        r.theme.subtle(),
    ));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use crate::app::Tab;
    use crate::terminal::ColorSupport;
    use crate::ui::test_support::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn bar_shows_the_typed_text_and_a_match_count() {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::Char('/')));
        for c in "proc1".chars() {
            app.handle_key_event(key(KeyCode::Char(c)));
        }
        let text = all_text(&render_with(&app, 120, 24));
        assert!(text.contains("proc1"));
        assert!(text.contains("match"), "match count missing:\n{text}");
    }

    #[test]
    fn bar_reports_zero_matches_rather_than_looking_empty() {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::Char('/')));
        for c in "zzzz".chars() {
            app.handle_key_event(key(KeyCode::Char(c)));
        }
        let text = all_text(&render_with(&app, 120, 24));
        assert!(text.contains("0 match"), "expected a zero count:\n{text}");
    }

    #[test]
    fn bar_always_shows_how_to_leave() {
        let mut app = app_with_data();
        app.tab = Tab::Network;
        app.handle_key_event(key(KeyCode::Char('/')));
        let text = all_text(&render_with(&app, 120, 24));
        assert!(text.contains("Esc"));
    }

    #[test]
    fn bar_is_ascii_in_ascii_mode() {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::Char('/')));
        let buf = render_caps(&mut app, 100, 24, ColorSupport::Basic, false);
        assert!(all_text(&buf).is_ascii());
    }
}
