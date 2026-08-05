// Load meters.
//
// Keeps muxtop 0.4's htop-style *zone* colouring — the fill is green over the
// 0–50% region, yellow over 50–80%, red beyond, so the colour tells you where
// the bar is rather than restating its length — and adds block glyphs with
// sub-cell resolution on terminals that can draw them.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// Where the green zone ends, as a fraction of the bar.
const ZONE_WARN: f64 = 0.50;
/// Where the yellow zone ends.
const ZONE_DANGER: f64 = 0.80;

/// Build an htop-style bar: `LABEL [||||||||        info]`.
///
/// `info` is right-aligned inside the brackets and the fill never overwrites
/// it. Returns an empty line when there is no room to draw anything.
pub fn bar_line(
    label: &str,
    label_width: usize,
    info: &str,
    percent: f64,
    width: u16,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Line<'static> {
    let width = width as usize;
    if width <= label_width + 2 {
        return Line::from(Span::styled(
            crate::ui::widgets::columns::cell(
                label,
                width as u16,
                crate::ui::widgets::Align::Left,
                glyphs,
            ),
            theme.strong(),
        ));
    }

    let bar_w = width - label_width - 2; // minus the two brackets
    let info = glyphs.truncate(info, bar_w);
    let info_len = info.chars().count();
    let track = bar_w.saturating_sub(info_len);

    let mut spans = Vec::with_capacity(8);
    spans.push(Span::styled(
        format!("{label:<label_width$}"),
        theme.strong(),
    ));
    spans.push(Span::styled("[", theme.dim()));
    spans.extend(fill_spans(percent, track, theme, glyphs));
    spans.push(Span::styled(info.into_owned(), theme.dim()));
    spans.push(Span::styled("]", theme.dim()));
    Line::from(spans)
}

/// A bracket-less meter for compact contexts (header vitals, dashboard cards).
pub fn inline(percent: f64, width: u16, theme: &Theme, glyphs: &Glyphs) -> Vec<Span<'static>> {
    fill_spans(percent, width as usize, theme, glyphs)
}

/// Render the fill itself: zone-coloured cells followed by the empty track.
fn fill_spans(percent: f64, track: usize, theme: &Theme, glyphs: &Glyphs) -> Vec<Span<'static>> {
    if track == 0 {
        return Vec::new();
    }
    let percent = percent.clamp(0.0, 100.0);
    let exact = track as f64 * percent / 100.0;
    let full = (exact.floor() as usize).min(track);
    // Sub-cell remainder, only drawable where partial block glyphs exist.
    let partial = if glyphs.meter_partials.is_empty() || full >= track {
        None
    } else {
        let frac = exact - full as f64;
        let step = (frac * 8.0).round() as usize;
        (1..=7)
            .contains(&step)
            .then(|| glyphs.meter_partials[step - 1])
    };

    // Zone boundaries in whole cells.
    let warn_at = (track as f64 * ZONE_WARN).round() as usize;
    let danger_at = (track as f64 * ZONE_DANGER).round() as usize;

    let green = full.min(warn_at);
    let yellow = full
        .saturating_sub(warn_at)
        .min(danger_at.saturating_sub(warn_at));
    let red = full.saturating_sub(danger_at);

    let mut spans = Vec::with_capacity(5);
    let mut push = |n: usize, color| {
        if n > 0 {
            spans.push(Span::styled(
                glyphs.meter_full.repeat(n),
                Style::default().fg(color),
            ));
        }
    };
    push(green, theme.success);
    push(yellow, theme.warning);
    push(red, theme.danger);

    let mut used = full;
    if let Some(p) = partial {
        // The partial cell belongs to whichever zone it lands in.
        let color = zone_color(full, warn_at, danger_at, theme);
        spans.push(Span::styled(p.to_string(), Style::default().fg(color)));
        used += 1;
    }

    let empty = track.saturating_sub(used);
    if empty > 0 {
        spans.push(Span::styled(
            glyphs.meter_empty.repeat(empty),
            Style::default().fg(theme.bar_empty),
        ));
    }
    spans
}

fn zone_color(
    cell_index: usize,
    warn_at: usize,
    danger_at: usize,
    theme: &Theme,
) -> ratatui::style::Color {
    if cell_index >= danger_at {
        theme.danger
    } else if cell_index >= warn_at {
        theme.warning
    } else {
        theme.success
    }
}

/// Format a byte count as a compact human-readable string (`1.2G`, `340M`).
pub fn human_bytes(bytes: u64) -> String {
    // Petabytes are not hypothetical on a storage host, and a column sized for
    // "999G" must not be blown out by a value that needed a wider unit.
    const UNITS: [(u64, &str); 6] = [
        (1 << 50, "P"),
        (1 << 40, "T"),
        (1 << 30, "G"),
        (1 << 20, "M"),
        (1 << 10, "K"),
        (1, "B"),
    ];
    for (scale, suffix) in UNITS {
        if bytes >= scale {
            let value = bytes as f64 / scale as f64;
            return if value >= 100.0 || scale == 1 {
                format!("{value:.0}{suffix}")
            } else {
                format!("{value:.1}{suffix}")
            };
        }
    }
    "0B".to_string()
}

