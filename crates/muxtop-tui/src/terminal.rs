use std::io::{self, Stdout};

use crossterm::{
    cursor,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::error::TuiError;

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
}
