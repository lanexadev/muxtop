// Glyph table — every non-ASCII character muxtop can draw, plus its fallback.
//
// muxtop is expected to run on a kitty window, inside tmux over ssh, and on the
// bare kernel console of a headless Ubuntu box that renders through a 256-glyph
// bitmap font. Views must never hardcode a Unicode character: they pick one
// from [`Glyphs`], which is resolved once from [`crate::terminal::TermCaps`].
//
// Rules for anything added here:
//   * BMP only, single-width. No emoji, no variation selectors — a double-width
//     glyph silently shifts every column to its right.
//   * The ASCII fallback must carry the same *meaning*, not merely fill space.

use ratatui::symbols::border;
use ratatui::widgets::BorderType;

/// Resolved glyph set for the running terminal.
#[derive(Debug, Clone, Copy)]
pub struct Glyphs {
    /// Fully-filled meter cell.
    pub meter_full: &'static str,
    /// Empty meter cell (the track).
    pub meter_empty: &'static str,
    /// Partial meter cells, 1/8 through 7/8 of a cell. Empty in ASCII mode,
    /// where meters quantise to whole cells.
    pub meter_partials: &'static [&'static str],
    /// Sparkline levels, lowest to highest. Always 8 entries.
    pub spark: &'static [&'static str; 8],
    /// Descending / ascending sort indicator.
    pub sort_desc: &'static str,
    pub sort_asc: &'static str,
    /// Left edge marker on the selected row — the one selection cue that
    /// survives a terminal with no usable background colors.
    pub sel_edge: &'static str,
    /// Scrollbar thumb / track.
    pub scroll_thumb: &'static str,
    pub scroll_track: &'static str,
    /// Tree connectors: mid-branch, last branch, vertical continuation, gap.
    pub tree_branch: &'static str,
    pub tree_last: &'static str,
    pub tree_pipe: &'static str,
    pub tree_gap: &'static str,
    /// Process / container / pod state markers.
    pub st_running: &'static str,
    pub st_idle: &'static str,
    pub st_paused: &'static str,
    pub st_warn: &'static str,
    pub st_dead: &'static str,
    pub st_unknown: &'static str,
    /// Traffic direction markers.
    pub arrow_up: &'static str,
    pub arrow_down: &'static str,
    /// Right-pointing marker (sub-view chevrons, menu affordances).
    pub chevron: &'static str,
    /// Spinner frames for in-flight work.
    pub spinner: &'static [&'static str],
    /// Inline separator between status-bar segments.
    pub sep: &'static str,
    /// Text cursor in input fields.
    pub cursor: &'static str,
    /// Connection state markers: local, remote, disconnected.
    pub conn_local: &'static str,
    pub conn_remote: &'static str,
    pub conn_down: &'static str,
    /// Truncation marker. One cell wide in both modes ("…" vs "~").
    pub ellipsis: &'static str,
    /// Value that could not be measured (no metrics-server, no cgroup limit).
    pub none: &'static str,
    /// Separator inside a title: `Process — nginx`.
    pub dash: &'static str,
    /// Whether this set is the Unicode one. Views should branch on a specific
    /// glyph rather than on this flag; it exists for layout maths that depend
    /// on having partial meter cells.
    pub unicode: bool,
}

const UNICODE_PARTIALS: &[&str] = &["▏", "▎", "▍", "▌", "▋", "▊", "▉"];
const ASCII_PARTIALS: &[&str] = &[];

