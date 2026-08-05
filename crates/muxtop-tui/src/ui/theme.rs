// Theme — three layers: primitive ramp, semantic tokens, component styles.
//
// Views only ever read semantic tokens or the style helpers at the bottom of
// this file. They never name a primitive colour, which is what makes adding a
// theme a small table rather than a sweep through every view.
//
// Every theme is resolved for four colour depths, because muxtop is expected to
// run on all of them: 24-bit in a modern terminal, 256 in Terminal.app or a
// default `xterm-256color` ssh session, 16 on the Linux virtual console, and
// none at all under `NO_COLOR` or `TERM=dumb`.

use std::fmt;
use std::str::FromStr;

use ratatui::style::{Color, Modifier, Style};

use crate::terminal::ColorSupport;

/// Severity of a message or a state badge. Replaces inferring severity from
/// message text, which silently painted failures green whenever the wording
/// changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Info,
    Success,
    Warning,
    Error,
    /// Deliberately unremarkable — a resting state, not news.
    Neutral,
}

/// Selectable colour scheme.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemeKind {
    /// Tokyo Night — muxtop's signature dark theme.
    #[default]
    TokyoNight,
    /// Tokyo Night Day — for light terminal backgrounds.
    TokyoNightLight,
    /// No hue at all: hierarchy through bold / dim / reverse only. For
    /// monochrome terminals, e-ink, and colour schemes that fight ours.
    Mono,
}

