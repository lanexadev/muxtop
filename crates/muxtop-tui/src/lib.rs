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

use std::net::SocketAddr;

use muxtop_core::process::SortField;
use muxtop_core::system::SystemSnapshot;
use tokio::sync::mpsc;

/// Whether the TUI is monitoring the local machine or a remote server.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum ConnectionMode {
    /// Collecting system data locally via sysinfo.
    #[default]
    Local,
    /// Receiving snapshots from a remote muxtop-server.
    Remote { hostname: String, addr: SocketAddr },
}

/// Configuration passed from CLI arguments to the TUI.
#[derive(Debug, Clone)]
pub struct CliConfig {
    /// Initial process filter pattern (from `--filter`).
    pub filter: Option<String>,
    /// Initial sort field (from `--sort`).
    pub sort_field: SortField,
    /// Start in tree view mode (from `--tree`).
    pub tree_mode: bool,
    /// Local or remote connection mode.
    pub connection_mode: ConnectionMode,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            filter: None,
            sort_field: SortField::Cpu,
            tree_mode: false,
            connection_mode: ConnectionMode::default(),
        }
    }
}

/// Run the TUI event loop. Blocks until the user quits.
/// The TerminalGuard ensures the terminal is restored on any exit path
/// (normal return, error propagation via ?, or panic unwind).
///
/// `container_engine` is shared with the Collector so that Stop/Kill/Restart
/// actions dispatched from the TUI hit the same daemon. Pass `None` to
/// disable container actions (the UI surfaces a "not configured" status
/// message instead of spawning work).
pub fn run(
    rx: mpsc::Receiver<SystemSnapshot>,
    config: CliConfig,
    container_engine: Option<
        std::sync::Arc<dyn muxtop_core::container_engine::ContainerEngine + Send + Sync>,
    >,
) -> Result<(), TuiError> {
    install_panic_hook();
    let mut guard = init_terminal()?;
    let term_caps = detect_terminal_caps();
    let mut app = app::AppState::with_config(config, term_caps);
    if let Some(engine) = container_engine {
        app.set_container_engine(engine);
    }
    let mut handler = EventHandler::new(rx);

    while app.running() {
        // Drain any container-action outcomes produced by spawned tokio
        // tasks (Stop/Kill/Restart) so they surface as status messages.
        // `pump_action_results` flips `needs_redraw` if anything arrived.
        app.pump_action_results();

        // PERF-H1: event-driven render. We only repaint when something the
        // UI actually shows has changed: a new event from the user / OS, a
        // new snapshot, an async action outcome, or an expired status
        // message. Idle ticks at 60 Hz no longer pay for a full draw.
        let mut should_draw = app.take_needs_redraw();

        match handler.poll_event()? {
            Event::Key(key) => {
                app.handle_key_event(key);
                should_draw = true;
            }
            Event::Mouse(mouse) => {
                app.handle_mouse_event(mouse);
                should_draw = true;
            }
            Event::Snapshot(snap) => {
                app.apply_snapshot(snap);
                should_draw = true;
            }
            Event::Resize(_, _) => {
                should_draw = true;
            }
            Event::Tick => {
                // PERF-H1: ticks no longer force a redraw. The only thing
                // that "ages" without a key/snapshot/action is the status
                // message — schedule one final repaint when it expires so
                // the bottom bar visually clears.
                if app.status_message_just_expired() {
                    should_draw = true;
                }
            }
        }

        if should_draw {
            guard.0.draw(|frame| ui::draw_root(frame, &app))?;
        }
    }

    // Explicit restore for clean exit (TerminalGuard Drop is the safety net).
    restore_terminal(&mut guard.0)?;
    Ok(())
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
        assert_eq!(config.connection_mode, ConnectionMode::Local);
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
            ..Default::default()
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

    #[test]
    fn test_connection_mode_default_is_local() {
        assert_eq!(ConnectionMode::default(), ConnectionMode::Local);
    }

    #[test]
    fn test_connection_mode_remote_variant() {
        let addr: std::net::SocketAddr = "127.0.0.1:4242".parse().unwrap();
        let mode = ConnectionMode::Remote {
            hostname: "prod-01".to_string(),
            addr,
        };
        assert!(matches!(mode, ConnectionMode::Remote { .. }));
        if let ConnectionMode::Remote { hostname, addr } = &mode {
            assert_eq!(hostname, "prod-01");
            assert_eq!(addr.port(), 4242);
        }
    }

    #[test]
    fn test_connection_mode_equality() {
        let addr: std::net::SocketAddr = "10.0.0.1:4242".parse().unwrap();
        let a = ConnectionMode::Remote {
            hostname: "host".to_string(),
            addr,
        };
        let b = ConnectionMode::Remote {
            hostname: "host".to_string(),
            addr,
        };
        assert_eq!(a, b);
        assert_ne!(ConnectionMode::Local, a);
    }

    #[test]
    fn test_cli_config_with_remote_mode() {
        let addr: std::net::SocketAddr = "192.168.1.1:4242".parse().unwrap();
        let config = CliConfig {
            connection_mode: ConnectionMode::Remote {
                hostname: "server".to_string(),
                addr,
            },
            ..Default::default()
        };
        assert!(matches!(
            config.connection_mode,
            ConnectionMode::Remote { .. }
        ));
    }
}
