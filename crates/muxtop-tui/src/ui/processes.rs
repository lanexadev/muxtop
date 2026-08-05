// Processes tab.
//
// The table itself now lives in `widgets::table`; what is left here is what is
// genuinely specific to processes: which columns exist, how a row is formatted,
// and how the tree connectors are drawn.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};

use super::Render;
use super::filter_bar;
use super::sanitize::scrub_ctrl;
use super::widgets::badge;
use super::widgets::columns::{Align, Column, PRIO_ESSENTIAL, PRIO_HIGH, PRIO_LOW, PRIO_MEDIUM};
use super::widgets::empty::{self, EmptyState};
use super::widgets::meter;
use super::widgets::table::{self, Cell, Row, Spec};
use crate::ui::theme::Level;
use muxtop_core::process::{ProcessInfo, SortField, SortOrder};

const COL_PID: usize = 0;
const COL_USER: usize = 1;
const COL_CPU: usize = 3;
const COL_MEM: usize = 4;
const COL_COMMAND: usize = 6;

const COLUMNS: &[Column] = &[
    Column::fixed("PID", 7, Align::Right, PRIO_ESSENTIAL),
    Column::fixed("USER", 10, Align::Left, PRIO_MEDIUM),
    Column::fixed("S", 2, Align::Left, PRIO_HIGH),
    Column::fixed("CPU%", 7, Align::Right, PRIO_HIGH),
    Column::fixed("MEM%", 7, Align::Right, PRIO_HIGH),
    Column::fixed("RSS", 8, Align::Right, PRIO_LOW),
    Column::flex("COMMAND", 12, PRIO_ESSENTIAL),
];

pub fn draw_processes_tab(frame: &mut Frame, area: Rect, r: &Render<'_>) {
    let app = r.app;

    if app.last_snapshot.is_none() {
        let waiting = r.ellipsis("Waiting for data");
        empty::render(frame, area, &EmptyState::waiting(&waiting), r.theme);
        return;
    }

    let filter_h = u16::from(app.filter_editing());
    let [table_area, filter_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(filter_h)]).areas(area);

    let filtered = !app.filter_text().is_empty();
    let empty_state = if filtered {
        EmptyState::no_match("No matching processes")
    } else {
        EmptyState::empty("No processes", None)
    };

    let spec = Spec {
        columns: COLUMNS,
        sort_col: sort_column(app.sort_field),
        descending: matches!(app.sort_order, SortOrder::Desc),
        total: app.process_count(),
        selected: app.selected,
        scroll: app.scroll_offset,
        col_scroll: app.col_scroll,
        empty: empty_state,
    };

    let tree = app.tree_mode;
    table::draw(frame, table_area, r, &spec, |idx| {
        if tree {
            let entries = &app.visible_tree;
            match entries.get(idx) {
                Some((proc, _)) => {
                    let prefix = tree_prefix(entries, idx, r);
                    process_row(proc, Some(&prefix), r)
                }
                None => Row::new(Vec::new()),
            }
        } else {
            match app.visible_processes.get(idx) {
                Some(proc) => process_row(proc, None, r),
                None => Row::new(Vec::new()),
            }
        }
    });

    if filter_h > 0 {
        filter_bar::draw(frame, filter_area, r, "Filter processes");
    }
}

/// Which table column the active sort field corresponds to.
fn sort_column(field: SortField) -> Option<usize> {
    Some(match field {
        SortField::Pid => COL_PID,
        SortField::User => COL_USER,
        SortField::Cpu => COL_CPU,
        SortField::Mem => COL_MEM,
        SortField::Name => COL_COMMAND,
    })
}

fn process_row(proc: &ProcessInfo, tree_prefix: Option<&str>, r: &Render<'_>) -> Row {
    // Process `comm` and `cmdline` come from `/proc/*/comm` and
    // `/proc/*/cmdline`; both are attacker-controlled by any local user able to
    // spawn a process, and they land in a Span verbatim.
    let command = scrub_ctrl(&proc.command);
    let command = match tree_prefix {
        Some(prefix) => format!("{prefix}{command}"),
        None => command.into_owned(),
    };
    let level = state_level(&proc.status);
    let theme = r.theme;

    Row::new(vec![
        Cell::new(proc.pid.to_string()),
        Cell::new(scrub_ctrl(&proc.user).into_owned()),
        Cell::colored(badge::marker(level, r.glyphs), theme.level_color(level)),
        Cell::colored(
            format!("{:.1}", proc.cpu_percent),
            theme.gauge_color(f64::from(proc.cpu_percent)),
        ),
        Cell::new(format!("{:.1}", proc.memory_percent)),
        Cell::new(meter::human_bytes(proc.memory_bytes)),
        Cell::new(command),
    ])
}

/// Severity of a process state, which decides both its marker and its colour.
fn state_level(status: &str) -> Level {
    match status {
        "Running" => Level::Success,
        "Sleeping" | "Idle" => Level::Neutral,
        "Zombie" => Level::Error,
        "Stopped" => Level::Warning,
        _ => Level::Info,
    }
}

