// General tab — CPU bars, memory bars, system info line.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::app::AppState;
use muxtop_core::system::{CoreSnapshot, SystemSnapshot};

/// Render the General tab content area.
pub fn draw_general_tab(frame: &mut Frame, area: Rect, app: &AppState) {
    let Some(ref snapshot) = app.last_snapshot else {
        let para = Paragraph::new("Waiting for data...").alignment(Alignment::Center);
        frame.render_widget(para, area);
        return;
    };

    let [cpu_area, mem_area, info_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(2),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_cpu_bars(frame, cpu_area, snapshot);
    draw_memory_bars(frame, mem_area, snapshot);
    draw_system_info(frame, info_area, snapshot);
}

/// Format uptime in seconds as "Xd Yh Zm".
fn format_uptime(secs: u64) -> String {
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    format!("{days}d {hours}h {mins}m")
}

/// Render a single-line system info bar: uptime, load averages, task counts.
fn draw_system_info(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot) {
    let running = snapshot
        .processes
        .iter()
        .filter(|p| p.status == "Running")
        .count();
    let total = snapshot.processes.len();
    let uptime = format_uptime(snapshot.load.uptime_secs);
    let text = format!(
        "Uptime: {uptime} | Load: {:.2} {:.2} {:.2} | Tasks: {total} ({running} running)",
        snapshot.load.one, snapshot.load.five, snapshot.load.fifteen,
    );
    let para = Paragraph::new(text).style(Style::default().fg(Color::Gray));
    frame.render_widget(para, area);
}

/// Convert bytes to GiB string with one decimal place.
fn format_bytes_gb(bytes: u64) -> String {
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    format!("{gib:.1}")
}

/// Render RAM bar and optional Swap bar.
fn draw_memory_bars(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot) {
    let mem = &snapshot.memory;
    let mut lines: Vec<Line<'static>> = Vec::new();

    lines.push(make_bar_line(
        "Mem",
        mem.used,
        mem.total,
        Color::Green,
        area.width,
    ));

    if mem.swap_total > 0 {
        lines.push(make_bar_line(
            "Swp",
            mem.swap_used,
            mem.swap_total,
            Color::Cyan,
            area.width,
        ));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}

/// Build a single horizontal bar line (used for both RAM and Swap).
fn make_bar_line(
    label: &str,
    used: u64,
    total: u64,
    fill_color: Color,
    width: u16,
) -> Line<'static> {
    let pct = if total > 0 {
        (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let used_gb = format_bytes_gb(used);
    let total_gb = format_bytes_gb(total);
    let info = format!("  {used_gb}/{total_gb} G  {pct:.1}%");

    let label_part = format!("{:<5}", label);
    let overhead = label_part.len() as u16 + 2 + info.len() as u16; // label + "[]" + info
    let bar_w = width.saturating_sub(overhead).max(1);

    let filled = ((bar_w as f64) * (pct / 100.0)).round() as u16;
    let empty = bar_w.saturating_sub(filled);

    Line::from(vec![
        Span::styled(
            label_part,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("["),
        Span::styled("█".repeat(filled as usize), Style::default().fg(fill_color)),
        Span::styled(
            "░".repeat(empty as usize),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("]"),
        Span::styled(info, Style::default().fg(Color::Gray)),
    ])
}

/// Render per-core CPU usage bars inside a "CPU" bordered block.
fn draw_cpu_bars(frame: &mut Frame, area: Rect, snapshot: &SystemSnapshot) {
    let block = Block::default().title("CPU").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cores = &snapshot.cpu.cores;
    if cores.is_empty() {
        return;
    }

    if cores.len() <= 16 {
        let lines: Vec<Line<'static>> = cores
            .iter()
            .map(|c| make_cpu_bar_line(c, inner.width))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    } else {
        let [left_area, right_area] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(inner);

        let mid = cores.len().div_ceil(2);
        let left_lines: Vec<Line<'static>> = cores[..mid]
            .iter()
            .map(|c| make_cpu_bar_line(c, left_area.width))
            .collect();
        let right_lines: Vec<Line<'static>> = cores[mid..]
            .iter()
            .map(|c| make_cpu_bar_line(c, right_area.width))
            .collect();

        frame.render_widget(Paragraph::new(left_lines), left_area);
        frame.render_widget(Paragraph::new(right_lines), right_area);
    }
}

/// Build a single CPU core bar line: "cpu0  [████░░░░]  10.0%".
fn make_cpu_bar_line(core: &CoreSnapshot, width: u16) -> Line<'static> {
    let pct = core.usage.clamp(0.0, 100.0);
    let label = format!("{:<6}", core.name);
    let info = format!("  {pct:.1}%");

    let overhead = label.len() as u16 + 2 + info.len() as u16; // label + "[]" + info
    let bar_w = width.saturating_sub(overhead).max(1);

    let filled = ((bar_w as f64) * (pct as f64 / 100.0)).round() as u16;
    let empty = bar_w.saturating_sub(filled);

    Line::from(vec![
        Span::styled(label, Style::default().fg(Color::White)),
        Span::raw("["),
        Span::styled(
            "█".repeat(filled as usize),
            Style::default().fg(Color::Green),
        ),
        Span::styled(
            "░".repeat(empty as usize),
            Style::default().fg(Color::DarkGray),
        ),
        Span::raw("]"),
        Span::styled(info, Style::default().fg(Color::Gray)),
    ])
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

    fn make_test_snapshot(
        core_count: usize,
        running_count: usize,
    ) -> muxtop_core::system::SystemSnapshot {
        use muxtop_core::process::ProcessInfo;
        use muxtop_core::system::*;
        use std::time::Instant;

        let cores = (0..core_count)
            .map(|i| CoreSnapshot {
                name: format!("cpu{i}"),
                usage: (i as f32 * 10.0) % 100.0,
                frequency: 3600,
            })
            .collect();

        let mut processes = Vec::new();
        for i in 0..5 {
            processes.push(ProcessInfo {
                pid: i as u32,
                parent_pid: None,
                name: format!("proc{i}"),
                command: format!("/usr/bin/proc{i}"),
                user: "user".to_string(),
                cpu_percent: 10.0,
                memory_bytes: 1000,
                memory_percent: 1.0,
                status: if i < running_count {
                    "Running".to_string()
                } else {
                    "Sleeping".to_string()
                },
            });
        }

        SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 42.5,
                cores,
            },
            memory: MemorySnapshot {
                total: 16_000_000_000,
                used: 8_000_000_000,
                available: 8_000_000_000,
                swap_total: 4_000_000_000,
                swap_used: 1_000_000_000,
            },
            load: LoadSnapshot {
                one: 2.31,
                five: 1.87,
                fifteen: 1.42,
                uptime_secs: 90061,
            },
            processes,
            timestamp: Instant::now(),
        }
    }

    // -- STORY-01: Scaffold --

    #[test]
    fn test_general_tab_callable() {
        let app = AppState::new();
        let buf = render_with(&app, 80, 24);
        assert!(!buffer_contains(&buf, "[General view"));
    }

    // -- STORY-02: 3-zone layout + no-data handling --

    #[test]
    fn test_general_no_data_shows_waiting() {
        let app = AppState::new();
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Waiting for data..."));
    }

    #[test]
    fn test_general_three_zones_with_data() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(!buffer_contains(&buf, "Waiting for data..."));
    }

    // -- STORY-05: System info line --

    #[test]
    fn test_format_uptime_zero() {
        assert_eq!(format_uptime(0), "0d 0h 0m");
    }

    #[test]
    fn test_format_uptime_complex() {
        assert_eq!(format_uptime(90061), "1d 1h 1m");
    }

    #[test]
    fn test_format_uptime_hours_only() {
        assert_eq!(format_uptime(7200), "0d 2h 0m");
    }

    #[test]
    fn test_system_info_tasks_count() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Tasks: 5 (2 running)"));
    }

    #[test]
    fn test_system_info_load_averages() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "2.31"));
        assert!(buffer_contains(&buf, "1.87"));
        assert!(buffer_contains(&buf, "1.42"));
    }

    #[test]
    fn test_system_info_pipe_separated() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "|"));
    }

    // -- STORY-04: Memory bars --

    #[test]
    fn test_format_bytes_gb() {
        let result = format_bytes_gb(16_000_000_000);
        assert!(
            result.contains('.'),
            "Should contain decimal point: {result}"
        );
        assert!(!result.is_empty());
    }

    #[test]
    fn test_format_bytes_gb_zero() {
        assert_eq!(format_bytes_gb(0), "0.0");
    }

    #[test]
    fn test_memory_bar_shows_values() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Mem"));
        assert!(buffer_contains(&buf, "%"));
    }

    #[test]
    fn test_memory_swap_shown_when_active() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "Swp"));
    }

    #[test]
    fn test_memory_swap_hidden_when_zero() {
        let mut app = AppState::new();
        let mut snap = make_test_snapshot(4, 2);
        snap.memory.swap_total = 0;
        snap.memory.swap_used = 0;
        app.apply_snapshot(snap);
        let buf = render_with(&app, 80, 24);
        assert!(!buffer_contains(&buf, "Swp"));
    }

    // -- STORY-03: CPU bars --

    #[test]
    fn test_cpu_bars_show_core_labels() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "cpu0"));
        assert!(buffer_contains(&buf, "cpu1"));
        assert!(buffer_contains(&buf, "cpu2"));
        assert!(buffer_contains(&buf, "cpu3"));
    }

    #[test]
    fn test_cpu_bars_show_percentages() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "%"));
    }

    #[test]
    fn test_cpu_bars_zero_cores_no_panic() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(0, 0));
        let _buf = render_with(&app, 80, 24);
    }

    #[test]
    fn test_cpu_bars_condense_20_cores() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(20, 0));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "cpu0"));
        assert!(buffer_contains(&buf, "cpu10"));
    }

    // Guard G-04: odd core count 2-column split
    #[test]
    fn test_cpu_bars_condense_odd_17_cores() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(17, 0));
        let buf = render_with(&app, 100, 30);
        // 17 cores: left column gets ceil(17/2)=9, right gets 8
        assert!(buffer_contains(&buf, "cpu0"));
        assert!(buffer_contains(&buf, "cpu8"));
        assert!(buffer_contains(&buf, "cpu16"));
    }

    #[test]
    fn test_cpu_bars_condense_odd_21_cores() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(21, 0));
        let buf = render_with(&app, 100, 30);
        // 21 cores: left column gets ceil(21/2)=11, right gets 10
        assert!(buffer_contains(&buf, "cpu0"));
        assert!(buffer_contains(&buf, "cpu10"));
        assert!(buffer_contains(&buf, "cpu20"));
    }

    // -- STORY-06: Integration + edge cases --

    #[test]
    fn test_general_full_render_80x24() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(buffer_contains(&buf, "CPU"));
        assert!(buffer_contains(&buf, "Mem"));
        assert!(buffer_contains(&buf, "Tasks"));
    }

    #[test]
    fn test_general_tiny_terminal_no_panic() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let _buf = render_with(&app, 40, 6);
    }

    #[test]
    fn test_general_large_terminal_no_panic() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let _buf = render_with(&app, 200, 50);
    }

    #[test]
    fn test_general_replaces_placeholder() {
        let mut app = AppState::new();
        app.apply_snapshot(make_test_snapshot(4, 2));
        let buf = render_with(&app, 80, 24);
        assert!(!buffer_contains(&buf, "[General view"));
    }
}
