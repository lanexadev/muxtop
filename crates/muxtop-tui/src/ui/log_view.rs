// Message log (`Ctrl+L`).
//
// Toasts expire. An action that failed while the user was looking at another
// tab used to be gone for good; now every message muxtop has produced this
// session is one keystroke away, with its severity intact.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::Render;
use crate::ui::widgets::{badge, empty, overlay};

pub fn draw_log(frame: &mut Frame, r: &Render<'_>) {
    let area = frame.area();
    let popup = overlay::centered_percent(80, 70, area);
    let inner = overlay::popup(frame, popup, "Messages", r.theme, r.glyphs);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let [body, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);

    let history = r.app.notifier.history();
    if history.is_empty() {
        empty::render(
            frame,
            body,
            &empty::EmptyState::empty("No messages yet", Some("Actions and errors land here.")),
            r.theme,
        );
    } else {
        // Newest first: the reason you opened this is almost always the last
        // thing that happened.
        let lines: Vec<Line<'static>> = history
            .iter()
            .rev()
            .skip(r.app.overlay_scroll)
            .take(body.height as usize)
            .map(|toast| {
                Line::from(vec![
                    Span::styled(
                        format!("{} ", badge::marker(toast.level, r.glyphs)),
                        r.theme.level_style(toast.level),
                    ),
                    Span::styled(toast.text.clone(), r.theme.body()),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), body);
    }

    let hint = Line::from(vec![
        Span::styled(" Ctrl+L ", r.theme.key()),
        Span::styled(" or ", r.theme.key_desc()),
        Span::styled(" Esc ", r.theme.key()),
        Span::styled(
            format!(" close   {} messages ", history.len()),
            r.theme.key_desc(),
        ),
    ]);
    frame.render_widget(Paragraph::new(hint), footer);
}

#[cfg(test)]
mod tests {
    use crate::app::Overlay;
    use crate::terminal::ColorSupport;
    use crate::ui::test_support::*;
    use crate::ui::theme::Level;

    #[test]
    fn empty_log_says_so() {
        let mut app = app_with_data();
        app.overlay = Overlay::Log;
        let text = all_text(&render_with(&app, 100, 30));
        assert!(text.contains("No messages yet"));
    }

    #[test]
    fn log_keeps_messages_after_the_toast_expires() {
        let mut app = app_with_data();
        app.notify(Level::Error, "Failed to stop nginx: permission denied");
        app.notifier.dismiss_all();
        app.overlay = Overlay::Log;
        let text = all_text(&render_with(&app, 100, 30));
        assert!(
            text.contains("Failed to stop nginx"),
            "a dismissed error must still be recoverable:\n{text}"
        );
    }

    #[test]
    fn log_shows_newest_first() {
        let mut app = app_with_data();
        app.notify(Level::Info, "older message");
        app.notify(Level::Info, "newer message");
        app.overlay = Overlay::Log;
        let buf = render_with(&app, 100, 30);
        // First occurrence only: the newest message is also echoed by the
        // status bar on the bottom row, which is not the log's ordering.
        let mut newer = None;
        let mut older = None;
        for row in 0..buf.area.height {
            let line = line_text(&buf, row);
            if newer.is_none() && line.contains("newer message") {
                newer = Some(row);
            }
            if older.is_none() && line.contains("older message") {
                older = Some(row);
            }
        }
        assert!(
            newer.expect("newest missing") < older.expect("oldest missing"),
            "newest must be on top"
        );
    }

    #[test]
    fn log_counts_its_entries() {
        let mut app = app_with_data();
        app.notify(Level::Info, "a");
        app.notify(Level::Info, "b");
        app.overlay = Overlay::Log;
        assert!(contains(&render_with(&app, 100, 30), "2 messages"));
    }

    #[test]
    fn log_survives_tiny_terminals_and_ascii() {
        let mut app = app_with_data();
        app.notify(Level::Warning, "something");
        app.overlay = Overlay::Log;
        for (w, h) in [(1u16, 1u16), (12, 5), (40, 10)] {
            let _ = render_with(&app, w, h);
        }
        let buf = render_caps(&mut app, 80, 20, ColorSupport::Basic, false);
        assert!(all_text(&buf).is_ascii());
    }
}
