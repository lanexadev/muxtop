// Scrollbar.
//
// Hand-rolled rather than ratatui's, for two reasons: the glyphs must fall back
// to ASCII on a console with no line-drawing font, and muxtop needs the "how
// far down the list am I" readout that goes with it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// Draw a vertical scrollbar in a one-column-wide `area`.
///
/// Draws nothing when everything already fits — a permanent full-height thumb
/// is noise, and the column it costs is better spent on data.
pub fn vertical(
    frame: &mut Frame,
    area: Rect,
    total: usize,
    visible: usize,
    offset: usize,
    theme: &Theme,
    glyphs: &Glyphs,
) {
    if area.width == 0 || area.height == 0 || total <= visible || visible == 0 {
        return;
    }

    let height = area.height as usize;
    // Thumb length is proportional to the visible fraction, never below one.
    let thumb = ((visible as f64 / total as f64) * height as f64)
        .round()
        .max(1.0) as usize;
    let thumb = thumb.min(height);

    let max_offset = total.saturating_sub(visible);
    let travel = height - thumb;
    let start = if max_offset == 0 {
        0
    } else {
        ((offset.min(max_offset) as f64 / max_offset as f64) * travel as f64).round() as usize
    };
    let start = start.min(travel);

    let lines: Vec<Line<'static>> = (0..height)
        .map(|i| {
            if i >= start && i < start + thumb {
                Line::from(Span::styled(
                    glyphs.scroll_thumb,
                    ratatui::style::Style::default().fg(theme.scrollbar_thumb),
                ))
            } else {
                Line::from(Span::styled(
                    glyphs.scroll_track,
                    ratatui::style::Style::default().fg(theme.scrollbar_track),
                ))
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

/// Position readout for the status bar: `7/342`.
///
/// Returns `None` for an empty list, so callers can skip the segment entirely
/// rather than print a meaningless `0/0`.
pub fn position_label(selected: usize, total: usize) -> Option<String> {
    (total > 0).then(|| format!("{}/{}", selected.min(total - 1) + 1, total))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::ColorSupport;
    use ratatui::{Terminal, backend::TestBackend};

    fn render(total: usize, visible: usize, offset: usize, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(1, height)).unwrap();
        let theme = Theme::new(ColorSupport::TrueColor);
        let glyphs = Glyphs::new(true);
        terminal
            .draw(|f| {
                let area = Rect::new(0, 0, 1, height);
                vertical(f, area, total, visible, offset, &theme, &glyphs);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..height)
            .map(|y| buf.cell((0, y)).unwrap().symbol().to_string())
            .collect()
    }

    #[test]
    fn nothing_is_drawn_when_everything_fits() {
        let cells = render(10, 20, 0, 10);
        assert!(
            cells.iter().all(|c| c == " "),
            "no scrollbar when the list fits: {cells:?}"
        );
    }

    #[test]
    fn thumb_sits_at_the_top_when_scrolled_to_the_top() {
        let cells = render(100, 10, 0, 10);
        assert_eq!(cells[0], "█");
        assert_eq!(cells[9], "│");
    }

    #[test]
    fn thumb_sits_at_the_bottom_when_scrolled_to_the_end() {
        let cells = render(100, 10, 90, 10);
        assert_eq!(
            cells[9], "█",
            "end of list must park the thumb at the bottom"
        );
        assert_eq!(cells[0], "│");
    }

    #[test]
    fn thumb_is_never_thinner_than_one_cell() {
        // 100k rows in a 10-row window rounds the thumb to zero without a floor.
        let cells = render(100_000, 10, 0, 10);
        assert!(cells.iter().any(|c| c == "█"), "thumb vanished: {cells:?}");
    }

    #[test]
    fn degenerate_geometry_does_not_panic() {
        let _ = render(0, 0, 0, 10);
        let _ = render(100, 0, 0, 10);
        let _ = render(100, 10, 9_999, 10);
        let _ = render(1, 1, 0, 1);
    }

    #[test]
    fn ascii_glyphs_when_unicode_is_off() {
        let mut terminal = Terminal::new(TestBackend::new(1, 4)).unwrap();
        let theme = Theme::new(ColorSupport::TrueColor);
        let glyphs = Glyphs::new(false);
        terminal
            .draw(|f| vertical(f, Rect::new(0, 0, 1, 4), 100, 4, 0, &theme, &glyphs))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let cells: String = (0..4)
            .map(|y| buf.cell((0, y)).unwrap().symbol().to_string())
            .collect();
        assert!(
            cells.is_ascii(),
            "ASCII mode leaked a block glyph: {cells:?}"
        );
    }

    #[test]
    fn position_label_is_one_indexed() {
        assert_eq!(position_label(0, 342).as_deref(), Some("1/342"));
        assert_eq!(position_label(6, 12).as_deref(), Some("7/12"));
    }

    #[test]
    fn position_label_is_absent_for_an_empty_list() {
        assert_eq!(position_label(0, 0), None);
    }

    #[test]
    fn position_label_clamps_a_stale_selection() {
        // A snapshot can shrink the list under the cursor before it is clamped.
        assert_eq!(position_label(99, 3).as_deref(), Some("3/3"));
    }
}