impl ThemeKind {
    pub const ALL: &'static [ThemeKind] = &[
        ThemeKind::TokyoNight,
        ThemeKind::TokyoNightLight,
        ThemeKind::Mono,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::TokyoNight => "tokyo-night",
            ThemeKind::TokyoNightLight => "tokyo-night-light",
            ThemeKind::Mono => "mono",
        }
    }

    /// Names accepted by `--theme`, for the CLI error message.
    pub fn names() -> String {
        Self::ALL
            .iter()
            .map(|k| k.name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for ThemeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for ThemeKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let normalised = s.trim().to_lowercase().replace('_', "-");
        match normalised.as_str() {
            "tokyo-night" | "tokyonight" | "dark" | "default" => Ok(ThemeKind::TokyoNight),
            "tokyo-night-light" | "tokyonight-light" | "light" | "day" => {
                Ok(ThemeKind::TokyoNightLight)
            }
            "mono" | "monochrome" | "none" => Ok(ThemeKind::Mono),
            other => Err(format!(
                "unknown theme `{other}` (available: {})",
                ThemeKind::names()
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Layer 1 — primitive ramp
// ---------------------------------------------------------------------------

/// The raw colours of a theme, before they mean anything.
///
/// One ramp exists per (theme, colour depth) pair. Nothing outside this file
/// reads a `Ramp` field directly.
#[derive(Debug, Clone, Copy)]
struct Ramp {
    bg: Color,
    bg_alt: Color,
    surface: Color,
    overlay: Color,
    border: Color,
    fg: Color,
    fg_muted: Color,
    fg_subtle: Color,
    fg_inverse: Color,
    sel_bg: Color,
    sel_fg: Color,
    cyan: Color,
    purple: Color,
    green: Color,
    yellow: Color,
    red: Color,
    blue: Color,
}

/// Tokyo Night, 24-bit.
const TOKYO_NIGHT_RGB: Ramp = Ramp {
    bg: Color::Rgb(26, 27, 38),          // #1a1b26
    bg_alt: Color::Rgb(30, 32, 48),      // #1e2030
    surface: Color::Rgb(36, 40, 59),     // #24283b
    overlay: Color::Rgb(41, 46, 66),     // #292e42
    border: Color::Rgb(59, 66, 97),      // #3b4261
    fg: Color::Rgb(192, 202, 245),       // #c0caf5
    fg_muted: Color::Rgb(134, 145, 190), // #8691be
    fg_subtle: Color::Rgb(86, 95, 137),  // #565f89
    fg_inverse: Color::Rgb(26, 27, 38),  // #1a1b26
    sel_bg: Color::Rgb(55, 65, 115),     // #374173
    sel_fg: Color::Rgb(255, 255, 255),
    cyan: Color::Rgb(125, 207, 255),   // #7dcfff
    purple: Color::Rgb(187, 154, 247), // #bb9af7
    green: Color::Rgb(158, 206, 106),  // #9ece6a
    yellow: Color::Rgb(224, 175, 104), // #e0af68
    red: Color::Rgb(247, 118, 142),    // #f7768e
    blue: Color::Rgb(122, 162, 247),   // #7aa2f7
};

/// Tokyo Night, xterm-256 approximation.
///
/// This is the level a default `ssh user@server` session lands on — `TERM` is
/// `xterm-256color` and `COLORTERM` is stripped — so it is worth getting right
/// rather than falling through to 16 colours.
const TOKYO_NIGHT_256: Ramp = Ramp {
    bg: Color::Indexed(234),
    bg_alt: Color::Indexed(235),
    surface: Color::Indexed(236),
    overlay: Color::Indexed(237),
    border: Color::Indexed(238),
    fg: Color::Indexed(189),
    fg_muted: Color::Indexed(146),
    fg_subtle: Color::Indexed(60),
    fg_inverse: Color::Indexed(234),
    sel_bg: Color::Indexed(60),
    sel_fg: Color::Indexed(255),
    cyan: Color::Indexed(117),
    purple: Color::Indexed(141),
    green: Color::Indexed(149),
    yellow: Color::Indexed(179),
    red: Color::Indexed(210),
    blue: Color::Indexed(111),
};

/// Tokyo Night Day, 24-bit.
const TOKYO_DAY_RGB: Ramp = Ramp {
    bg: Color::Rgb(225, 226, 231),         // #e1e2e7
    bg_alt: Color::Rgb(213, 214, 219),     // #d5d6db
    surface: Color::Rgb(200, 202, 212),    // #c8cad4
    overlay: Color::Rgb(183, 193, 227),    // #b7c1e3
    border: Color::Rgb(160, 166, 189),     // #a0a6bd
    fg: Color::Rgb(52, 59, 88),            // #343b58
    fg_muted: Color::Rgb(101, 109, 145),   // #656d91
    fg_subtle: Color::Rgb(132, 140, 181),  // #848cb5
    fg_inverse: Color::Rgb(225, 226, 231), // #e1e2e7
    sel_bg: Color::Rgb(183, 193, 227),     // #b7c1e3
    sel_fg: Color::Rgb(26, 27, 38),        // #1a1b26
    cyan: Color::Rgb(0, 113, 151),         // #007197
    purple: Color::Rgb(120, 71, 189),      // #7847bd
    green: Color::Rgb(88, 117, 57),        // #587539
    yellow: Color::Rgb(140, 108, 62),      // #8c6c3e
    red: Color::Rgb(196, 26, 74),          // #c41a4a
    blue: Color::Rgb(45, 91, 179),         // #2d5bb3
};

/// Tokyo Night Day, xterm-256 approximation.
const TOKYO_DAY_256: Ramp = Ramp {
    bg: Color::Indexed(255),
    bg_alt: Color::Indexed(254),
    surface: Color::Indexed(253),
    overlay: Color::Indexed(252),
    border: Color::Indexed(247),
    fg: Color::Indexed(236),
    fg_muted: Color::Indexed(60),
    fg_subtle: Color::Indexed(103),
    fg_inverse: Color::Indexed(255),
    sel_bg: Color::Indexed(152),
    sel_fg: Color::Indexed(232),
    cyan: Color::Indexed(31),
    purple: Color::Indexed(91),
    green: Color::Indexed(64),
    yellow: Color::Indexed(136),
    red: Color::Indexed(125),
    blue: Color::Indexed(26),
};

/// 16-colour ANSI. The terminal's own palette decides the actual hues, which is
/// the correct behaviour: we are a guest in the user's colour scheme here.
const ANSI_DARK: Ramp = Ramp {
    bg: Color::Reset,
    bg_alt: Color::Reset,
    surface: Color::Black,
    overlay: Color::Black,
    border: Color::DarkGray,
    fg: Color::Gray,
    fg_muted: Color::DarkGray,
    fg_subtle: Color::DarkGray,
    fg_inverse: Color::Black,
    sel_bg: Color::Blue,
    sel_fg: Color::White,
    cyan: Color::Cyan,
    purple: Color::Magenta,
    green: Color::Green,
    yellow: Color::Yellow,
    red: Color::Red,
    blue: Color::Blue,
};

/// No colour at all. Every token is `Reset`; the style helpers carry the whole
/// visual hierarchy through BOLD / DIM / REVERSED, which is why views must use
/// them rather than assembling `Style::default().fg(...)` by hand.
const NO_COLOR: Ramp = Ramp {
    bg: Color::Reset,
    bg_alt: Color::Reset,
    surface: Color::Reset,
    overlay: Color::Reset,
    border: Color::Reset,
    fg: Color::Reset,
    fg_muted: Color::Reset,
    fg_subtle: Color::Reset,
    fg_inverse: Color::Reset,
    sel_bg: Color::Reset,
    sel_fg: Color::Reset,
    cyan: Color::Reset,
    purple: Color::Reset,
    green: Color::Reset,
    yellow: Color::Reset,
    red: Color::Reset,
    blue: Color::Reset,
};

fn ramp_for(kind: ThemeKind, support: ColorSupport) -> Ramp {
    match (kind, support) {
        (_, ColorSupport::NoColor) | (ThemeKind::Mono, _) => NO_COLOR,
        (ThemeKind::TokyoNight, ColorSupport::TrueColor) => TOKYO_NIGHT_RGB,
        (ThemeKind::TokyoNight, ColorSupport::Colors256) => TOKYO_NIGHT_256,
        (ThemeKind::TokyoNight, ColorSupport::Basic) => ANSI_DARK,
        (ThemeKind::TokyoNightLight, ColorSupport::TrueColor) => TOKYO_DAY_RGB,
        (ThemeKind::TokyoNightLight, ColorSupport::Colors256) => TOKYO_DAY_256,
        (ThemeKind::TokyoNightLight, ColorSupport::Basic) => ANSI_DARK,
    }
}

// ---------------------------------------------------------------------------
// Layer 2 — semantic tokens
// ---------------------------------------------------------------------------

/// Resolved theme for the running terminal.
///
/// The field names from muxtop 0.4 are kept as-is so existing views keep
/// compiling; the newer semantic names sit alongside them.
#[derive(Debug, Clone)]
pub struct Theme {
    // -- surfaces --
    pub bg: Color,
    /// Alternating row background (zebra striping).
    pub surface: Color,
    /// Chrome background: header line, table header row, status bar.
    pub header_bg: Color,
    /// Overlay background: palette, help, confirm, inspector.
    pub overlay_bg: Color,
    /// Panel borders at rest / when the panel holds focus.
    pub border: Color,
    pub border_focus: Color,

    // -- text --
    pub fg: Color,
    /// De-emphasised text: units, secondary values, inactive tabs.
    pub text_dim: Color,
    /// Barely-there text: hints, disabled entries.
    pub text_subtle: Color,
    /// Text drawn on top of an accent fill.
    pub fg_inverse: Color,

    // -- accents --
    pub accent_primary: Color,
    pub accent_secondary: Color,

    // -- selection --
    pub selection_bg: Color,
    pub selection_fg: Color,
    /// Left edge marker colour on the selected row.
    pub selection_edge: Color,

    // -- status --
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,
    /// Resting state (sleeping processes, exited containers).
    pub sleeping: Color,

    // -- meters --
    /// Unfilled part of a meter.
    pub bar_empty: Color,
    pub scrollbar_thumb: Color,
    pub scrollbar_track: Color,

    /// True when the terminal gives us no colour, so the style helpers must
    /// carry hierarchy with text attributes instead.
    mono: bool,
    /// True when the theme is designed for a light terminal background.
    light: bool,
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ColorSupport::TrueColor)
    }
}

impl Theme {
    /// Resolve the default theme for a colour depth.
    pub fn new(support: ColorSupport) -> Self {
        Self::with_kind(ThemeKind::default(), support)
    }

    /// Resolve a specific theme for a colour depth.
    pub fn with_kind(kind: ThemeKind, support: ColorSupport) -> Self {
        let r = ramp_for(kind, support);
        let mono = support == ColorSupport::NoColor || kind == ThemeKind::Mono;
        Self {
            bg: r.bg,
            surface: r.bg_alt,
            header_bg: r.surface,
            overlay_bg: r.bg_alt,
            border: r.border,
            border_focus: r.cyan,

            fg: r.fg,
            text_dim: r.fg_muted,
            text_subtle: r.fg_subtle,
            fg_inverse: r.fg_inverse,

            accent_primary: r.cyan,
            accent_secondary: r.purple,

            selection_bg: r.sel_bg,
            selection_fg: r.sel_fg,
            selection_edge: r.cyan,

            success: r.green,
            warning: r.yellow,
            danger: r.red,
            info: r.blue,
            sleeping: r.fg_subtle,

            bar_empty: r.overlay,
            scrollbar_thumb: r.fg_subtle,
            scrollbar_track: r.overlay,

            mono,
            light: kind == ThemeKind::TokyoNightLight,
        }
    }

    /// Whether the terminal gives us no colour to work with.
    pub fn is_mono(&self) -> bool {
        self.mono
    }

    /// Whether this theme targets a light background.
    pub fn is_light(&self) -> bool {
        self.light
    }

    /// Colour for a load gauge: green below 50%, yellow to 80%, red above.
    pub fn gauge_color(&self, percent: f64) -> Color {
        if percent >= 80.0 {
            self.danger
        } else if percent >= 50.0 {
            self.warning
        } else {
            self.success
        }
    }

    /// Colour carrying a severity.
    pub fn level_color(&self, level: Level) -> Color {
        match level {
            Level::Info => self.info,
            Level::Success => self.success,
            Level::Warning => self.warning,
            Level::Error => self.danger,
            Level::Neutral => self.text_dim,
        }
    }

    // -- Layer 3: component styles -----------------------------------------
    //
    // Every one of these degrades to a text attribute when there is no colour,
    // so the UI stays legible under `NO_COLOR` and on `TERM=dumb`.

    /// Body text.
    pub fn body(&self) -> Style {
        Style::default().fg(self.fg).bg(self.bg)
    }

    /// De-emphasised text.
    pub fn dim(&self) -> Style {
        let s = Style::default().fg(self.text_dim);
        if self.mono {
            s.add_modifier(Modifier::DIM)
        } else {
            s
        }
    }

    /// Barely-there text: inline hints, disabled commands.
    pub fn subtle(&self) -> Style {
        let s = Style::default().fg(self.text_subtle);
        if self.mono {
            s.add_modifier(Modifier::DIM)
        } else {
            s
        }
    }

    /// Emphasised text that is not an accent.
    pub fn strong(&self) -> Style {
        Style::default().fg(self.fg).add_modifier(Modifier::BOLD)
    }

    /// Accented text: panel titles, active sort column, key names.
    pub fn accent(&self) -> Style {
        Style::default()
            .fg(self.accent_primary)
            .add_modifier(Modifier::BOLD)
    }

    /// A filled accent chip: the brand tag, the active tab marker.
    pub fn accent_fill(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.fg_inverse)
                .bg(self.accent_primary)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// The chrome background used by the header line and the status bar.
    pub fn chrome(&self) -> Style {
        Style::default().fg(self.fg).bg(self.header_bg)
    }

    /// A table's column-header row.
    pub fn table_header(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default()
                .fg(self.accent_primary)
                .bg(self.header_bg)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// The selected row.
    pub fn selected_row(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
                .fg(self.selection_fg)
                .bg(self.selection_bg)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// A normal row, alternating background for odd rows.
    ///
    /// Zebra striping is dropped without colour: `REVERSED` on every other row
    /// would be unreadable, so mono relies on the selection marker alone.
    pub fn row(&self, odd: bool) -> Style {
        if self.mono {
            Style::default()
        } else if odd {
            Style::default().fg(self.fg).bg(self.surface)
        } else {
            Style::default().fg(self.fg).bg(self.bg)
        }
    }

    /// A severity-coloured message or badge.
    pub fn level_style(&self, level: Level) -> Style {
        let s = Style::default().fg(self.level_color(level));
        match (self.mono, level) {
            (true, Level::Error) => s.add_modifier(Modifier::BOLD | Modifier::REVERSED),
            (true, Level::Warning) => s.add_modifier(Modifier::BOLD),
            (true, Level::Neutral) => s.add_modifier(Modifier::DIM),
            _ => s,
        }
    }

    /// A filled severity badge — used for toasts and container/pod states.
    pub fn level_fill(&self, level: Level) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        } else {
            Style::default()
                .fg(self.fg_inverse)
                .bg(self.level_color(level))
                .add_modifier(Modifier::BOLD)
        }
    }

    /// Panel border, brighter when the panel holds focus.
    pub fn border_style(&self, focused: bool) -> Style {
        if self.mono {
            if focused {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            }
        } else if focused {
            Style::default().fg(self.border_focus)
        } else {
            Style::default().fg(self.border)
        }
    }

    /// A key name in a hint or the help screen.
    pub fn key(&self) -> Style {
        if self.mono {
            Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            Style::default()
                .fg(self.fg)
                .bg(self.selection_bg)
                .add_modifier(Modifier::BOLD)
        }
    }

    /// The description that follows a key name.
    pub fn key_desc(&self) -> Style {
        if self.mono {
            Style::default()
        } else {
            Style::default().fg(self.text_dim).bg(self.header_bg)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_supports() -> [ColorSupport; 4] {
        [
            ColorSupport::TrueColor,
            ColorSupport::Colors256,
            ColorSupport::Basic,
            ColorSupport::NoColor,
        ]
    }

    #[test]
    fn every_theme_resolves_at_every_depth() {
        for &kind in ThemeKind::ALL {
            for support in all_supports() {
                let _ = Theme::with_kind(kind, support);
            }
        }
    }

    #[test]
    fn colors256_gets_indexed_colors_not_the_ansi_fallback() {
        // Regression: 0.4 detected Colors256 and then handed it the 16-colour
        // ramp, so Terminal.app and a default ssh session both lost the theme.
        let t = Theme::with_kind(ThemeKind::TokyoNight, ColorSupport::Colors256);
        assert!(
            matches!(t.accent_primary, Color::Indexed(_)),
            "256-colour terminals must get indexed colours, got {:?}",
            t.accent_primary
        );
        assert!(matches!(t.fg, Color::Indexed(_)));
        assert!(matches!(t.danger, Color::Indexed(_)));
    }

    #[test]
    fn truecolor_gets_rgb() {
        let t = Theme::with_kind(ThemeKind::TokyoNight, ColorSupport::TrueColor);
        assert_eq!(t.accent_primary, Color::Rgb(125, 207, 255));
        assert_eq!(t.fg, Color::Rgb(192, 202, 245));
    }

    #[test]
    fn no_color_emits_no_color_at_all() {
        // Regression: 0.4 gave NoColor terminals the 16-colour ramp, so
        // `NO_COLOR=1` and `TERM=dumb` still got Cyan and Green.
        let t = Theme::new(ColorSupport::NoColor);
        for c in [
            t.bg,
            t.fg,
            t.text_dim,
            t.text_subtle,
            t.accent_primary,
            t.accent_secondary,
            t.success,
            t.warning,
            t.danger,
            t.info,
            t.selection_bg,
            t.selection_fg,
            t.border,
            t.border_focus,
            t.surface,
            t.header_bg,
            t.overlay_bg,
            t.bar_empty,
            t.scrollbar_thumb,
            t.scrollbar_track,
            t.sleeping,
            t.selection_edge,
            t.fg_inverse,
        ] {
            assert_eq!(c, Color::Reset, "NoColor theme leaked a colour: {c:?}");
        }
        assert!(t.is_mono());
    }

    #[test]
    fn mono_theme_is_colorless_even_on_truecolor() {
        let t = Theme::with_kind(ThemeKind::Mono, ColorSupport::TrueColor);
        assert_eq!(t.accent_primary, Color::Reset);
        assert_eq!(t.danger, Color::Reset);
        assert!(t.is_mono());
    }

    #[test]
    fn mono_carries_hierarchy_with_modifiers() {
        // Without colour, every emphasis level must still differ from the next.
        let t = Theme::new(ColorSupport::NoColor);
        assert!(t.selected_row().add_modifier.contains(Modifier::REVERSED));
        assert!(t.dim().add_modifier.contains(Modifier::DIM));
        assert!(t.strong().add_modifier.contains(Modifier::BOLD));
        assert!(t.table_header().add_modifier.contains(Modifier::BOLD));
        assert!(t.key().add_modifier.contains(Modifier::BOLD));
        assert!(
            t.level_style(Level::Error)
                .add_modifier
                .contains(Modifier::BOLD)
        );
        // Zebra striping is meaningless without colour and must not become
        // REVERSED on every other row.
        assert_eq!(t.row(true), t.row(false));
    }

    #[test]
    fn colored_themes_stripe_odd_rows() {
        let t = Theme::new(ColorSupport::TrueColor);
        assert_ne!(t.row(true), t.row(false));
    }

    #[test]
    fn light_theme_is_marked_light() {
        assert!(Theme::with_kind(ThemeKind::TokyoNightLight, ColorSupport::TrueColor).is_light());
        assert!(!Theme::with_kind(ThemeKind::TokyoNight, ColorSupport::TrueColor).is_light());
    }

    #[test]
    fn light_theme_text_is_darker_than_its_background() {
        // A light theme that ships an unreadable foreground is worse than none.
        let t = Theme::with_kind(ThemeKind::TokyoNightLight, ColorSupport::TrueColor);
        let luma = |c: Color| match c {
            Color::Rgb(r, g, b) => 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64,
            _ => panic!("expected RGB, got {c:?}"),
        };
        assert!(
            luma(t.fg) < luma(t.bg) - 60.0,
            "light theme foreground is not dark enough against its background"
        );
    }

    #[test]
    fn gauge_color_thresholds() {
        let t = Theme::new(ColorSupport::TrueColor);
        assert_eq!(t.gauge_color(0.0), t.success);
        assert_eq!(t.gauge_color(49.9), t.success);
        assert_eq!(t.gauge_color(50.0), t.warning);
        assert_eq!(t.gauge_color(79.9), t.warning);
        assert_eq!(t.gauge_color(80.0), t.danger);
        assert_eq!(t.gauge_color(100.0), t.danger);
    }

    #[test]
    fn level_colors_are_distinct() {
        let t = Theme::new(ColorSupport::TrueColor);
        let colors = [
            t.level_color(Level::Info),
            t.level_color(Level::Success),
            t.level_color(Level::Warning),
            t.level_color(Level::Error),
        ];
        for (i, a) in colors.iter().enumerate() {
            for b in &colors[i + 1..] {
                assert_ne!(a, b, "two severities share a colour");
            }
        }
    }

    #[test]
    fn theme_kind_parses_its_own_names() {
        for &kind in ThemeKind::ALL {
            assert_eq!(ThemeKind::from_str(kind.name()).unwrap(), kind);
        }
    }

    #[test]
    fn theme_kind_parses_aliases_and_rejects_junk() {
        assert_eq!(ThemeKind::from_str("dark").unwrap(), ThemeKind::TokyoNight);
        assert_eq!(
            ThemeKind::from_str("  LIGHT ").unwrap(),
            ThemeKind::TokyoNightLight
        );
        assert_eq!(
            ThemeKind::from_str("tokyo_night").unwrap(),
            ThemeKind::TokyoNight
        );
        let err = ThemeKind::from_str("dracula").unwrap_err();
        assert!(err.contains("dracula"), "error should quote the bad value");
        assert!(err.contains("tokyo-night"), "error should list what works");
    }
}
