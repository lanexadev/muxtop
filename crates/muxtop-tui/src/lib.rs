pub mod app;
pub mod error;
pub mod event;
pub mod terminal;
pub mod ui;

pub use app::{AppState, Tab};
pub use error::TuiError;
pub use event::{Event, EventHandler, TICK_RATE};
pub use terminal::{init_terminal, install_panic_hook, restore_terminal, TerminalGuard, Tui};

use muxtop_core::system::SystemSnapshot;
use tokio::sync::mpsc;

/// Run the TUI event loop. Blocks until the user quits.
/// The TerminalGuard ensures the terminal is restored on any exit path
/// (normal return, error propagation via ?, or panic unwind).
pub fn run(rx: mpsc::Receiver<SystemSnapshot>) -> Result<(), TuiError> {
    install_panic_hook();
    let mut guard = init_terminal()?;
    let mut app = app::AppState::new();
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
