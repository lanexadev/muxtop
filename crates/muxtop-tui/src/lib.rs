pub mod app;
pub mod error;
pub mod event;
pub mod terminal;
pub mod ui;

pub use app::{AppState, Command, ConfirmAction, PaletteState, Tab};
pub use error::TuiError;
pub use event::{Event, EventHandler, TICK_RATE};
pub use terminal::{
    ColorSupport, TermCaps, TerminalGuard, Tui, detect_terminal_caps, init_terminal,
    install_panic_hook, restore_terminal,
};

use muxtop_core::process::SortField;
use muxtop_core::system::SystemSnapshot;
use tokio::sync::mpsc;

/// Configuration passed from CLI arguments to the TUI.
#[derive(Debug, Clone)]
pub struct CliConfig {
    /// Initial process filter pattern (from `--filter`).
    pub filter: Option<String>,
    /// Initial sort field (from `--sort`).
    pub sort_field: SortField,
    /// Start in tree view mode (from `--tree`).
    pub tree_mode: bool,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            filter: None,
            sort_field: SortField::Cpu,
            tree_mode: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cli_config_default() {
        let config = CliConfig::default();
        assert!(config.filter.is_none());
        assert!(matches!(config.sort_field, SortField::Cpu));
        assert!(!config.tree_mode);
    }

    #[test]
    fn test_cli_config_with_filter() {
        let config = CliConfig {
            filter: Some("firefox".to_string()),
            ..Default::default()
        };
        assert_eq!(config.filter.as_deref(), Some("firefox"));
    }

    #[test]
    fn test_cli_config_with_sort() {
        let config = CliConfig {
            sort_field: SortField::Mem,
            ..Default::default()
        };
        assert!(matches!(config.sort_field, SortField::Mem));
    }

    #[test]
    fn test_cli_config_with_tree() {
        let config = CliConfig {
            tree_mode: true,
            ..Default::default()
        };
        assert!(config.tree_mode);
    }

    #[test]
    fn test_cli_config_clone() {
        let config = CliConfig {
            filter: Some("test".to_string()),
            sort_field: SortField::Pid,
            tree_mode: true,
        };
        let cloned = config.clone();
        assert_eq!(cloned.filter, config.filter);
        assert!(matches!(cloned.sort_field, SortField::Pid));
        assert!(cloned.tree_mode);
    }

    #[test]
    fn test_cli_config_debug() {
        let config = CliConfig::default();
        assert!(!format!("{config:?}").is_empty());
    }
}

/// Run the TUI event loop. Blocks until the user quits.
/// The TerminalGuard ensures the terminal is restored on any exit path
/// (normal return, error propagation via ?, or panic unwind).
pub fn run(rx: mpsc::Receiver<SystemSnapshot>, config: CliConfig) -> Result<(), TuiError> {
    install_panic_hook();
    let mut guard = init_terminal()?;
    let term_caps = detect_terminal_caps();
    let mut app = app::AppState::with_config(config, term_caps);
    let mut handler = EventHandler::new(rx);

    while app.running() {
        guard.0.draw(|frame| ui::draw_root(frame, &app))?;

        match handler.poll_event()? {
            Event::Key(key) => app.handle_key_event(key),
            Event::Mouse(mouse) => app.handle_mouse_event(mouse),
            Event::Snapshot(snap) => app.apply_snapshot(snap),
            Event::Resize(_, _) | Event::Tick => {}
        }
    }

    // Explicit restore for clean exit (TerminalGuard Drop is the safety net).
    restore_terminal(&mut guard.0)?;
    Ok(())
}
