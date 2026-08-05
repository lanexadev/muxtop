// Help overlay.
//
// The screen muxtop 0.4 did not have — its README said so out loud. Every row
// is generated from `keymap::BINDINGS`, so it cannot drift from what the keys
// actually do, and a binding added to the keymap documents itself here.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::keymap::{self, Group, Scope};
use crate::ui::Render;
use crate::ui::widgets::overlay;

/// Width below which the two columns collapse into one.
const TWO_COLUMN_WIDTH: u16 = 76;

/// Width reserved for the key column.
const KEY_WIDTH: usize = 16;

pub fn draw_help(frame: &mut Frame, r: &Render<'_>) {
    let area = frame.area();
    let popup = overlay::centered_percent(88, 88, area);
    let title = r.titled("Help", &format!("muxtop v{}", env!("CARGO_PKG_VERSION")));
    let inner = overlay::popup(frame, popup, &title, r.theme, r.glyphs);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Reserve the last row for the overlay's own instructions: an overlay with
    // no visible way out is a trap.
    let [body, footer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);

    let (left, right) = sections(r);

    if body.width >= TWO_COLUMN_WIDTH {
        let [l, gap, rr] = Layout::horizontal([
            Constraint::Percentage(50),
            Constraint::Length(2),
            Constraint::Fill(1),
        ])
        .areas(body);
        let _ = gap;
        render_column(frame, l, &left, r.app.overlay_scroll);
        render_column(frame, rr, &right, r.app.overlay_scroll);
    } else {
        let mut all = left;
        all.push(Line::from(""));
        all.extend(right);
        render_column(frame, body, &all, r.app.overlay_scroll);
    }

    let hint = Line::from(vec![
        Span::styled(" ↑↓ ", r.theme.key()),
        Span::styled(" scroll   ", r.theme.key_desc()),
        Span::styled(" ? ", r.theme.key()),
        Span::styled(" or ", r.theme.key_desc()),
        Span::styled(" Esc ", r.theme.key()),
        Span::styled(" close ", r.theme.key_desc()),
    ]);
    frame.render_widget(Paragraph::new(ascii_safe(hint, r)), footer);
}

/// Replace the arrow glyphs in the footer hint on ASCII terminals.
fn ascii_safe<'a>(line: Line<'a>, r: &Render<'_>) -> Line<'a> {
    if r.glyphs.unicode {
        return line;
    }
    Line::from(
        line.spans
            .into_iter()
            .map(|s| {
                let content = s
                    .content
                    .replace('↑', r.glyphs.arrow_up)
                    .replace('↓', r.glyphs.arrow_down);
                Span::styled(content, s.style)
            })
            .collect::<Vec<_>>(),
    )
}

/// Build the two columns of the help screen.
///
/// The active tab's own bindings come first: they are the ones a user cannot
/// guess, and the ones 0.4 documented nowhere but the README.
fn sections(r: &Render<'_>) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
    let app = r.app;
    let mut left: Vec<Line<'static>> = Vec::new();
    let mut right: Vec<Line<'static>> = Vec::new();

    let tab_rows = keymap::help_rows(app.tab, Scope::Tab(app.tab));
    if !tab_rows.is_empty() {
        left.push(heading(
            &format!("THIS TAB {} {}", r.glyphs.dash, app.tab.label()),
            r,
        ));
        for (keys, label, _) in &tab_rows {
            left.push(binding_line(keys, label, r));
        }
        left.push(Line::from(""));
    }

    let global = keymap::help_rows(app.tab, Scope::Global);
    // Actions and table keys go on the left under the tab-specific block;
    // navigation, sorting and application keys on the right.
    let left_groups = [Group::Actions, Group::Table];
    let right_groups = [Group::Navigation, Group::SortFilter, Group::App];

    for group in left_groups {
        push_group(&mut left, &global, group, r);
    }
    for group in right_groups {
        push_group(&mut right, &global, group, r);
    }

    if app.is_remote() {
        right.push(Line::from(""));
        right.push(Line::from(Span::styled(
            "Remote mode: kill, renice and container",
            r.theme.level_style(crate::ui::theme::Level::Warning),
        )));
        right.push(Line::from(Span::styled(
            "actions are disabled: the server owns them.",
            r.theme.level_style(crate::ui::theme::Level::Warning),
        )));
    }

    (left, right)
}

