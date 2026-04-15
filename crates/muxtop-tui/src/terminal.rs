use std::io::{self, Stdout};

use crossterm::{
    cursor,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::error::TuiError;

// ---------------------------------------------------------------------------
// Terminal capability detection
// ---------------------------------------------------------------------------

/// Detected color support level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorSupport {
    /// No color support (e.g., `TERM=dumb`).
    NoColor,
    /// Basic 16-color ANSI.
    Basic,
    /// 256-color mode (`TERM` ends in `-256color`).
    Colors256,
    /// True color (24-bit) — `$COLORTERM` is `truecolor` or `24bit`.
    TrueColor,
}

/// Terminal capabilities detected at startup.
#[derive(Debug, Clone)]
pub struct TermCaps {
    /// Color support level.
    pub color_support: ColorSupport,
    /// Whether the terminal supports Unicode rendering.
    pub unicode: bool,
    /// Terminal width at detection time.
    pub width: u16,
    /// Terminal height at detection time.
    pub height: u16,
}

impl Default for TermCaps {
    fn default() -> Self {
        Self {
            color_support: ColorSupport::TrueColor,
            unicode: true,
            width: 80,
            height: 24,
        }
    }
}

impl TermCaps {
    /// Whether the terminal is considered "small" (< 80 columns or < 24 rows).
    pub fn is_small(&self) -> bool {
        self.width < 80 || self.height < 24
    }
}

/// Detect terminal capabilities from environment variables and terminal size.
pub fn detect_terminal_caps() -> TermCaps {
    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let lang = std::env::var("LANG").unwrap_or_default();

    let color_support = detect_color_support(&term, &colorterm);
    let unicode = detect_unicode(&term, &lang);
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));

    TermCaps {
        color_support,
        unicode,
        width,
        height,
    }
}

fn detect_color_support(term: &str, colorterm: &str) -> ColorSupport {
    let ct_lower = colorterm.to_lowercase();
    if ct_lower == "truecolor" || ct_lower == "24bit" {
        return ColorSupport::TrueColor;
    }

    let term_lower = term.to_lowercase();
    if term_lower == "dumb" || term_lower.is_empty() {
        return ColorSupport::NoColor;
    }

    if term_lower.ends_with("-256color") || term_lower.contains("256color") {
        return ColorSupport::Colors256;
    }

    // Most modern terminals support at least basic colors.
    ColorSupport::Basic
}

fn detect_unicode(term: &str, lang: &str) -> bool {
    let term_lower = term.to_lowercase();
    if term_lower == "dumb" {
        return false;
    }

    // Check LANG for UTF-8 indicators.
    let lang_lower = lang.to_lowercase();
    if lang_lower.contains("utf-8") || lang_lower.contains("utf8") {
        return true;
    }

    // Most modern terminals support Unicode by default.
    true
}

/// Type alias for our terminal backend.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// RAII guard that restores the terminal on drop (normal exit, error, or unwind).
pub struct TerminalGuard(pub Tui);

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = restore_terminal(&mut self.0);
    }
}

/// Initialize the terminal: raw mode, alternate screen, mouse capture.
/// On partial failure, cleans up any state already set.
pub fn init_terminal() -> Result<TerminalGuard, TuiError> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(TuiError::Terminal(e));
    }

    let backend = CrosstermBackend::new(stdout);
    match Terminal::new(backend) {
        Ok(terminal) => Ok(TerminalGuard(terminal)),
        Err(e) => {
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
            let _ = disable_raw_mode();
            Err(TuiError::Terminal(e))
        }
    }
}

/// Restore the terminal to its original state.
pub fn restore_terminal(terminal: &mut Tui) -> Result<(), TuiError> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

/// Install a panic hook that restores the terminal before printing the panic message.
/// Must be called BEFORE init_terminal().
pub fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            cursor::Show
        );
        original_hook(panic_info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_panic_hook_install() {
        install_panic_hook();
    }

    #[test]
    fn test_restore_is_idempotent() {
        let _ = disable_raw_mode();
    }

    #[test]
    fn test_terminal_guard_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<TerminalGuard>();
    }

    // ---- Terminal capability detection tests ----

    #[test]
    fn test_detect_truecolor() {
        let result = detect_color_support("xterm-256color", "truecolor");
        assert_eq!(result, ColorSupport::TrueColor);
    }

    #[test]
    fn test_detect_truecolor_24bit() {
        let result = detect_color_support("xterm", "24bit");
        assert_eq!(result, ColorSupport::TrueColor);
    }

    #[test]
    fn test_detect_256color() {
        let result = detect_color_support("xterm-256color", "");
        assert_eq!(result, ColorSupport::Colors256);
    }

    #[test]
    fn test_detect_basic_color() {
        let result = detect_color_support("xterm", "");
        assert_eq!(result, ColorSupport::Basic);
    }

    #[test]
    fn test_detect_no_color_dumb() {
        let result = detect_color_support("dumb", "");
        assert_eq!(result, ColorSupport::NoColor);
    }

    #[test]
    fn test_detect_no_color_empty() {
        let result = detect_color_support("", "");
        assert_eq!(result, ColorSupport::NoColor);
    }

    #[test]
    fn test_detect_unicode_utf8_lang() {
        assert!(detect_unicode("xterm", "en_US.UTF-8"));
    }

    #[test]
    fn test_detect_unicode_dumb_term() {
        assert!(!detect_unicode("dumb", "en_US.UTF-8"));
    }

    #[test]
    fn test_ascii_fallback_no_unicode() {
        // When TERM=dumb, unicode is false
        assert!(!detect_unicode("dumb", ""));
    }

    #[test]
    fn test_term_caps_default() {
        let caps = TermCaps::default();
        assert_eq!(caps.color_support, ColorSupport::TrueColor);
        assert!(caps.unicode);
        assert_eq!(caps.width, 80);
        assert_eq!(caps.height, 24);
    }

    #[test]
    fn test_term_caps_is_small() {
        let mut caps = TermCaps::default();
        assert!(!caps.is_small()); // 80x24 is not small
        caps.width = 79;
        assert!(caps.is_small());
        caps.width = 80;
        caps.height = 23;
        assert!(caps.is_small());
    }
}
