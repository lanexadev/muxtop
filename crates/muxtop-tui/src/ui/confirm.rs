// Confirmation dialog overlay rendering.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::theme::Theme;
use crate::app::AppState;

/// Render the confirmation dialog as a centered overlay.
pub fn draw_confirm(frame: &mut Frame, app: &AppState, theme: &Theme) {
    let Some(ref action) = app.confirm else {
        return;
    };

    let area = frame.area();
    let prompt = action.prompt();

    // Dialog dimensions: fit the prompt + some padding.
    let width = (prompt.len() as u16 + 6).min(area.width.saturating_sub(4));
    let height = 3u16.min(area.height.saturating_sub(2)); // border + text + border

    if width < 10 || height < 3 {
        return;
    }

    let popup = centered_rect(width, height, area);

    // Clear the area behind the popup.
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .title(" Confirm ")
        .title_style(
            Style::default()
                .fg(theme.bg)
                .bg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme.warning).bg(theme.bg));

    let inner = block.inner(popup);
    frame.render_widget(Clear, popup); // Clear must be matched precisely to widget bounds.
    frame.render_widget(block, popup);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let line = Line::from(vec![Span::styled(
        format!(" {prompt}"),
        Style::default().fg(theme.fg),
    )]);

    frame.render_widget(Paragraph::new(line), inner);
}

/// Create a centered `Rect` of given size within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use crate::app::{AppState, ConfirmAction};
    use muxtop_core::actions::Signal;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_with(app: &AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                crate::ui::draw_root(frame, app);
            })
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_contains(buf: &ratatui::buffer::Buffer, needle: &str) -> bool {
        let height = buf.area.height;
        (0..height).any(|row| {
            let width = buf.area.width;
            let line: String = (0..width)
                .map(|col| buf.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            line.contains(needle)
        })
    }

    #[test]
    fn test_confirm_dialog_not_shown_when_none() {
        let app = AppState::new();
        let buf = render_with(&app, 80, 24);
        assert!(!buffer_contains(&buf, "Confirm"));
    }

    #[test]
    fn test_confirm_dialog_shows_kill_prompt() {
        let mut app = AppState::new();
        app.confirm = Some(ConfirmAction::Kill {
            pid: 1234,
            name: "firefox".to_string(),
            signal: Signal::Term,
        });
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Confirm"));
        assert!(buffer_contains(&buf, "SIGTERM"));
        assert!(buffer_contains(&buf, "firefox"));
        assert!(buffer_contains(&buf, "1234"));
    }

    #[test]
    fn test_confirm_dialog_shows_sigkill_prompt() {
        let mut app = AppState::new();
        app.confirm = Some(ConfirmAction::Kill {
            pid: 42,
            name: "chrome".to_string(),
            signal: Signal::Kill,
        });
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "SIGKILL"));
        assert!(buffer_contains(&buf, "chrome"));
    }

    #[test]
    fn test_confirm_dialog_shows_renice_prompt() {
        let mut app = AppState::new();
        app.confirm = Some(ConfirmAction::Renice {
            pid: 99,
            name: "node".to_string(),
            delta: 1,
        });
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Renice"));
        assert!(buffer_contains(&buf, "node"));
    }

    #[test]
    fn test_confirm_dialog_minimal_terminal_no_panic() {
        let mut app = AppState::new();
        app.confirm = Some(ConfirmAction::Kill {
            pid: 1,
            name: "init".to_string(),
            signal: Signal::Term,
        });
        let _buf = render_with(&app, 10, 5);
        let _buf = render_with(&app, 1, 1);
    }

    #[test]
    fn test_confirm_action_prompt_format() {
        let kill = ConfirmAction::Kill {
            pid: 100,
            name: "test".to_string(),
            signal: Signal::Term,
        };
        assert!(kill.prompt().contains("SIGTERM"));
        assert!(kill.prompt().contains("100"));

        let renice = ConfirmAction::Renice {
            pid: 200,
            name: "proc".to_string(),
            delta: -1,
        };
        assert!(renice.prompt().contains("higher priority"));
    }
}
