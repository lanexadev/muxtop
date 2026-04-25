// Processes tab — sortable, scrollable, filterable process table with tree view.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::sanitize::scrub_ctrl;
use super::theme::Theme;
use crate::app::AppState;
use muxtop_core::process::{ProcessInfo, SortField, SortOrder};

// Fixed column widths.
const COL_PID: usize = 7;
const COL_USER: usize = 10;
const COL_STATUS: usize = 2;
const COL_CPU: usize = 7;
const COL_MEM: usize = 7;
const FIXED_COLS: usize = COL_PID + COL_USER + COL_STATUS + COL_CPU + COL_MEM;

/// Render the Processes tab content area.
pub fn draw_processes_tab(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    if app.last_snapshot.is_none() {
        let para = Paragraph::new("Waiting for data...").alignment(Alignment::Center);
        frame.render_widget(para, area);
        return;
    }

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme.text_dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let filter_h = u16::from(app.filter_active);
    let [table_area, filter_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(filter_h)]).areas(inner);

    if table_area.height >= 2 {
        let [header_area, body_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(table_area);

        draw_header(frame, header_area, app, theme);
        draw_body(frame, body_area, app, theme);
    }

    if app.filter_active {
        draw_filter_bar(frame, filter_area, app, theme);
    }
}

/// Render the column header row with a sort indicator on the active column.
fn draw_header(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let arrow = if app.term_caps.unicode {
        match app.sort_order {
            SortOrder::Desc => "▼",
            SortOrder::Asc => "▲",
        }
    } else {
        match app.sort_order {
            SortOrder::Desc => "v",
            SortOrder::Asc => "^",
        }
    };
    let cmd_w = (area.width as usize).saturating_sub(FIXED_COLS);
    let style = Style::default()
        .fg(theme.accent_primary)
        .bg(theme.header_bg)
        .add_modifier(Modifier::BOLD);

    let header = format!(
        "{}{}{}{}{}{}",
        col_text(
            &sort_label("PID", SortField::Pid, app.sort_field, arrow),
            COL_PID
        ),
        col_text(
            &sort_label("USER", SortField::User, app.sort_field, arrow),
            COL_USER,
        ),
        col_text("S", COL_STATUS),
        col_text(
            &sort_label("CPU%", SortField::Cpu, app.sort_field, arrow),
            COL_CPU
        ),
        col_text(
            &sort_label("MEM%", SortField::Mem, app.sort_field, arrow),
            COL_MEM,
        ),
        col_text(
            &sort_label("COMMAND", SortField::Name, app.sort_field, arrow),
            cmd_w,
        ),
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(header, style))),
        area,
    );
}

/// Render the process rows with virtualized scrolling.
fn draw_body(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let vis_h = area.height as usize;
    if vis_h == 0 {
        return;
    }

    let scroll = effective_scroll(app.selected, app.scroll_offset, vis_h);
    let cmd_w = (area.width as usize).saturating_sub(FIXED_COLS);

    let unicode = app.term_caps.unicode;
    let lines: Vec<Line<'static>> = if app.tree_mode {
        let entries = &app.visible_tree;
        let end = (scroll + vis_h).min(entries.len());
        (scroll..end)
            .map(|i| {
                let pfx = tree_prefix_with_unicode(entries, i, unicode);
                process_row(
                    &entries[i].0,
                    cmd_w,
                    i == app.selected,
                    Some(&pfx),
                    theme,
                    unicode,
                    i,
                )
            })
            .collect()
    } else {
        let entries = &app.visible_processes;
        let end = (scroll + vis_h).min(entries.len());
        (scroll..end)
            .map(|i| {
                process_row(
                    &entries[i],
                    cmd_w,
                    i == app.selected,
                    None,
                    theme,
                    unicode,
                    i,
                )
            })
            .collect()
    };

    frame.render_widget(Paragraph::new(lines), area);
}