fn push_group(
    out: &mut Vec<Line<'static>>,
    rows: &[(String, &'static str, Group)],
    group: Group,
    r: &Render<'_>,
) {
    let in_group: Vec<_> = rows.iter().filter(|(_, _, g)| *g == group).collect();
    if in_group.is_empty() {
        return;
    }
    out.push(heading(group.title(), r));
    for (keys, label, _) in in_group {
        out.push(binding_line(keys, label, r));
    }
    out.push(Line::from(""));
}

fn heading(text: &str, r: &Render<'_>) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), r.theme.accent()))
}

fn binding_line(keys: &str, label: &str, r: &Render<'_>) -> Line<'static> {
    // Keys are already ASCII-safe: `keymap` stores arrow glyphs for the arrow
    // keys, so they are translated here for terminals without them.
    let keys = if r.glyphs.unicode {
        keys.to_string()
    } else {
        keys.replace('↑', "Up")
            .replace('↓', "Down")
            .replace('←', "Left")
            .replace('→', "Right")
    };
    let padded = format!("{keys:<KEY_WIDTH$}");
    Line::from(vec![
        Span::styled(padded, r.theme.strong()),
        Span::styled(label.to_string(), r.theme.dim()),
    ])
}

fn render_column(frame: &mut Frame, area: Rect, lines: &[Line<'static>], scroll: usize) {
    if area.height == 0 {
        return;
    }
    let max_scroll = lines.len().saturating_sub(area.height as usize);
    let scroll = scroll.min(max_scroll);
    let visible: Vec<Line<'static>> = lines
        .iter()
        .skip(scroll)
        .take(area.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(visible), area);
}

#[cfg(test)]
mod tests {

    use crate::app::{Overlay, Tab};
    use crate::terminal::ColorSupport;
    use crate::ui::test_support::*;

    fn help_text(tab: Tab, w: u16, h: u16) -> String {
        let mut app = app_with_data();
        app.tab = tab;
        app.overlay = Overlay::Help;
        all_text(&render_with(&app, w, h))
    }

    #[test]
    fn help_opens_and_names_itself() {
        let text = help_text(Tab::Processes, 100, 30);
        assert!(text.contains("Help"));
        assert!(text.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn help_always_shows_the_way_out() {
        // An overlay with no visible exit is a trap.
        let text = help_text(Tab::General, 100, 30);
        assert!(text.contains("Esc"));
        assert!(text.contains("close"));
    }

    #[test]
    fn help_leads_with_the_active_tab() {
        let text = help_text(Tab::Kube, 100, 30);
        assert!(text.contains("THIS TAB"));
        assert!(text.contains("Kubernetes") || text.contains("Kube"));
    }

    #[test]
    fn help_documents_the_keys_that_only_existed_in_the_readme() {
        let kube = help_text(Tab::Kube, 120, 40);
        assert!(
            kube.contains("namespace scope"),
            "the `A` key was documented nowhere in the app"
        );
        let procs = help_text(Tab::Processes, 120, 40);
        assert!(
            procs.contains("nice"),
            "renice must be discoverable in-app: {procs}"
        );
    }

    #[test]
    fn help_shows_no_kill_binding_for_the_network_tab() {
        let net = help_text(Tab::Network, 120, 40);
        assert!(
            !net.contains("SIGKILL"),
            "F10 does not force-kill from the Network tab any more"
        );
    }

    #[test]
    fn help_warns_about_remote_mode() {
        let mut app = app_with_data();
        app.overlay = Overlay::Help;
        app.connection_mode = crate::ConnectionMode::Remote {
            hostname: "prod".into(),
            addr: "10.0.0.1:4242".parse().unwrap(),
        };
        let text = all_text(&render_with(&app, 120, 40));
        assert!(text.contains("Remote mode"));
    }

    #[test]
    fn help_collapses_to_one_column_when_narrow() {
        // Both column contents must still be reachable; nothing may be dropped
        // just because the terminal is small.
        let narrow = help_text(Tab::Processes, 62, 40);
        assert!(narrow.contains("NAVIGATION"));
        assert!(narrow.contains("ACTIONS"));
    }

    #[test]
    fn help_survives_a_tiny_terminal() {
        for (w, h) in [(1u16, 1u16), (10, 4), (20, 6), (40, 8)] {
            let _ = help_text(Tab::Processes, w, h);
        }
    }

    #[test]
    fn help_is_ascii_in_ascii_mode() {
        let mut app = app_with_data();
        app.overlay = Overlay::Help;
        let buf = render_caps(&mut app, 100, 30, ColorSupport::Basic, false);
        let text = all_text(&buf);
        assert!(
            text.is_ascii(),
            "help leaked Unicode in ASCII mode:\n{text}"
        );
    }

    #[test]
    fn help_scrolls() {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app.overlay = Overlay::Help;
        let top = all_text(&render_with(&app, 100, 14));
        app.overlay_scroll = 5;
        let scrolled = all_text(&render_with(&app, 100, 14));
        assert_ne!(top, scrolled, "the help screen must scroll");
    }
}
