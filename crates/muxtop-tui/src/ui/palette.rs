// Command palette overlay rendering.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::app::AppState;

/// Teal accent color (matches tab bar).
const TEAL: Color = Color::Rgb(78, 201, 176);

/// Render the command palette as a centered overlay.
pub fn draw_palette(frame: &mut Frame, app: &AppState) {
    let area = frame.area();

    // Palette dimensions: 60 wide (or area width - 4), up to 16 tall.
    let width = 60.min(area.width.saturating_sub(4));
    let max_results = 10;
    // 3 = border top + input line + border bottom, +1 for each result row
    let result_rows = app.palette.filtered.len().min(max_results);
    let height = (3
        + result_rows as u16
        + if app.palette.filtered.is_empty() {
            1
        } else {
            0
        })
    .min(area.height.saturating_sub(2));

    if width < 10 || height < 3 {
        return; // Terminal too small for palette
    }

    let popup = centered_rect(width, height, area);

    // Clear the area behind the popup.
    frame.render_widget(Clear, popup);

    // Draw the border.
    let block = Block::default()
        .title(" Command Palette ")
        .title_style(Style::default().fg(TEAL).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(TEAL));

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Split inner into input line + results area.
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).split(inner);
    let input_area = chunks[0];
    let results_area = chunks[1];

    // Render input line with cursor.
    let input_text = format!("> {}\u{2588}", app.palette.input); // block cursor
    let input_line = Paragraph::new(Line::from(Span::styled(
        input_text,
        Style::default().fg(Color::White),
    )));
    frame.render_widget(input_line, input_area);

    // Render results.
    if app.palette.filtered.is_empty() {
        if results_area.height > 0 {
            let no_match = Paragraph::new(Line::from(Span::styled(
                "  No matches",
                Style::default().fg(Color::DarkGray),
            )));
            frame.render_widget(no_match, results_area);
        }
        return;
    }

    let visible_count = (results_area.height as usize).min(app.palette.filtered.len());
    // Scroll so selected item is always visible.
    let scroll_offset = if app.palette.selected >= visible_count {
        app.palette.selected - visible_count + 1
    } else {
        0
    };

    for (i, &(cmd, _score)) in app
        .palette
        .filtered
        .iter()
        .skip(scroll_offset)
        .enumerate()
        .take(visible_count)
    {
        let list_idx = scroll_offset + i;
        let is_selected = list_idx == app.palette.selected;

        let row_area = Rect {
            x: results_area.x,
            y: results_area.y + i as u16,
            width: results_area.width,
            height: 1,
        };

        let label = cmd.label();
        let shortcut = cmd.shortcut();

        // Right-align the shortcut.
        let (label_style, shortcut_style) = if is_selected {
            (
                Style::default()
                    .fg(TEAL)
                    .add_modifier(Modifier::BOLD)
                    .bg(Color::DarkGray),
                Style::default().fg(Color::White).bg(Color::DarkGray),
            )
        } else {
            (
                Style::default().fg(Color::White),
                Style::default().fg(Color::DarkGray),
            )
        };

        // Build the line: "  label" + padding + "shortcut  "
        let label_str = format!("  {}", label);
        let shortcut_str = format!("  {}", shortcut);
        let padding_len = row_area
            .width
            .saturating_sub(label_str.len() as u16 + shortcut_str.len() as u16);
        let padding = " ".repeat(padding_len as usize);

        let line = Line::from(vec![
            Span::styled(label_str, label_style),
            Span::styled(
                padding,
                if is_selected {
                    Style::default().bg(Color::DarkGray)
                } else {
                    Style::default()
                },
            ),
            Span::styled(shortcut_str, shortcut_style),
        ]);

        // For selected row, fill the background.
        if is_selected {
            let bg = Paragraph::new(Line::from(" ".repeat(row_area.width as usize)))
                .style(Style::default().bg(Color::DarkGray));
            frame.render_widget(bg, row_area);
        }

        frame.render_widget(Paragraph::new(line), row_area);
    }
}

/// Create a centered `Rect` of given size within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use ratatui::{Terminal, backend::TestBackend};

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

    fn buffer_line_text(buf: &ratatui::buffer::Buffer, row: u16) -> String {
        let width = buf.area.width;
        (0..width)
            .map(|col| buf.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    // AC-10: Palette renders centered overlay
    #[test]
    fn test_palette_renders_overlay() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        let buf = render_with(&app, 80, 24);
        assert!(
            buffer_contains(&buf, "Command Palette"),
            "Palette title should be visible"
        );
    }

    // AC-12: Empty input shows all commands
    #[test]
    fn test_palette_shows_all_commands_when_empty() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Quit"), "Should show Quit command");
        assert!(
            buffer_contains(&buf, "Toggle tree view"),
            "Should show Toggle tree view"
        );
    }

    // AC-11: Shortcut hints shown
    #[test]
    fn test_palette_shows_shortcuts() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        let buf = render_with(&app, 80, 24);
        // "q" is the shortcut for Quit — but it could also be part of text
        // Check for a more distinct shortcut
        assert!(
            buffer_contains(&buf, "F3"),
            "Should show F3 shortcut for Sort by CPU"
        );
    }

    // AC-13: No matches message
    #[test]
    fn test_palette_no_matches() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.input = "zzzzz".to_string();
        app.palette.refilter();
        let buf = render_with(&app, 80, 24);
        assert!(
            buffer_contains(&buf, "No matches"),
            "Should show 'No matches' message"
        );
    }

    // AC-10: Centered rendering check
    #[test]
    fn test_palette_centered() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        let buf = render_with(&app, 80, 24);
        // The palette should NOT start at column 0 (it's centered)
        // Find the "Command Palette" title row
        for row in 0..24 {
            let line = buffer_line_text(&buf, row);
            if line.contains("Command Palette") {
                // Should have leading whitespace (centered)
                assert!(
                    line.starts_with(' '),
                    "Palette should be centered, not left-aligned"
                );
                break;
            }
        }
    }

    // Palette does not render when show_palette is false
    #[test]
    fn test_palette_not_shown_when_closed() {
        let app = AppState::new();
        let buf = render_with(&app, 80, 24);
        assert!(
            !buffer_contains(&buf, "Command Palette"),
            "Palette should not be visible when closed"
        );
    }

    // Minimal terminal: no panic
    #[test]
    fn test_palette_minimal_terminal_no_panic() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        let _buf = render_with(&app, 10, 5);
        let _buf = render_with(&app, 1, 1);
    }

    // centered_rect unit test
    #[test]
    fn test_centered_rect() {
        let area = Rect::new(0, 0, 80, 24);
        let r = centered_rect(60, 16, area);
        assert_eq!(r.x, 10);
        assert_eq!(r.y, 4);
        assert_eq!(r.width, 60);
        assert_eq!(r.height, 16);
    }

    #[test]
    fn test_centered_rect_small_area() {
        let area = Rect::new(0, 0, 20, 10);
        let r = centered_rect(60, 16, area);
        // Width clamped to area width
        assert_eq!(r.width, 20);
        assert_eq!(r.height, 10);
    }
}