/// Render the filter input bar at the bottom of the content area.
fn draw_filter_bar(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let cursor = if app.term_caps.unicode { "█" } else { "_" };
    let line = Line::from(vec![
        Span::styled(
            "Filter: ",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}{cursor}", app.filter_input),
            Style::default().fg(theme.fg),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ---------------------------------------------------------------------------
// Row rendering
// ---------------------------------------------------------------------------

/// Format a single process row with fixed-width columns.
fn process_row(
    proc: &ProcessInfo,
    cmd_w: usize,
    selected: bool,
    tree_pfx: Option<&str>,
    theme: &Theme,
    unicode: bool,
    row_idx: usize,
) -> Line<'static> {
    let (sc, sc_color) = status_style(&proc.status, theme, unicode);
    // Scrub external strings before they hit a Span (MED-S5). Process `comm`
    // and `cmdline` come from `/proc/*/comm` and `/proc/*/cmdline` — both are
    // attacker-controlled by any local user able to spawn a process.
    let safe_command = scrub_ctrl(&proc.command);
    let safe_user = scrub_ctrl(&proc.user);
    let cmd_text = match tree_pfx {
        Some(pfx) => format!("{pfx}{safe_command}"),
        None => safe_command.into_owned(),
    };

    let bg = if selected {
        theme.selection_bg
    } else if row_idx % 2 == 1 {
        theme.surface
    } else {
        theme.bg
    };
    let fg = if selected {
        theme.selection_fg
    } else {
        theme.fg
    };
    let base = if selected {
        Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(bg).fg(fg)
    };
    let st = if selected {
        base.fg(sc_color)
    } else {
        Style::default().fg(sc_color).bg(bg)
    };

    Line::from(vec![
        Span::styled(format!("{:>6} ", proc.pid), base),
        Span::styled(col_text(&safe_user, COL_USER), base),
        Span::styled(format!("{sc} "), st),
        Span::styled(format!("{:>5.1} ", proc.cpu_percent), base),
        Span::styled(format!("{:>5.1} ", proc.memory_percent), base),
        Span::styled(col_text(&cmd_text, cmd_w), base),
    ])
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Build the column header label, appending a sort arrow when this column is active.
fn sort_label(name: &str, field: SortField, active: SortField, arrow: &str) -> String {
    if std::mem::discriminant(&field) == std::mem::discriminant(&active) {
        format!("{name}{arrow}")
    } else {
        name.to_string()
    }
}

/// Pad (left-aligned) or truncate a string to exactly `width` characters.
fn col_text(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let truncated: String = s.chars().take(width).collect();
    format!("{truncated:<width$}")
}

/// Compute the effective scroll offset so that `selected` is always visible.
fn effective_scroll(selected: usize, scroll_offset: usize, visible_height: usize) -> usize {
    if visible_height == 0 {
        return 0;
    }
    if selected < scroll_offset {
        selected
    } else if selected >= scroll_offset + visible_height {
        selected.saturating_sub(visible_height - 1)
    } else {
        scroll_offset
    }
}

/// Map a process status string to a single character and color.
fn status_style(status: &str, theme: &Theme, unicode: bool) -> (char, Color) {
    if unicode {
        match status {
            "Running" => ('●', theme.success),
            "Sleeping" | "Idle" => ('○', theme.sleeping),
            "Zombie" => ('⚠', theme.danger),
            "Stopped" => ('⏸', theme.warning),
            _ => ('?', theme.text_dim),
        }
    } else {
        match status {
            "Running" => ('R', theme.success),
            "Sleeping" | "Idle" => ('S', theme.sleeping),
            "Zombie" => ('Z', theme.danger),
            "Stopped" => ('T', theme.warning),
            _ => ('?', theme.text_dim),
        }
    }
}

// ---------------------------------------------------------------------------
// Tree view helpers
// ---------------------------------------------------------------------------

/// Check whether the entry at `idx` is the last sibling at its depth level.
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

/// Build the tree-connector prefix string (Unicode mode — used in tests).
#[cfg(test)]
fn tree_prefix(entries: &[(ProcessInfo, usize)], idx: usize) -> String {
    tree_prefix_with_unicode(entries, idx, true)
}

/// Build the tree-connector prefix string, with optional ASCII fallback.
fn tree_prefix_with_unicode(entries: &[(ProcessInfo, usize)], idx: usize, unicode: bool) -> String {
    let depth = entries[idx].1;
    if depth == 0 {
        return String::new();
    }
    let connector = if is_last_at_depth(entries, idx) {
        if unicode { "└── " } else { "\\-- " }
    } else if unicode {
        "├── "
    } else {
        "|-- "
    };
    let indent = "  ".repeat(depth.saturating_sub(1));
    format!("{indent}{connector}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, Tab};
    use muxtop_core::system::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_with(app: &AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| crate::ui::draw_root(frame, app))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_line_text(buf: &ratatui::buffer::Buffer, row: u16) -> String {
        let width = buf.area.width;
        (0..width)
            .map(|col| buf.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn buffer_contains(buf: &ratatui::buffer::Buffer, needle: &str) -> bool {
        let height = buf.area.height;
        (0..height).any(|row| buffer_line_text(buf, row).contains(needle))
    }

    fn make_proc(pid: u32, name: &str, cpu: f32, mem_pct: f32) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid: None,
            name: name.to_string(),
            command: format!("/usr/bin/{name}"),
            user: "user".to_string(),
            cpu_percent: cpu,
            memory_bytes: 1000,
            memory_percent: mem_pct,
            status: "Running".to_string(),
        }
    }

    fn make_proc_with_parent(pid: u32, ppid: Option<u32>, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid: ppid,
            name: name.to_string(),
            command: format!("/usr/bin/{name}"),
            user: "root".to_string(),
            cpu_percent: 1.0,
            memory_bytes: 1000,
            memory_percent: 1.0,
            status: "Running".to_string(),
        }
    }

    fn make_snapshot(procs: Vec<ProcessInfo>) -> SystemSnapshot {
        SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 25.0,
                cores: vec![],
            },
            memory: MemorySnapshot {
                total: 16_000_000_000,
                used: 8_000_000_000,
                available: 8_000_000_000,
                swap_total: 0,
                swap_used: 0,
            },
            load: LoadSnapshot {
                one: 1.0,
                five: 0.8,
                fifteen: 0.5,
                uptime_secs: 3600,
            },
            processes: procs,
            networks: muxtop_core::network::NetworkSnapshot {
                interfaces: vec![],
                total_rx: 0,
                total_tx: 0,
            },
            containers: None,
            timestamp_ms: 0,
        }
    }

    fn processes_app(procs: Vec<ProcessInfo>) -> AppState {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        app.apply_snapshot(make_snapshot(procs));
        app
    }

    // ---- Pure helper unit tests ----

    #[test]
    fn test_effective_scroll_in_view() {
        assert_eq!(effective_scroll(5, 0, 10), 0);
        assert_eq!(effective_scroll(9, 0, 10), 0);
    }

    #[test]
    fn test_effective_scroll_below_view() {
        assert_eq!(effective_scroll(15, 0, 10), 6);
    }

    #[test]
    fn test_effective_scroll_above_view() {
        assert_eq!(effective_scroll(2, 10, 10), 2);
    }

    #[test]
    fn test_effective_scroll_zero_height() {
        assert_eq!(effective_scroll(5, 0, 0), 0);
    }

    #[test]
    fn test_status_running() {
        let theme = super::super::theme::Theme::default();
        let (c, color) = status_style("Running", &theme, false);
        assert_eq!(c, 'R');
        assert_eq!(color, theme.success);
    }

    #[test]
    fn test_status_sleeping() {
        let theme = super::super::theme::Theme::default();
        let (c, _) = status_style("Sleeping", &theme, false);
        assert_eq!(c, 'S');
    }

    #[test]
    fn test_status_zombie() {
        let theme = super::super::theme::Theme::default();
        let (c, color) = status_style("Zombie", &theme, false);
        assert_eq!(c, 'Z');
        assert_eq!(color, theme.danger);
    }

    #[test]
    fn test_status_stopped() {
        let theme = super::super::theme::Theme::default();
        let (c, color) = status_style("Stopped", &theme, false);
        assert_eq!(c, 'T');
        assert_eq!(color, theme.warning);
    }

    #[test]
    fn test_status_unknown() {
        let theme = super::super::theme::Theme::default();
        let (c, _) = status_style("SomethingElse", &theme, false);
        assert_eq!(c, '?');
    }

    #[test]
    fn test_col_text_pad() {
        assert_eq!(col_text("hi", 5), "hi   ");
    }

    #[test]
    fn test_col_text_truncate() {
        assert_eq!(col_text("hello world", 5), "hello");
    }

    #[test]
    fn test_col_text_exact() {
        assert_eq!(col_text("abcde", 5), "abcde");
    }

    #[test]
    fn test_col_text_zero_width() {
        assert_eq!(col_text("anything", 0), "");
    }

    #[test]
    fn test_sort_label_active() {
        let label = sort_label("CPU%", SortField::Cpu, SortField::Cpu, "▼");
        assert_eq!(label, "CPU%▼");
    }

    #[test]
    fn test_sort_label_inactive() {
        let label = sort_label("CPU%", SortField::Cpu, SortField::Mem, "▼");
        assert_eq!(label, "CPU%");
    }

    // ---- Tree prefix tests ----

    #[test]
    fn test_tree_prefix_root() {
        let entries = vec![(make_proc(1, "init", 1.0, 1.0), 0)];
        assert_eq!(tree_prefix(&entries, 0), "");
    }

    #[test]
    fn test_tree_prefix_last_child() {
        let entries = vec![
            (make_proc(1, "init", 1.0, 1.0), 0),
            (make_proc(2, "child", 1.0, 1.0), 1),
        ];
        assert_eq!(tree_prefix(&entries, 1), "└── ");
    }

    #[test]
    fn test_tree_prefix_non_last_child() {
        let entries = vec![
            (make_proc(1, "init", 1.0, 1.0), 0),
            (make_proc(2, "child1", 1.0, 1.0), 1),
            (make_proc(3, "child2", 1.0, 1.0), 1),
        ];
        assert_eq!(tree_prefix(&entries, 1), "├── ");
        assert_eq!(tree_prefix(&entries, 2), "└── ");
    }

    #[test]
    fn test_tree_prefix_grandchild() {
        let entries = vec![
            (make_proc(1, "init", 1.0, 1.0), 0),
            (make_proc(2, "child", 1.0, 1.0), 1),
            (make_proc(3, "grandchild", 1.0, 1.0), 2),
        ];
        assert_eq!(tree_prefix(&entries, 2), "  └── ");
    }

    #[test]
    fn test_is_last_at_depth_single() {
        let entries = vec![
            (make_proc(1, "init", 1.0, 1.0), 0),
            (make_proc(2, "child", 1.0, 1.0), 1),
        ];
        assert!(is_last_at_depth(&entries, 1));
    }

    #[test]
    fn test_is_last_at_depth_with_sibling() {
        let entries = vec![
            (make_proc(1, "init", 1.0, 1.0), 0),
            (make_proc(2, "child1", 1.0, 1.0), 1),
            (make_proc(3, "child2", 1.0, 1.0), 1),
        ];
        assert!(!is_last_at_depth(&entries, 1));
        assert!(is_last_at_depth(&entries, 2));
    }

    // ---- Rendering tests (AC coverage) ----

    // AC-01: Header columns
    #[test]
    fn test_header_shows_columns() {
        let app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "PID"));
        assert!(buffer_contains(&buf, "USER"));
        assert!(buffer_contains(&buf, "CPU%"));
        assert!(buffer_contains(&buf, "MEM%"));
        assert!(buffer_contains(&buf, "COMMAND"));
    }

    // AC-02: Process data visible
    #[test]
    fn test_rows_show_process_data() {
        let app = processes_app(vec![make_proc(1234, "firefox", 25.3, 10.2)]);
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "1234"));
        assert!(buffer_contains(&buf, "user"));
        assert!(buffer_contains(&buf, "25.3"));
    }

    // AC-03: Selected row
    #[test]
    fn test_selected_row_visible() {
        let mut app = processes_app(vec![
            make_proc(1, "alpha", 90.0, 5.0),
            make_proc(2, "bravo", 50.0, 3.0),
            make_proc(3, "charlie", 10.0, 1.0),
        ]);
        // sorted desc by CPU: alpha(90), bravo(50), charlie(10)
        app.selected = 1;
        let buf = render_with(&app, 80, 24);
        // Selected process (bravo) should be visible
        assert!(buffer_contains(&buf, "bravo"));
    }

    #[test]
    fn test_selected_row_has_background() {
        let mut app = processes_app(vec![
            make_proc(1, "alpha", 90.0, 5.0),
            make_proc(2, "bravo", 50.0, 3.0),
        ]);
        app.selected = 0;
        let buf = render_with(&app, 80, 24);
        // The table is bordered, so row 3 is the top border, row 4 is header, row 5 is selected process data.
        let cell = buf.cell((2, 5)).unwrap(); // column 2 just inside border
        let theme = super::super::theme::Theme::default();
        assert_eq!(
            cell.bg, theme.selection_bg,
            "Selected row should have selection background"
        );
    }

    // AC-05: Sort indicator
    #[test]
    fn test_sort_indicator_desc() {
        let app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "▼"));
    }

    #[test]
    fn test_sort_indicator_asc() {
        let mut app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        app.sort_order = SortOrder::Asc;
        app.recompute_visible();
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "▲"));
    }

    // AC-06: No data
    #[test]
    fn test_no_data_shows_waiting() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Waiting for data..."));
    }

    // AC-08: Scroll follows selection
    #[test]
    fn test_scroll_follows_selection() {
        let procs: Vec<ProcessInfo> = (0..100)
            .map(|i| make_proc(i, &format!("proc{i}"), i as f32, 1.0))
            .collect();
        let mut app = processes_app(procs);
        app.selected = 50;
        let buf = render_with(&app, 80, 24);
        // The selected process should be visible in the rendered buffer
        assert!(buffer_contains(&buf, "proc50"));
    }

    // AC-09: Filter bar
    #[test]
    fn test_filter_bar_shown() {
        let mut app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        app.filter_active = true;
        app.filter_input = "fire".to_string();
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Filter:"));
        assert!(buffer_contains(&buf, "fire"));
    }

    #[test]
    fn test_filter_bar_hidden_when_inactive() {
        let app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        let buf = render_with(&app, 80, 24);
        assert!(!buffer_contains(&buf, "Filter:"));
    }

    // AC-10: Filter reduces visible rows
    #[test]
    fn test_filter_reduces_visible() {
        let mut app = processes_app(vec![
            make_proc(1, "firefox", 10.0, 5.0),
            make_proc(2, "chrome", 20.0, 8.0),
        ]);
        app.filter_input = "fire".to_string();
        app.recompute_visible();
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "firefox"));
        assert!(!buffer_contains(&buf, "chrome"));
    }

    // AC-11: Tree connectors
    #[test]
    fn test_tree_mode_shows_connectors() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        let snap = make_snapshot(vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(10, Some(1), "sshd"),
            make_proc_with_parent(100, Some(10), "bash"),
        ]);
        app.apply_snapshot(snap);
        app.tree_mode = true;
        app.recompute_visible();
        let buf = render_with(&app, 80, 24);
        assert!(
            buffer_contains(&buf, "└──") || buffer_contains(&buf, "├──"),
            "Tree mode should show connectors"
        );
    }

    // AC-12: Tree depth
    #[test]
    fn test_tree_depth_indentation() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        let snap = make_snapshot(vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(10, Some(1), "child"),
            make_proc_with_parent(100, Some(10), "grandchild"),
        ]);
        app.apply_snapshot(snap);
        app.tree_mode = true;
        app.recompute_visible();
        let buf = render_with(&app, 100, 24);
        // Grandchild at depth 2 should have "  └── " prefix (2-space indent + connector)
        assert!(buffer_contains(&buf, "grandchild"));
    }

    // AC-13: No panic on tiny terminal
    #[test]
    fn test_minimal_terminal_no_panic() {
        let app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        let _buf = render_with(&app, 40, 6);
    }

    #[test]
    fn test_extreme_terminal_1x1() {
        let app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        let _buf = render_with(&app, 1, 1);
    }

    // Edge cases
    #[test]
    fn test_empty_process_list() {
        let app = processes_app(vec![]);
        let buf = render_with(&app, 80, 24);
        // Should render header but no data rows — no panic
        assert!(buffer_contains(&buf, "PID"));
    }

    #[test]
    fn test_large_terminal() {
        let app = processes_app(vec![make_proc(1, "test", 10.0, 5.0)]);
        let _buf = render_with(&app, 200, 50);
    }

    #[test]
    fn test_many_processes_virtual_scroll() {
        let procs: Vec<ProcessInfo> = (0..500)
            .map(|i| make_proc(i, &format!("proc{i}"), 1.0, 0.1))
            .collect();
        let app = processes_app(procs);
        // Should not render all 500 rows — virtualized
        let buf = render_with(&app, 80, 24);
        // First few processes should be visible (after sort)
        assert!(buffer_contains(&buf, "PID"));
    }

    #[test]
    fn test_status_colors_in_row() {
        let mut proc = make_proc(1, "zombie_proc", 0.0, 0.0);
        proc.status = "Zombie".to_string();
        let app = processes_app(vec![proc]);
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "⚠") || buffer_contains(&buf, "Z"));
    }
}