/// Format a byte-per-second rate (`1.2M/s`).
pub fn human_rate(bytes_per_sec: u64) -> String {
    format!("{}/s", human_bytes(bytes_per_sec))
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
    fn bar_is_exactly_the_requested_width() {
        let t = theme();
        for glyphs in [Glyphs::new(true), Glyphs::new(false)] {
            for width in 10u16..60 {
                for pct in [0.0, 1.0, 33.3, 50.0, 79.9, 99.9, 100.0] {
                    let line = bar_line("cpu0", 6, "42.0%", pct, width, &t, &glyphs);
                    assert_eq!(
                        text(&line).chars().count(),
                        width as usize,
                        "width {width} at {pct}% drifted"
                    );
                }
            }
        }
    }

    #[test]
    fn bar_survives_a_width_smaller_than_its_label() {
        let t = theme();
        let g = Glyphs::new(true);
        for width in 0u16..9 {
            let line = bar_line("cpu0", 6, "42.0%", 50.0, width, &t, &g);
            assert_eq!(text(&line).chars().count(), width as usize);
        }
    }

    #[test]
    fn fill_never_overwrites_the_info_label() {
        let t = theme();
        let g = Glyphs::new(true);
        let line = bar_line("Mem", 5, "100%  16.0/16.0G", 100.0, 40, &t, &g);
        assert!(
            text(&line).contains("100%  16.0/16.0G"),
            "a full bar must not eat its own readout: {}",
            text(&line)
        );
    }

    #[test]
    fn zone_colors_follow_the_load() {
        let t = theme();
        let g = Glyphs::new(true);
        let colors = |pct: f64| -> Vec<ratatui::style::Color> {
            fill_spans(pct, 20, &t, &g)
                .iter()
                .filter_map(|s| s.style.fg)
                .collect()
        };
        // A quarter-full bar is green only.
        assert!(colors(25.0).contains(&t.success));
        assert!(!colors(25.0).contains(&t.warning));
        // Past the halfway mark yellow appears, but not red.
        assert!(colors(65.0).contains(&t.warning));
        assert!(!colors(65.0).contains(&t.danger));
        // A nearly-full bar shows all three zones.
        let hot = colors(95.0);
        assert!(hot.contains(&t.success) && hot.contains(&t.warning) && hot.contains(&t.danger));
    }

    #[test]
    fn empty_and_full_bars_are_not_confused() {
        let t = theme();
        let g = Glyphs::new(true);
        let empty = fill_spans(0.0, 10, &t, &g);
        assert!(
            empty.iter().all(|s| s.content.trim().is_empty()),
            "a 0% bar must draw no fill"
        );
        let full: String = fill_spans(100.0, 10, &t, &g)
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(full, "█".repeat(10));
    }

    #[test]
    fn ascii_mode_never_emits_partial_blocks() {
        let t = theme();
        let g = Glyphs::new(false);
        for pct in [12.5, 33.3, 47.9, 66.6, 91.2] {
            let s: String = fill_spans(pct, 13, &t, &g)
                .iter()
                .map(|x| x.content.as_ref())
                .collect();
            assert!(s.is_ascii(), "ASCII meter leaked a block glyph: {s:?}");
        }
    }

    #[test]
    fn out_of_range_percentages_clamp() {
        let t = theme();
        let g = Glyphs::new(true);
        for pct in [-50.0, -0.1, 100.1, 1e9] {
            let s: String = fill_spans(pct, 10, &t, &g)
                .iter()
                .map(|x| x.content.as_ref())
                .collect();
            assert_eq!(s.chars().count(), 10, "clamping failed at {pct}");
        }
    }

    #[test]
    fn zero_width_meter_draws_nothing() {
        let t = theme();
        let g = Glyphs::new(true);
        assert!(fill_spans(50.0, 0, &t, &g).is_empty());
        assert!(inline(50.0, 0, &t, &g).is_empty());
    }

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(0), "0B");
        assert_eq!(human_bytes(512), "512B");
        assert_eq!(human_bytes(1536), "1.5K");
        assert_eq!(human_bytes(5 * (1 << 20)), "5.0M");
        assert_eq!(human_bytes(150 * (1 << 20)), "150M");
        assert_eq!(human_bytes(3 * (1 << 30)), "3.0G");
    }

    #[test]
    fn human_bytes_stays_short() {
        // Column widths depend on this staying within five cells.
        for b in [0u64, 1, 1023, 1024, u64::MAX / 2, u64::MAX] {
            assert!(
                human_bytes(b).chars().count() <= 6,
                "human_bytes({b}) is too wide: {}",
                human_bytes(b)
            );
        }
    }

    #[test]
    fn human_rate_appends_per_second() {
        assert_eq!(human_rate(1536), "1.5K/s");
    }
}
