// Confirmation dialog.
//
// The last gate before something irreversible. Restyled for 0.5.1: a danger
// border, the target spelled out, and cancel offered first — but the behaviour
// that mattered is unchanged, because it was already right.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use ratatui::layout::Alignment;

use crate::ui::Render;
use crate::ui::theme::Level;
use crate::ui::widgets::{overlay, panel};

pub fn draw_confirm(frame: &mut Frame, r: &Render<'_>) {
    let Some(action) = r.app.confirm.as_ref() else {
        return;
    };
    let prompt = action.prompt();
    let area = frame.area();

    // Wide enough for the prompt, within reason.
    let width = (prompt.chars().count() as u16 + 6)
        .clamp(24, 72)
        .min(area.width);
    let height = 7.min(area.height);
    let popup = overlay::centered(width, height, area);

    if popup.width < 6 || popup.height < 3 {
        return;
    }

    frame.render_widget(Clear, popup);
    let block = panel::block(Some("Confirm"), true, r.theme, r.glyphs)
        .border_style(r.theme.level_style(Level::Error))
        .title_style(r.theme.level_style(Level::Error));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    if inner.height == 0 {
        return;
    }

    let [text_area, _, keys_area] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(prompt, r.theme.strong())))
            .alignment(Alignment::Center)
            .wrap(ratatui::widgets::Wrap { trim: true }),
        text_area,
    );

    // Cancel is listed first: the safe answer should be the one the eye lands
    // on, even though both keys are one press away.
    let keys = Line::from(vec![
        Span::styled(" Esc ", r.theme.key()),
        Span::styled(" cancel   ", r.theme.key_desc()),
        Span::styled(" y ", r.theme.level_fill(Level::Error)),
        Span::styled(" confirm ", r.theme.key_desc()),
    ]);
    frame.render_widget(Paragraph::new(keys).alignment(Alignment::Center), keys_area);
}

#[cfg(test)]
mod tests {
    use crate::app::{AppState, ConfirmAction, Tab};
    use crate::terminal::ColorSupport;
    use crate::ui::test_support::*;
    use muxtop_core::actions::Signal;

    fn app_with_confirm() -> AppState {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app.confirm = Some(ConfirmAction::Kill {
            pid: 1234,
            name: "firefox".to_string(),
            signal: Signal::Term,
        });
        app
    }

    #[test]
    fn confirm_names_the_exact_target() {
        let text = all_text(&render_with(&app_with_confirm(), 100, 30));
        assert!(text.contains("firefox"));
        assert!(text.contains("1234"), "the PID disambiguates the target");
        assert!(text.contains("SIGTERM"));
    }

    #[test]
    fn confirm_offers_both_answers() {
        let text = all_text(&render_with(&app_with_confirm(), 100, 30));
        assert!(text.contains("cancel"));
        assert!(text.contains("confirm"));
    }

    #[test]
    fn confirm_scrubs_control_characters_from_the_target_name() {
        let mut app = app_with_data();
        app.confirm = Some(ConfirmAction::Kill {
            pid: 7,
            name: "evil\x1b[31mname".to_string(),
            signal: Signal::Kill,
        });
        let text = all_text(&render_with(&app, 100, 30));
        assert!(!text.contains('\x1b'));
    }

    #[test]
    fn confirm_outranks_every_other_overlay() {
        let mut app = app_with_confirm();
        app.overlay = crate::app::Overlay::Help;
        let text = all_text(&render_with(&app, 100, 30));
        assert!(
            text.contains("firefox"),
            "the destructive gate must stay on top:\n{text}"
        );
    }

    #[test]
    fn confirm_survives_tiny_terminals_and_ascii() {
        let mut app = app_with_confirm();
        for (w, h) in [(1u16, 1u16), (8, 3), (20, 6), (40, 10)] {
            let _ = render_with(&app, w, h);
        }
        let buf = render_caps(&mut app, 80, 24, ColorSupport::Basic, false);
        assert!(all_text(&buf).is_ascii());
    }
}
