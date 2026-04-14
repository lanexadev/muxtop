use crate::terminal::ColorSupport;
use ratatui::style::Color;

/// Theme definition for muxtop.
/// Applies the Tokyo Night theme if TrueColor is supported,
/// otherwise falls back to standard 256 or Basic colors.
pub struct Theme {
    pub bg: Color,
    pub fg: Color,
    pub text_dim: Color,
    pub accent_primary: Color,
    pub accent_secondary: Color,
    pub header_bg: Color,
    pub selection_bg: Color,
    pub selection_fg: Color,
    pub surface: Color,
    pub bar_empty: Color,

    // Status colors
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub sleeping: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self::new(ColorSupport::TrueColor)
    }
}

impl Theme {
    pub fn new(support: ColorSupport) -> Self {
        if support == ColorSupport::TrueColor {
            // Tokyo Night TrueColor Palette
            Self {
                bg: Color::Rgb(26, 27, 38),                  // #1a1b26
                fg: Color::Rgb(192, 202, 245),               // #c0caf5
                text_dim: Color::Rgb(86, 95, 137),           // #565f89
                accent_primary: Color::Rgb(125, 207, 255),   // #7dcfff (Cyan)
                accent_secondary: Color::Rgb(187, 154, 247), // #bb9af7 (Purple)
                header_bg: Color::Rgb(36, 40, 59),           // #24283b
                selection_bg: Color::Rgb(55, 65, 115),       // #374173
                selection_fg: Color::Rgb(255, 255, 255),     // White
                surface: Color::Rgb(30, 32, 48),             // #1e2030
                bar_empty: Color::Rgb(40, 44, 62),           // #282c3e

                success: Color::Rgb(158, 206, 106), // #9ece6a
                warning: Color::Rgb(224, 175, 104), // #e0af68
                danger: Color::Rgb(247, 118, 142),  // #f7768e
                sleeping: Color::Rgb(86, 95, 137),  // #565f89
            }
        } else {
            // ANSI / 16-color Fallbacks
            Self {
                bg: Color::Reset,
                fg: Color::White,
                text_dim: Color::DarkGray,
                accent_primary: Color::Cyan,
                accent_secondary: Color::Magenta,
                header_bg: Color::DarkGray,
                selection_bg: Color::DarkGray,
                selection_fg: Color::White,
                surface: Color::Reset,
                bar_empty: Color::DarkGray,

                success: Color::Green,
                warning: Color::Yellow,
                danger: Color::Red,
                sleeping: Color::DarkGray,
            }
        }
    }

    /// Helper for the gradient gauge on CPU/Mem.
    /// Returns green if <50%, yellow if 50%-80%, red if >80%.
    pub fn gauge_color(&self, percent: f64) -> Color {
        if percent >= 80.0 {
            self.danger
        } else if percent >= 50.0 {
            self.warning
        } else {
            self.success
        }
    }
}
