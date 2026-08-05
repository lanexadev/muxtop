// Sparklines.
//
// Written by hand rather than using ratatui's `Sparkline` so the glyph set can
// fall back to ASCII and so each bar can carry the colour of the threshold it
// crosses — a spike that matters should look different from a busy baseline.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// How a sparkline picks its colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    /// One flat colour: for rates, where "high" carries no judgement.
    Flat,
    /// Green / yellow / red by percentage of the scale: for saturation.
    Load,
}

/// Render `data` as a one-line sparkline, right-aligned so the most recent
/// sample is always the rightmost cell.
///
/// The scale is the maximum of the window unless `max` is given, so an idle
/// interface still shows its own shape instead of a flat line.
pub fn line(
    data: &[u64],
    width: u16,
    max: Option<u64>,
    tint: Tint,
    color: ratatui::style::Color,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Line<'static> {
    let width = width as usize;
    if width == 0 {
        return Line::default();
    }

    // Keep the newest `width` samples.
    let window: &[u64] = if data.len() > width {
        &data[data.len() - width..]
    } else {
        data
    };
    let scale = max
        .or_else(|| window.iter().copied().max())
        .unwrap_or(0)
        .max(1);

    let mut spans = Vec::with_capacity(width);
    // Left-pad so a short history grows in from the right.
    let pad = width - window.len();
    if pad > 0 {
        spans.push(Span::styled(
            " ".repeat(pad),
            Style::default().fg(theme.bar_empty),
        ));
    }
    for &v in window {
        let style = match tint {
            Tint::Flat => Style::default().fg(color),
            Tint::Load => {
                let pct = (v.min(scale) as f64 / scale as f64) * 100.0;
                Style::default().fg(theme.gauge_color(pct))
            }
        };
        spans.push(Span::styled(
            glyphs.spark_level(v, scale).to_string(),
            style,
        ));
    }
    Line::from(spans)
}

/// Convenience wrapper for `f32` percentage series (CPU histories).
pub fn line_percent(data: &[f32], width: u16, theme: &Theme, glyphs: &Glyphs) -> Line<'static> {
    let scaled: Vec<u64> = data
        .iter()
        .map(|v| v.clamp(0.0, 100.0).round() as u64)
        .collect();
    line(
        &scaled,
        width,
        Some(100),
        Tint::Load,
        theme.accent_primary,
        theme,
        glyphs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::ColorSupport;

    fn text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn theme() -> Theme {
        Theme::new(ColorSupport::TrueColor)
    }

    #[test]
    fn sparkline_is_exactly_the_requested_width() {
        let t = theme();
        for glyphs in [Glyphs::new(true), Glyphs::new(false)] {
            for width in 0u16..40 {
                for len in [0usize, 1, 5, 40, 200] {
                    let data: Vec<u64> = (0..len as u64).collect();
                    let l = line(&data, width, None, Tint::Flat, t.info, &t, &glyphs);
                    assert_eq!(
                        text(&l).chars().count(),
                        width as usize,
                        "width {width} with {len} samples drifted"
                    );
                }
            }
        }
    }

    #[test]
    fn newest_sample_is_rightmost() {
        let t = theme();
        let g = Glyphs::new(true);
        let l = line(&[0, 0, 100], 3, Some(100), Tint::Flat, t.info, &t, &g);
        let s = text(&l);
        assert_eq!(s.chars().last().unwrap(), '█');
        assert_eq!(s.chars().next().unwrap(), '▁');
    }

    #[test]
    fn short_history_grows_in_from_the_right() {
        let t = theme();
        let g = Glyphs::new(true);
        let l = line(&[100], 5, Some(100), Tint::Flat, t.info, &t, &g);
        assert_eq!(text(&l), "    █");
    }

    #[test]
    fn long_history_keeps_the_newest_window() {
        let t = theme();
        let g = Glyphs::new(true);
        let data: Vec<u64> = (0..100).collect();
        let l = line(&data, 4, Some(99), Tint::Flat, t.info, &t, &g);
        // The last four samples are the highest ones.
        assert_eq!(text(&l), "████");
    }

    #[test]
    fn flat_series_does_not_divide_by_zero() {
        let t = theme();
        let g = Glyphs::new(true);
        let l = line(&[0, 0, 0, 0], 4, None, Tint::Flat, t.info, &t, &g);
        assert_eq!(text(&l).chars().count(), 4);
    }

    #[test]
    fn load_tint_colors_by_threshold() {
        let t = theme();
        let g = Glyphs::new(true);
        let l = line(&[10, 60, 95], 3, Some(100), Tint::Load, t.info, &t, &g);
        let colors: Vec<_> = l.spans.iter().filter_map(|s| s.style.fg).collect();
        assert_eq!(colors, vec![t.success, t.warning, t.danger]);
    }

    #[test]
    fn flat_tint_uses_one_color() {
        let t = theme();
        let g = Glyphs::new(true);
        let l = line(
            &[10, 60, 95],
            3,
            Some(100),
            Tint::Flat,
            t.accent_secondary,
            &t,
            &g,
        );
        for span in &l.spans {
            assert_eq!(span.style.fg, Some(t.accent_secondary));
        }
    }

    #[test]
    fn ascii_mode_stays_ascii() {
        let t = theme();
        let g = Glyphs::new(false);
        let l = line(
            &[0, 25, 50, 75, 100],
            5,
            Some(100),
            Tint::Load,
            t.info,
            &t,
            &g,
        );
        assert!(text(&l).is_ascii(), "ASCII sparkline leaked: {}", text(&l));
    }

    #[test]
    fn percent_helper_clamps_out_of_range_values() {
        let t = theme();
        let g = Glyphs::new(true);
        let l = line_percent(&[-5.0, 50.0, 150.0, f32::NAN], 4, &t, &g);
        assert_eq!(text(&l).chars().count(), 4);
    }
}