/// Build the tree-connector prefix for the entry at `idx`.
///
/// 0.4 indented with plain spaces, so on a deep tree the lineage was lost after
/// the second level. The vertical continuation is drawn for every ancestor that
/// still has siblings below it.
fn tree_prefix(entries: &[(ProcessInfo, usize)], idx: usize, r: &Render<'_>) -> String {
    let depth = entries[idx].1;
    if depth == 0 {
        return String::new();
    }
    let g = r.glyphs;
    let mut prefix = String::with_capacity(depth * 4);
    for level in 1..depth {
        if ancestor_has_more_siblings(entries, idx, level) {
            prefix.push_str(g.tree_pipe);
        } else {
            prefix.push_str(g.tree_gap);
        }
    }
    prefix.push_str(if is_last_at_depth(entries, idx) {
        g.tree_last
    } else {
        g.tree_branch
    });
    prefix
}

/// Whether the entry at `idx` is the last of its siblings.
fn is_last_at_depth(entries: &[(ProcessInfo, usize)], idx: usize) -> bool {
    let depth = entries[idx].1;
    for (_, d) in &entries[idx + 1..] {
        if *d < depth {
            return true;
        }
        if *d == depth {
            return false;
        }
    }
    true
}

/// Whether the ancestor of `idx` at `level` still has siblings after this
/// subtree — i.e. whether a vertical connector belongs in that column.
fn ancestor_has_more_siblings(entries: &[(ProcessInfo, usize)], idx: usize, level: usize) -> bool {
    for (_, d) in &entries[idx + 1..] {
        if *d < level {
            return false;
        }
        if *d == level {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, Tab};
    use crate::terminal::ColorSupport;
    use crate::ui::test_support::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn app() -> AppState {
        let mut app = app_with_data();
        app.tab = Tab::Processes;
        app
    }

    #[test]
    fn table_renders_columns_and_rows() {
        let text = all_text(&render_with(&app(), 120, 24));
        assert!(text.contains("PID"));
        assert!(text.contains("COMMAND"));
        assert!(text.contains("proc0"));
    }

    #[test]
    fn rss_column_shows_absolute_memory() {
        // A percentage alone does not tell you whether to worry.
        let text = all_text(&render_with(&app(), 140, 24));
        assert!(text.contains("RSS"));
    }

    #[test]
    fn narrow_terminal_keeps_the_command_column() {
        let text = all_text(&render_with(&app(), 60, 24));
        assert!(
            text.contains("COMMAND"),
            "the identity column must survive:\n{text}"
        );
    }

    #[test]
    fn sort_indicator_follows_the_sort_field() {
        let mut app = app();
        app.sort_field = SortField::Mem;
        app.sort_order = SortOrder::Desc;
        let text = all_text(&render_with(&app, 140, 24));
        assert!(text.contains("MEM%▼"), "sort marker missing:\n{text}");
    }

    #[test]
    fn empty_filter_result_explains_itself() {
        // 0.4 rendered a blank table, indistinguishable from a machine with no
        // processes at all.
        let mut app = app();
        app.set_filter("zzzz-no-such-process");
        let text = all_text(&render_with(&app, 100, 24));
        assert!(text.contains("No matching processes"));
        assert!(text.contains("Esc"), "a dead end must offer a way out");
    }

    #[test]
    fn filter_bar_appears_only_while_editing() {
        let mut app = app();
        assert!(!all_text(&render_with(&app, 100, 24)).contains("Filter processes"));
        app.handle_key_event(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE));
        assert!(all_text(&render_with(&app, 100, 24)).contains("Filter processes"));
    }

    #[test]
    fn tree_view_draws_connectors() {
        let mut app = app();
        app.tree_mode = true;
        app.recompute_visible();
        let text = all_text(&render_with(&app, 120, 30));
        assert!(
            text.contains('├') || text.contains('└'),
            "tree connectors missing:\n{text}"
        );
    }

    #[test]
    fn tree_view_uses_ascii_connectors_without_unicode() {
        let mut app = app();
        app.tree_mode = true;
        app.recompute_visible();
        let buf = render_caps(&mut app, 120, 30, ColorSupport::Basic, false);
        let text = all_text(&buf);
        assert!(text.is_ascii());
        assert!(text.contains("|--") || text.contains("\\--"));
    }

    #[test]
    fn control_characters_never_reach_the_screen() {
        let mut snap = snapshot();
        snap.processes.insert(
            0,
            ProcessInfo {
                pid: 66,
                parent_pid: None,
                name: "evil".to_string(),
                command: "/bin/evil\x1b[31m\x07".to_string(),
                user: "ro\x1bot".to_string(),
                cpu_percent: 99.0,
                memory_bytes: 1,
                memory_percent: 0.1,
                status: "Running".to_string(),
            },
        );
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        app.apply_snapshot(snap);
        let text = all_text(&render_with(&app, 120, 24));
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('\x07'));
    }

    #[test]
    fn state_levels_distinguish_the_states_that_matter() {
        assert_eq!(state_level("Running"), Level::Success);
        assert_eq!(state_level("Zombie"), Level::Error);
        assert_eq!(state_level("Stopped"), Level::Warning);
        assert_ne!(state_level("Running"), state_level("Sleeping"));
    }
}
