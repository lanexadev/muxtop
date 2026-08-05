// Contextual actions menu (`x`).
//
// muxtop 0.4 required memorising which function key did what on which tab, and
// hid the answer in the README. `x` now lists exactly the actions available
// here, right now, with their shortcuts — so the menu teaches the keymap
// instead of replacing it.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::Render;
use crate::ui::widgets::overlay;

pub fn draw_actions(frame: &mut Frame, r: &Render<'_>) {
    let actions = r.app.tab_actions();
    let area = frame.area();

    let width = 48.min(area.width.saturating_sub(2)).max(20);
    let height = (actions.len() as u16 + 4).min(area.height);
    let popup = overlay::centered(width, height, area);

    let title = r.titled("Actions", r.app.tab.label());
    let inner = overlay::popup(frame, popup, &title, r.theme, r.glyphs);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let [body, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);

    if actions.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "  No actions on this tab",
                r.theme.dim(),
            ))),
            body,
        );
    } else {
        for (i, (key, label, _)) in actions.iter().take(body.height as usize).enumerate() {
            let selected = i == r.app.actions_selected;
            let row = Rect {
                x: body.x,
                y: body.y + i as u16,
                width: body.width,
                height: 1,
            };
            let (edge, style) = if selected {
                (r.glyphs.sel_edge, r.theme.selected_row())
            } else {
                (" ", r.theme.body())
            };
            // Pad the label so the selection highlight covers the full row.
            let text = format!("{label:<w$}", w = row.width.saturating_sub(12) as usize);
            let line = Line::from(vec![
                Span::styled(
                    edge.to_string(),
                    r.theme.level_style(crate::ui::theme::Level::Info),
                ),
                Span::styled(format!(" {text}"), style),
                Span::styled(
                    format!("{key:>9} "),
                    if selected { style } else { r.theme.dim() },
                ),
            ]);
            frame.render_widget(Paragraph::new(line), row);
        }
    }

    let hint = Line::from(vec![
        Span::styled(" Enter ", r.theme.key()),
        Span::styled(" run   ", r.theme.key_desc()),
        Span::styled(" Esc ", r.theme.key()),
        Span::styled(" cancel ", r.theme.key_desc()),
    ]);
    frame.render_widget(Paragraph::new(hint), footer);
}

#[cfg(test)]
mod tests {
    use crate::app::{Overlay, Tab};
    use crate::terminal::ColorSupport;
    use crate::ui::test_support::*;

    fn menu(tab: Tab) -> String {
        let mut app = app_with_data();
        app.tab = tab;
        app.overlay = Overlay::Actions;
        all_text(&render_with(&app, 100, 30))
    }

    #[test]
    fn menu_lists_the_tabs_own_actions_with_their_keys() {
        let text = menu(Tab::Processes);
        assert!(text.contains("Kill process"));
        assert!(text.contains("F9"), "the menu must teach the shortcut");
    }

    #[test]
    fn menu_is_contextual() {
        let containers = menu(Tab::Containers);
        assert!(containers.contains("Restart container"));
        assert!(
            !containers.contains("SIGTERM)"),
            "process actions do not belong on the Containers tab"
        );
    }

    #[test]
    fn menu_says_so_when_a_tab_has_no_actions() {
        let text = menu(Tab::Network);
        assert!(text.contains("No actions"));
    }

    #[test]
    fn menu_hides_actions_that_cannot_work_remotely() {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app.overlay = Overlay::Actions;
        app.connection_mode = crate::ConnectionMode::Remote {
            hostname: "prod".into(),
            addr: "10.0.0.1:4242".parse().unwrap(),
        };
        let text = all_text(&render_with(&app, 100, 30));
        assert!(
            !text.contains("Kill process"),
            "must not offer a kill we cannot perform:\n{text}"
        );
    }

    #[test]
    fn menu_marks_the_selection() {
        let mut app = app_with_data();
        app.tab = Tab::Containers;
        app.overlay = Overlay::Actions;
        app.actions_selected = 1;
        let text = all_text(&render_with(&app, 100, 30));
        assert!(text.contains("Kill container"));
    }

    #[test]
    fn menu_survives_tiny_terminals_and_ascii() {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app.overlay = Overlay::Actions;
        for (w, h) in [(1u16, 1u16), (10, 4), (24, 8)] {
            let _ = render_with(&app, w, h);
        }
        let buf = render_caps(&mut app, 80, 20, ColorSupport::Basic, false);
        assert!(all_text(&buf).is_ascii());
    }
}
