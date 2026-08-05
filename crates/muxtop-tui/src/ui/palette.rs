// Command palette (`Ctrl+P`) and command line (`:`).
//
// Three things changed from 0.4. The list is context-aware, so the tab in front
// of you ranks first. Matched characters are highlighted, so it is obvious why
// a result is there. And argument forms work — `kill firefox`, `sort mem`,
// `theme mono` — which the README had been advertising since 0.3 against a
// command enum that could not carry an argument.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::Command;
use crate::ui::Render;
use crate::ui::widgets::overlay;

/// Preferred width of the palette.
const WIDTH: u16 = 64;
/// Most results shown at once.
const MAX_RESULTS: usize = 12;

pub fn draw_palette(frame: &mut Frame, r: &Render<'_>) {
    let app = r.app;
    let area = frame.area();

    let result_rows = app.palette.filtered.len().clamp(1, MAX_RESULTS) as u16;
    // border(2) + input(1) + separator(1) + results + hint(1)
    let height = (result_rows + 5).min(area.height);
    let width = WIDTH.min(area.width.saturating_sub(2)).max(20);
    let popup = overlay::centered(width, height, area);

    let title = if app.command_mode() {
        "Command"
    } else {
        "Command Palette"
    };
    let inner = overlay::popup(frame, popup, title, r.theme, r.glyphs);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let [input_area, results_area, hint_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    draw_input(frame, input_area, r);

    if app.palette.filtered.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(vec![Span::styled(
                "  No matches",
                r.theme.dim(),
            )])),
            results_area,
        );
    } else {
        draw_results(frame, results_area, r);
    }

    draw_hint(frame, hint_area, r);
}

