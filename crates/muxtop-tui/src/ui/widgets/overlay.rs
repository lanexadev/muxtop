// Overlays — the geometry shared by the palette, help, confirm and inspector.
//
// Every overlay must survive a terminal smaller than it wants to be, because
// the alternative is a panic or a half-drawn box on somebody's 60-column
// phone-sized ssh session.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Clear;

use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

use super::panel;

/// A rectangle of at most `width`×`height`, centred in `area`.
///
/// Clamps to `area` rather than overflowing it, so an overlay asking for more
/// room than exists simply fills the screen.
pub fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

/// A rectangle taking `percent` of `area`'s width, centred, with margins.
pub fn centered_percent(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let w = area.width.saturating_mul(percent_x.min(100)) / 100;
    let h = area.height.saturating_mul(percent_y.min(100)) / 100;
    centered(w.max(1), h.max(1), area)
}

/// Clear `popup` and draw a titled panel in it, returning the inner region.
///
/// Returns an empty `Rect` when there is no room for content, which callers
/// must treat as "draw nothing" rather than as an error.
pub fn popup(frame: &mut Frame, popup: Rect, title: &str, theme: &Theme, glyphs: &Glyphs) -> Rect {
    if popup.width < 3 || popup.height < 3 {
        return Rect::new(popup.x, popup.y, 0, 0);
    }
    frame.render_widget(Clear, popup);
    let block = panel::block(Some(title), true, theme, glyphs);
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    inner
}

/// Footer hint drawn on an overlay's bottom border, e.g. `Esc close`.
pub fn hint_title(hint: &str) -> String {
    format!(" {hint} ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::ColorSupport;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn centered_is_centered() {
        let area = Rect::new(0, 0, 80, 24);
        let r = centered(60, 16, area);
        assert_eq!((r.x, r.y, r.width, r.height), (10, 4, 60, 16));
    }

    #[test]
    fn centered_clamps_to_the_available_area() {
        let area = Rect::new(0, 0, 20, 10);
        let r = centered(60, 16, area);
        assert_eq!((r.width, r.height), (20, 10));
        assert_eq!((r.x, r.y), (0, 0));
    }

    #[test]
    fn centered_respects_a_non_zero_origin() {
        let area = Rect::new(5, 3, 40, 10);
        let r = centered(20, 4, area);
        assert_eq!((r.x, r.y), (15, 6));
    }

    #[test]
    fn centered_percent_never_returns_zero_size() {
        let area = Rect::new(0, 0, 4, 2);
        let r = centered_percent(10, 10, area);
        assert!(r.width >= 1 && r.height >= 1);
    }

    #[test]
    fn popup_returns_empty_when_there_is_no_room() {
        let mut terminal = Terminal::new(TestBackend::new(10, 10)).unwrap();
        let theme = Theme::new(ColorSupport::TrueColor);
        let glyphs = Glyphs::new(true);
        let mut inner = Rect::new(0, 0, 9, 9);
        terminal
            .draw(|f| {
                inner = popup(f, Rect::new(0, 0, 2, 2), "Help", &theme, &glyphs);
            })
            .unwrap();
        assert_eq!((inner.width, inner.height), (0, 0));
    }

    #[test]
    fn popup_draws_its_title_and_returns_the_inner_area() {
        let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
        let theme = Theme::new(ColorSupport::TrueColor);
        let glyphs = Glyphs::new(true);
        let mut inner = Rect::default();
        terminal
            .draw(|f| {
                inner = popup(f, Rect::new(2, 1, 20, 6), "Help", &theme, &glyphs);
            })
            .unwrap();
        assert_eq!((inner.width, inner.height), (18, 4));
        let buf = terminal.backend().buffer().clone();
        let top: String = (0..30)
            .map(|x| buf.cell((x, 1)).map(|c| c.symbol()).unwrap_or(" "))
            .collect();
        assert!(top.contains("Help"));
    }
}
