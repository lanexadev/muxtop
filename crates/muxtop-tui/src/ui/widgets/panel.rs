// Panels — the bordered box every view sits in.
//
// One builder so that border glyphs, title styling and focus state are decided
// in a single place instead of being re-specified in each view.

use ratatui::widgets::{Block, Borders};

use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// A bordered panel with an optional title.
///
/// `focused` brightens the border; on a colourless terminal it switches between
/// bold and dim instead, so the focused panel is still identifiable.
pub fn block(title: Option<&str>, focused: bool, theme: &Theme, glyphs: &Glyphs) -> Block<'static> {
    let mut b = Block::default()
        .borders(Borders::ALL)
        .border_set(glyphs.border_set())
        .border_style(theme.border_style(focused));

    if let Some(title) = title {
        b = b.title(format!(" {title} ")).title_style(if focused {
            theme.accent()
        } else {
            theme.dim()
        });
    }
    b
}

/// A borderless panel: the content region of a full-width table, where borders
/// would only cost two columns of data.
pub fn bare() -> Block<'static> {
    Block::default().borders(Borders::NONE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::ColorSupport;
    use ratatui::layout::Rect;
    use ratatui::{Terminal, backend::TestBackend};

    fn render(focused: bool, unicode: bool) -> String {
        let mut terminal = Terminal::new(TestBackend::new(20, 4)).unwrap();
        let theme = Theme::new(ColorSupport::TrueColor);
        let glyphs = Glyphs::new(unicode);
        terminal
            .draw(|f| {
                let b = block(Some("CPU"), focused, &theme, &glyphs);
                f.render_widget(b, Rect::new(0, 0, 20, 4));
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        (0..4)
            .map(|y| {
                (0..20)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn panel_renders_its_title() {
        assert!(render(true, true).contains("CPU"));
    }

    #[test]
    fn unicode_panel_uses_rounded_corners() {
        assert!(render(false, true).starts_with('╭'));
    }

    #[test]
    fn ascii_panel_is_pure_ascii() {
        let out = render(false, false);
        assert!(out.is_ascii(), "ASCII panel leaked box drawing: {out}");
        assert!(out.starts_with('+'));
    }

    #[test]
    fn focus_changes_the_border_style() {
        let theme = Theme::new(ColorSupport::TrueColor);
        assert_ne!(theme.border_style(true), theme.border_style(false));
    }

    #[test]
    fn focus_is_visible_without_color() {
        let theme = Theme::new(ColorSupport::NoColor);
        assert_ne!(theme.border_style(true), theme.border_style(false));
    }

    #[test]
    fn untitled_panel_is_allowed() {
        let theme = Theme::new(ColorSupport::TrueColor);
        let glyphs = Glyphs::new(true);
        let _ = block(None, false, &theme, &glyphs);
        let _ = bare();
    }
}