const UNICODE_SPARK: &[&str; 8] = &["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
// Five distinguishable ASCII heights, padded to 8. `.` reads as "near zero"
// and `#` as "full" without needing a legend.
const ASCII_SPARK: &[&str; 8] = &[".", ".", ":", ":", "|", "|", "#", "#"];

const UNICODE_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const ASCII_SPINNER: &[&str] = &["-", "\\", "|", "/"];

/// The Unicode glyph set. Braille appears only in the spinner, which is
/// decorative — every other glyph is box drawing or block elements, the two
/// ranges terminal fonts cover most reliably.
pub const UNICODE: Glyphs = Glyphs {
    meter_full: "█",
    meter_empty: " ",
    meter_partials: UNICODE_PARTIALS,
    spark: UNICODE_SPARK,
    sort_desc: "▼",
    sort_asc: "▲",
    sel_edge: "▎",
    scroll_thumb: "█",
    scroll_track: "│",
    tree_branch: "├── ",
    tree_last: "└── ",
    tree_pipe: "│   ",
    tree_gap: "    ",
    st_running: "●",
    st_idle: "○",
    st_paused: "◐",
    st_warn: "▲",
    st_dead: "✕",
    st_unknown: "?",
    arrow_up: "↑",
    arrow_down: "↓",
    chevron: "›",
    spinner: UNICODE_SPINNER,
    sep: "·",
    cursor: "█",
    conn_local: "●",
    conn_remote: "◆",
    conn_down: "✕",
    ellipsis: "…",
    none: "—",
    dash: "—",
    unicode: true,
};

/// The ASCII fallback. Everything here is printable 7-bit ASCII, so it renders
/// on the Linux virtual console, a vt220, a serial link, and any locale.
pub const ASCII: Glyphs = Glyphs {
    meter_full: "|",
    meter_empty: " ",
    meter_partials: ASCII_PARTIALS,
    spark: ASCII_SPARK,
    sort_desc: "v",
    sort_asc: "^",
    sel_edge: ">",
    scroll_thumb: "#",
    scroll_track: "|",
    tree_branch: "|-- ",
    tree_last: "\\-- ",
    tree_pipe: "|   ",
    tree_gap: "    ",
    st_running: "R",
    st_idle: "S",
    st_paused: "T",
    st_warn: "!",
    st_dead: "X",
    st_unknown: "?",
    arrow_up: "^",
    arrow_down: "v",
    chevron: ">",
    spinner: ASCII_SPINNER,
    sep: "-",
    cursor: "_",
    conn_local: "*",
    conn_remote: "@",
    conn_down: "X",
    ellipsis: "~",
    none: "-",
    dash: "-",
    unicode: false,
};

/// Pure-ASCII box drawing, for terminals whose font has no line-drawing range.
const ASCII_BORDER: border::Set = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

impl Glyphs {
    /// Resolve the glyph set for a terminal.
    pub const fn new(unicode: bool) -> Self {
        if unicode { UNICODE } else { ASCII }
    }

    /// Border symbols for panels.
    pub const fn border_set(&self) -> border::Set<'static> {
        if self.unicode {
            border::ROUNDED
        } else {
            ASCII_BORDER
        }
    }

    /// Border type, for the call sites that use `Block::border_type`.
    pub const fn border_type(&self) -> BorderType {
        if self.unicode {
            BorderType::Rounded
        } else {
            BorderType::Plain
        }
    }

    /// Spinner frame for a monotonically increasing tick counter.
    pub fn spinner_frame(&self, tick: usize) -> &'static str {
        self.spinner[tick % self.spinner.len()]
    }

    /// Sparkline glyph for `value` scaled against `max`.
    ///
    /// A zero value always renders as the lowest glyph rather than a blank, so
    /// an idle interface still shows a baseline instead of a hole in the chart.
    pub fn spark_level(&self, value: u64, max: u64) -> &'static str {
        if max == 0 {
            return self.spark[0];
        }
        let idx = ((value.min(max) as f64 / max as f64) * 7.0).round() as usize;
        self.spark[idx.min(7)]
    }

    /// Truncate `s` to `width` display cells, appending the ellipsis glyph when
    /// characters were dropped.
    ///
    /// Counts `char`s rather than grapheme clusters: muxtop's variable-width
    /// strings are process names, image tags and Kubernetes identifiers, which
    /// are ASCII in practice, and this keeps the row loop allocation-free for
    /// the overwhelmingly common case where nothing needs truncating.
    pub fn truncate<'a>(&self, s: &'a str, width: usize) -> std::borrow::Cow<'a, str> {
        if width == 0 {
            return std::borrow::Cow::Borrowed("");
        }
        let len = s.chars().count();
        if len <= width {
            return std::borrow::Cow::Borrowed(s);
        }
        if width == 1 {
            return std::borrow::Cow::Borrowed(self.ellipsis);
        }
        let mut out: String = s.chars().take(width - 1).collect();
        out.push_str(self.ellipsis);
        std::borrow::Cow::Owned(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_set_is_pure_ascii() {
        // Every ASCII-mode glyph must survive a single-byte terminal.
        let g = ASCII;
        for s in [
            g.meter_full,
            g.meter_empty,
            g.sort_desc,
            g.sort_asc,
            g.sel_edge,
            g.scroll_thumb,
            g.scroll_track,
            g.tree_branch,
            g.tree_last,
            g.tree_pipe,
            g.st_running,
            g.st_idle,
            g.st_paused,
            g.st_warn,
            g.st_dead,
            g.arrow_up,
            g.arrow_down,
            g.chevron,
            g.sep,
            g.cursor,
            g.conn_local,
            g.conn_remote,
            g.conn_down,
            g.ellipsis,
            g.none,
            g.dash,
        ] {
            assert!(s.is_ascii(), "non-ASCII glyph in the ASCII set: {s:?}");
        }
        for s in g.spark {
            assert!(s.is_ascii(), "non-ASCII spark glyph: {s:?}");
        }
        for s in g.spinner {
            assert!(s.is_ascii(), "non-ASCII spinner glyph: {s:?}");
        }
    }

    #[test]
    fn ascii_border_is_pure_ascii() {
        let b = ASCII.border_set();
        for s in [
            b.top_left,
            b.top_right,
            b.bottom_left,
            b.bottom_right,
            b.vertical_left,
            b.vertical_right,
            b.horizontal_top,
            b.horizontal_bottom,
        ] {
            assert!(s.is_ascii(), "non-ASCII border glyph: {s:?}");
        }
    }

    #[test]
    fn every_glyph_is_one_cell() {
        // A two-cell glyph would silently shift every column to its right.
        for g in [UNICODE, ASCII] {
            for s in [
                g.meter_full,
                g.sort_desc,
                g.sort_asc,
                g.sel_edge,
                g.scroll_thumb,
                g.scroll_track,
                g.st_running,
                g.st_idle,
                g.st_paused,
                g.st_warn,
                g.st_dead,
                g.arrow_up,
                g.arrow_down,
                g.chevron,
                g.sep,
                g.cursor,
                g.conn_local,
                g.conn_remote,
                g.conn_down,
                g.ellipsis,
                g.none,
                g.dash,
            ] {
                assert_eq!(s.chars().count(), 1, "glyph is not a single char: {s:?}");
            }
            for s in g.spark {
                assert_eq!(s.chars().count(), 1, "spark glyph is not one char: {s:?}");
            }
            // Tree connectors are deliberately four cells wide.
            for s in [g.tree_branch, g.tree_last, g.tree_pipe, g.tree_gap] {
                assert_eq!(s.chars().count(), 4, "tree connector width drift: {s:?}");
            }
        }
    }

    #[test]
    fn spark_level_scales() {
        let g = UNICODE;
        assert_eq!(g.spark_level(0, 100), "▁");
        assert_eq!(g.spark_level(100, 100), "█");
        // Half of a 0–7 scale rounds up to index 4.
        assert_eq!(g.spark_level(50, 100), "▅");
        // A zero maximum must not divide by zero.
        assert_eq!(g.spark_level(0, 0), "▁");
        assert_eq!(g.spark_level(9, 0), "▁");
        // Values above max clamp instead of panicking on the index.
        assert_eq!(g.spark_level(500, 100), "█");
    }

    #[test]
    fn truncate_borrows_when_it_fits() {
        let g = UNICODE;
        assert!(matches!(
            g.truncate("nginx", 10),
            std::borrow::Cow::Borrowed("nginx")
        ));
        assert!(matches!(
            g.truncate("nginx", 5),
            std::borrow::Cow::Borrowed("nginx")
        ));
    }

    #[test]
    fn truncate_marks_elision() {
        let g = UNICODE;
        assert_eq!(g.truncate("postgres", 5), "post…");
        assert_eq!(g.truncate("postgres", 1), "…");
        assert_eq!(g.truncate("postgres", 0), "");
        assert_eq!(ASCII.truncate("postgres", 5), "post~");
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        let g = UNICODE;
        // Eight chars, sixteen bytes: must not slice mid-codepoint.
        assert_eq!(g.truncate("ééééééée", 4), "ééé…");
    }

    #[test]
    fn spinner_frame_wraps() {
        let g = UNICODE;
        assert_eq!(g.spinner_frame(0), g.spinner_frame(g.spinner.len()));
        // Must not panic for a long-running session's tick count.
        let _ = g.spinner_frame(usize::MAX);
    }

    #[test]
    fn new_selects_the_right_set() {
        assert!(Glyphs::new(true).unicode);
        assert!(!Glyphs::new(false).unicode);
        assert_eq!(Glyphs::new(false).meter_full, "|");
    }
}
