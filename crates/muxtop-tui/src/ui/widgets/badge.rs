// State badges.
//
// A container that exited with code 1 and one that exited cleanly are different
// news; plain text made them look identical. Badges carry severity in colour on
// a colour terminal and in reverse video on one without.

use ratatui::text::Span;

use crate::ui::glyphs::Glyphs;
use crate::ui::theme::{Level, Theme};

/// A filled pill: ` running `.
///
/// Used where the state is the point of the cell (container state, pod phase).
pub fn filled(text: &str, level: Level, theme: &Theme) -> Span<'static> {
    Span::styled(format!(" {text} "), theme.level_fill(level))
}

/// A marker plus its label: `● running`.
///
/// The default for table cells — a filled pill on every row of a long table is
/// visual noise, while the marker still carries the state at a glance.
pub fn marked(text: &str, level: Level, theme: &Theme, glyphs: &Glyphs) -> Span<'static> {
    let dot = marker(level, glyphs);
    Span::styled(format!("{dot} {text}"), theme.level_style(level))
}

/// The state marker alone, for width-constrained columns.
pub fn marker(level: Level, glyphs: &Glyphs) -> &'static str {
    match level {
        Level::Success => glyphs.st_running,
        Level::Neutral => glyphs.st_idle,
        Level::Info => glyphs.st_paused,
        Level::Warning => glyphs.st_warn,
        Level::Error => glyphs.st_dead,
    }
}

/// A dim key/value chip for context bars: `ns: kube-system`.
pub fn chip(label: &str, value: &str, theme: &Theme) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{label}: "), theme.subtle()),
        Span::styled(value.to_string(), theme.dim().fg(theme.fg)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::ColorSupport;
    use ratatui::style::Modifier;

    #[test]
    fn filled_badge_pads_its_label() {
        let t = Theme::new(ColorSupport::TrueColor);
        let s = filled("running", Level::Success, &t);
        assert_eq!(s.content.as_ref(), " running ");
    }

    #[test]
    fn marked_badge_prefixes_a_state_marker() {
        let t = Theme::new(ColorSupport::TrueColor);
        let g = Glyphs::new(true);
        assert_eq!(
            marked("running", Level::Success, &t, &g).content.as_ref(),
            "● running"
        );
        assert_eq!(
            marked("exited", Level::Error, &t, &g).content.as_ref(),
            "✕ exited"
        );
    }

    #[test]
    fn markers_are_distinct_per_level() {
        for g in [Glyphs::new(true), Glyphs::new(false)] {
            let all = [
                marker(Level::Success, &g),
                marker(Level::Neutral, &g),
                marker(Level::Info, &g),
                marker(Level::Warning, &g),
                marker(Level::Error, &g),
            ];
            for (i, a) in all.iter().enumerate() {
                for b in &all[i + 1..] {
                    assert_ne!(a, b, "two states share a marker glyph");
                }
            }
        }
    }

    #[test]
    fn ascii_markers_stay_ascii() {
        let g = Glyphs::new(false);
        for level in [
            Level::Success,
            Level::Neutral,
            Level::Info,
            Level::Warning,
            Level::Error,
        ] {
            assert!(marker(level, &g).is_ascii());
        }
    }

    #[test]
    fn badges_survive_a_colorless_terminal() {
        // Without colour the pill must still stand out, via reverse video.
        let t = Theme::new(ColorSupport::NoColor);
        let s = filled("exited", Level::Error, &t);
        assert!(s.style.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn chip_renders_label_and_value() {
        let t = Theme::new(ColorSupport::TrueColor);
        let spans = chip("ns", "kube-system", &t);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "ns: kube-system");
    }
}