fn draw_input(frame: &mut Frame, area: Rect, r: &Render<'_>) {
    let app = r.app;
    let prompt = if app.command_mode() { ":" } else { ">" };
    let mut spans = vec![
        Span::styled(format!("{prompt} "), r.theme.accent()),
        Span::styled(app.palette.input.clone(), r.theme.body()),
        Span::styled(r.glyphs.cursor.to_string(), r.theme.accent()),
    ];
    // Show the parsed argument so `kill firefox` visibly became a target.
    if let Some(arg) = app.palette.arg.as_ref()
        && !arg.is_empty()
    {
        spans.push(Span::styled(
            format!("   {} {arg}", r.glyphs.chevron),
            r.theme.subtle(),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_results(frame: &mut Frame, area: Rect, r: &Render<'_>) {
    let app = r.app;
    let visible = (area.height as usize).min(app.palette.filtered.len());
    if visible == 0 {
        return;
    }
    // Keep the cursor on screen.
    let scroll = app
        .palette
        .selected
        .saturating_sub(visible.saturating_sub(1));

    for (i, &(cmd, _)) in app
        .palette
        .filtered
        .iter()
        .skip(scroll)
        .take(visible)
        .enumerate()
    {
        let selected = scroll + i == app.palette.selected;
        let row = Rect {
            x: area.x,
            y: area.y + i as u16,
            width: area.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(result_line(cmd, selected, row.width, r)),
            row,
        );
    }
}

fn result_line(cmd: Command, selected: bool, width: u16, r: &Render<'_>) -> Line<'static> {
    let theme = r.theme;
    let base = if selected {
        theme.selected_row()
    } else {
        theme.body()
    };
    let shortcut = cmd.shortcut();
    let label = cmd.label();

    // Layout: edge + label + padding + shortcut.
    let edge_w = 2usize;
    let shortcut_w = shortcut.chars().count() + 2;
    let label_w = (width as usize).saturating_sub(edge_w + shortcut_w);

    let mut spans = Vec::with_capacity(6);
    spans.push(Span::styled(
        if selected {
            format!("{} ", r.glyphs.sel_edge)
        } else {
            "  ".to_string()
        },
        if selected {
            theme.level_style(crate::ui::theme::Level::Info)
        } else {
            base
        },
    ));

    // Highlight the characters the query matched, so the ranking is explainable.
    let query = if r.app.palette.arg.is_some() {
        String::new()
    } else {
        r.app.palette.input.to_lowercase()
    };
    let highlight = base.fg(theme.accent_primary);
    let truncated = r.glyphs.truncate(label, label_w);
    let mut rendered = 0usize;
    let mut q = query.chars().peekable();
    let mut run = String::new();
    let mut run_hit = false;
    for ch in truncated.chars() {
        let hit = q
            .peek()
            .is_some_and(|c| c.eq_ignore_ascii_case(&ch.to_ascii_lowercase()));
        if hit {
            q.next();
        }
        if hit != run_hit && !run.is_empty() {
            spans.push(Span::styled(
                std::mem::take(&mut run),
                if run_hit { highlight } else { base },
            ));
        }
        run_hit = hit;
        run.push(ch);
        rendered += 1;
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_hit { highlight } else { base }));
    }
    spans.push(Span::styled(
        " ".repeat(label_w.saturating_sub(rendered)),
        base,
    ));
    spans.push(Span::styled(
        format!("{shortcut:>w$}  ", w = shortcut_w.saturating_sub(2)),
        if selected { base } else { theme.subtle() },
    ));
    Line::from(spans)
}

fn draw_hint(frame: &mut Frame, area: Rect, r: &Render<'_>) {
    let hint = if r.app.command_mode() {
        Line::from(vec![
            Span::styled(" try ", r.theme.subtle()),
            Span::styled("kill firefox", r.theme.dim()),
            Span::styled("  ", r.theme.subtle()),
            Span::styled("sort mem", r.theme.dim()),
            Span::styled("  ", r.theme.subtle()),
            Span::styled("theme mono", r.theme.dim()),
        ])
    } else {
        Line::from(vec![
            Span::styled(" Enter ", r.theme.key()),
            Span::styled(" run  ", r.theme.key_desc()),
            Span::styled(" : ", r.theme.key()),
            Span::styled(" command with arguments  ", r.theme.key_desc()),
            Span::styled(" Esc ", r.theme.key()),
            Span::styled(" close ", r.theme.key_desc()),
        ])
    };
    frame.render_widget(Paragraph::new(hint), area);
}

#[cfg(test)]
mod tests {
    use crate::app::{AppState, Overlay, Tab};
    use crate::terminal::ColorSupport;
    use crate::ui::test_support::*;

    fn palette_text(app: &AppState) -> String {
        all_text(&render_with(app, 100, 30))
    }

    fn open(input: &str) -> AppState {
        let mut app = app_with_data();
        app.overlay = Overlay::Palette;
        app.palette.input = input.to_string();
        app.palette.refilter_ctx(&[], Some(app.tab));
        app
    }

    #[test]
    fn palette_renders_and_names_itself() {
        let app = open("");
        assert!(palette_text(&app).contains("Command Palette"));
    }

    #[test]
    fn palette_lists_commands_with_their_shortcuts() {
        let app = open("");
        let text = palette_text(&app);
        assert!(text.contains("Quit"));
        assert!(text.contains("Help"));
    }

    #[test]
    fn palette_says_when_nothing_matches() {
        let app = open("zzzzzz");
        assert!(palette_text(&app).contains("No matches"));
    }

    #[test]
    fn palette_hides_argument_forms_until_their_verb_is_typed() {
        let empty = palette_text(&open(""));
        assert!(
            !empty.contains("Kill process by name"),
            "an argument command with no argument is noise"
        );
        let typed = palette_text(&open("kill firefox"));
        assert!(typed.contains("Kill process by name"));
    }

    #[test]
    fn palette_shows_the_parsed_argument() {
        let text = palette_text(&open("kill firefox"));
        assert!(
            text.contains("firefox"),
            "the target must be visible before Enter:\n{text}"
        );
    }

    #[test]
    fn command_mode_advertises_the_argument_forms() {
        let mut app = open("");
        app.overlay = Overlay::Command;
        let text = palette_text(&app);
        assert!(text.contains("Command"));
        assert!(text.contains("kill firefox"), "usage hint missing:\n{text}");
    }

    #[test]
    fn palette_ranks_the_active_tab_first() {
        let mut app = app_with_data();
        app.tab = Tab::Containers;
        app.overlay = Overlay::Palette;
        app.palette.input = "sort".to_string();
        app.palette.refilter_ctx(&[], Some(Tab::Containers));
        let first = app.palette.filtered.first().map(|&(c, _)| c.label());
        assert!(
            first.is_some_and(|l| l.contains("container")),
            "on the Containers tab, a sort query should surface container sorts first, got {first:?}"
        );
    }

    #[test]
    fn palette_offers_the_kube_commands_that_had_no_entry_before() {
        let app = open("kube");
        let text = palette_text(&app);
        assert!(
            text.contains("namespace scope") || text.contains("Pods"),
            "kube sub-views were unreachable from the 0.4 palette:\n{text}"
        );
    }

    #[test]
    fn palette_survives_tiny_terminals_and_ascii() {
        let mut app = open("s");
        for (w, h) in [(1u16, 1u16), (10, 4), (24, 8), (40, 12)] {
            let _ = render_with(&app, w, h);
        }
        let buf = render_caps(&mut app, 80, 24, ColorSupport::Basic, false);
        assert!(all_text(&buf).is_ascii());
    }

    #[test]
    fn palette_scrolls_to_keep_the_selection_visible() {
        let mut app = open("");
        app.palette.selected = app.palette.filtered.len() - 1;
        let text = palette_text(&app);
        let last = app.palette.filtered.last().unwrap().0.label();
        assert!(text.contains(last), "selection scrolled out of view");
    }

    #[test]
    fn palette_is_absent_when_closed() {
        let app = app_with_data();
        assert!(!palette_text(&app).contains("Command Palette"));
    }
}
