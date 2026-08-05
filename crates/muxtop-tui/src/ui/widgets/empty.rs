// Empty and error states.
//
// Generalises the one thing muxtop 0.4 already did well — the Containers and
// Kube placeholders that say *why* a view is empty and *what to do about it* —
// so that a filter matching nothing, a daemon that is down and a table that is
// genuinely empty all read the same way.

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::theme::{Level, Theme};

/// A placeholder for a view with nothing to show.
pub struct EmptyState<'a> {
    /// What happened, in a few words.
    pub title: &'a str,
    /// Why it happened. Optional, but a title with no reason is a dead end.
    pub reason: Option<&'a str>,
    /// What the user can do next. Optional for states with no remedy.
    pub remedy: Option<&'a str>,
    /// Severity — a filter matching nothing is not an error.
    pub level: Level,
}

impl<'a> EmptyState<'a> {
    /// Nothing matched the active filter. Not a failure: the user typed it.
    pub fn no_match(noun: &'a str) -> Self {
        Self {
            title: noun,
            reason: Some("No rows match the active filter."),
            remedy: Some("Press Esc to clear it, or / to edit it."),
            level: Level::Neutral,
        }
    }

    /// The view is legitimately empty.
    pub fn empty(title: &'a str, reason: Option<&'a str>) -> Self {
        Self {
            title,
            reason,
            remedy: None,
            level: Level::Neutral,
        }
    }

    /// Something is wrong and the user can act on it.
    pub fn error(title: &'a str, reason: &'a str, remedy: &'a str) -> Self {
        Self {
            title,
            reason: Some(reason),
            remedy: Some(remedy),
            level: Level::Error,
        }
    }

    /// Data has not arrived yet.
    pub fn waiting(what: &'a str) -> Self {
        Self {
            title: what,
            reason: None,
            remedy: None,
            level: Level::Info,
        }
    }
}

/// Render an empty state, vertically centred in `area`.
pub fn render(frame: &mut Frame, area: Rect, state: &EmptyState<'_>, theme: &Theme) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(4);
    lines.push(Line::from(Span::styled(
        state.title.to_string(),
        theme
            .level_style(state.level)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )));
    if let Some(reason) = state.reason {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(reason.to_string(), theme.dim())));
    }
    if let Some(remedy) = state.remedy {
        lines.push(Line::from(Span::styled(remedy.to_string(), theme.subtle())));
    }

    // Centre vertically, but never push the title off the top of a short area.
    let content_h = lines.len() as u16;
    let top_pad = area.height.saturating_sub(content_h) / 2;
    let inner = Rect {
        y: area.y + top_pad.min(area.height.saturating_sub(1)),
        height: area.height - top_pad.min(area.height.saturating_sub(1)),
        ..area
    };

    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::ColorSupport;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_to_string(state: &EmptyState<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let theme = Theme::new(ColorSupport::TrueColor);
        terminal
            .draw(|f| render(f, Rect::new(0, 0, w, h), state, &theme))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..h)
            .map(|y| {
                (0..w)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_title_reason_and_remedy() {
        let s = EmptyState::error(
            "No container daemon detected",
            "The socket is not answering.",
            "Start Docker, or run `podman system service`.",
        );
        let out = render_to_string(&s, 60, 10);
        assert!(out.contains("No container daemon detected"));
        assert!(out.contains("The socket is not answering."));
        assert!(out.contains("Start Docker"));
    }

    #[test]
    fn no_match_state_tells_the_user_how_to_recover() {
        let s = EmptyState::no_match("Processes");
        let out = render_to_string(&s, 60, 10);
        assert!(out.contains("filter"));
        assert!(out.contains("Esc"), "a dead end must offer a way out");
    }

    #[test]
    fn tiny_areas_do_not_panic_and_still_show_the_title() {
        let s = EmptyState::error("Broken", "Because.", "Fix it.");
        let out = render_to_string(&s, 40, 1);
        assert!(out.contains("Broken"), "the title must survive one row");
        let _ = render_to_string(&s, 1, 1);
        let _ = render_to_string(&s, 0, 0);
    }

    #[test]
    fn waiting_state_has_no_false_remedy() {
        let s = EmptyState::waiting("Waiting for data…");
        assert!(s.remedy.is_none());
        assert!(s.reason.is_none());
        let out = render_to_string(&s, 40, 6);
        assert!(out.contains("Waiting for data"));
    }
}
