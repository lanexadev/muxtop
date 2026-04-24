// Containers tab — Docker/Podman container table with stats, sparklines, and no-daemon fallback.
//
// Mirrors the Network tab structure: summary bar + column header + table body
// + per-row color/state + optional sparkline panel + filter bar. The data
// source is `AppState::last_snapshot.containers` populated by the Collector
// in E4.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
};

use super::theme::Theme;
use crate::app::{AppState, ContainerSortField};
use muxtop_core::containers::{ContainerSnapshot, ContainerState, ContainersSnapshot, EngineKind};
use muxtop_core::process::SortOrder;

// Fixed column widths (sum < 120 to leave breathing room on narrow terminals).
const COL_NAME: usize = 20;
const COL_IMAGE: usize = 30;
const COL_STATE: usize = 12;
const COL_CPU: usize = 7;
const COL_MEM: usize = 18; // "1234MB/5678MB"
const COL_NET_RX: usize = 10;
const COL_NET_TX: usize = 10;
const COL_UPTIME: usize = 10;

const DOWN_ARROW: &str = "\u{2193}";
const UP_ARROW: &str = "\u{2191}";
const ARROW_DESC: &str = "\u{25bc}";
const ARROW_ASC: &str = "\u{25b2}";

/// Render the Containers tab content area.
pub fn draw_containers_tab(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    // No snapshot at all: still booting.
    let snapshot = match &app.last_snapshot {
        Some(s) => s,
        None => {
            let para = Paragraph::new("Waiting for data...").alignment(Alignment::Center);
            frame.render_widget(para, area);
            return;
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(theme.text_dim));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Three states for the container slot:
    //   - None: collector has no engine attached yet (CLI didn't wire one).
    //   - Some(unavailable): engine failed, show "no daemon" message.
    //   - Some(ok): render table.
    let containers = match snapshot.containers.as_ref() {
        None => {
            draw_no_engine(frame, inner, theme);
            return;
        }
        Some(cs) if !cs.daemon_up => {
            draw_no_daemon(frame, inner, theme);
            return;
        }
        Some(cs) => cs,
    };

    // Layout: summary(1) + table(fill) + sparklines(4 optional) + filter(0|1).
    let filter_h = u16::from(app.containers_filter_active);
    let sparkline_h = if app.containers_selected < visible_count(app, containers) {
        4
    } else {
        0
    };
    let constraints = if sparkline_h > 0 {
        vec![
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(sparkline_h),
            Constraint::Length(filter_h),
        ]
    } else {
        vec![
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(filter_h),
        ]
    };
    let areas = Layout::vertical(constraints).split(inner);

    draw_summary_bar(frame, areas[0], containers, theme);

    let now_ms = snapshot.timestamp_ms;
    if areas[1].height >= 2 {
        let [header_area, body_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(areas[1]);
        draw_header(frame, header_area, app, theme);
        draw_body(frame, body_area, app, containers, theme, now_ms);
    }

    if sparkline_h > 0 && areas.len() > 2 {
        draw_sparklines(frame, areas[2], app, containers, theme);
    }

    if app.containers_filter_active {
        let filter_area = areas[areas.len() - 1];
        draw_filter_bar(frame, filter_area, app, theme);
    }
}

/// Empty-engine placeholder — the Collector is running without a container
/// engine (CLI didn't pass `--docker-socket` or autodetection failed).
fn draw_no_engine(frame: &mut Frame, area: Rect, theme: &Theme) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "No container engine configured",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Start Docker or Podman, then relaunch muxtop.",
            Style::default().fg(theme.text_dim),
        )),
    ];
    let para = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

/// Daemon unreachable — engine wired but `/info` failed.
fn draw_no_daemon(frame: &mut Frame, area: Rect, theme: &Theme) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "No container daemon detected",
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Run `docker` or `podman system service` on the host.",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Check that your user is in the `docker` group.",
            Style::default().fg(theme.text_dim),
        )),
    ];
    let para = Paragraph::new(lines).alignment(Alignment::Center);
    frame.render_widget(para, area);
}

/// Summary bar: engine kind, running/total count.
fn draw_summary_bar(frame: &mut Frame, area: Rect, containers: &ContainersSnapshot, theme: &Theme) {
    let total = containers.containers.len();
    let running = containers
        .containers
        .iter()
        .filter(|c| c.state == ContainerState::Running)
        .count();
    let engine_label = match containers.engine {
        EngineKind::Docker => "Docker",
        EngineKind::Podman => "Podman",
        EngineKind::Unknown => "Engine",
    };

    let line = Line::from(vec![
        Span::styled(
            format!(" {engine_label} "),
            Style::default()
                .bg(theme.accent_primary)
                .fg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("Containers: {running} running / {total} total"),
            Style::default().fg(theme.text_dim),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Column header row with active-sort arrow.
fn draw_header(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let arrow = if matches!(app.containers_sort_order, SortOrder::Desc) {
        ARROW_DESC
    } else {
        ARROW_ASC
    };
    let style = Style::default()
        .fg(theme.accent_primary)
        .bg(theme.header_bg)
        .add_modifier(Modifier::BOLD);

    let header = format!(
        "{}{}{}{}{}{}{}{}",
        col_text(
            &sort_label(
                "NAME",
                ContainerSortField::Name,
                app.containers_sort_field,
                arrow
            ),
            COL_NAME,
        ),
        col_text("IMAGE", COL_IMAGE),
        col_text("STATE", COL_STATE),
        col_text(
            &sort_label(
                "CPU%",
                ContainerSortField::Cpu,
                app.containers_sort_field,
                arrow
            ),
            COL_CPU,
        ),
        col_text(
            &sort_label(
                "MEM",
                ContainerSortField::Mem,
                app.containers_sort_field,
                arrow
            ),
            COL_MEM,
        ),
        col_text(
            &sort_label(
                "NET RX",
                ContainerSortField::NetRx,
                app.containers_sort_field,
                arrow
            ),
            COL_NET_RX,
        ),
        col_text(
            &sort_label(
                "NET TX",
                ContainerSortField::NetTx,
                app.containers_sort_field,
                arrow
            ),
            COL_NET_TX,
        ),
        col_text(
            &sort_label(
                "UPTIME",
                ContainerSortField::Uptime,
                app.containers_sort_field,
                arrow
            ),
            COL_UPTIME,
        ),
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(header, style))),
        area,
    );
}

/// Render rows with virtualized scroll + filter.
fn draw_body(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    containers: &ContainersSnapshot,
    theme: &Theme,
    now_ms: u64,
) {
    let vis_h = area.height as usize;
    if vis_h == 0 {
        return;
    }

    let rows = sorted_filtered_containers(app, containers);
    if rows.is_empty() {
        let msg = if app.containers_filter_input.is_empty() {
            "No containers"
        } else {
            "No containers match filter"
        };
        let para = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim));
        frame.render_widget(para, area);
        return;
    }

    let scroll = effective_scroll(app.containers_selected, app.containers_scroll_offset, vis_h);
    let end = (scroll + vis_h).min(rows.len());

    let lines: Vec<Line<'static>> = (scroll..end)
        .map(|i| container_row(&rows[i], i == app.containers_selected, theme, i, now_ms))
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

/// Render a single container row.
fn container_row(
    c: &ContainerSnapshot,
    selected: bool,
    theme: &Theme,
    row_idx: usize,
    now_ms: u64,
) -> Line<'static> {
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

    let state_str = state_label(c.state);
    let state_style = if selected {
        base
    } else {
        Style::default().fg(state_color(c.state, theme)).bg(bg)
    };

    let cpu_str = format!("{:>5.1}%", c.cpu_pct);
    let mem_str = format!(
        "{}/{}",
        format_bytes(c.mem_used_bytes),
        format_bytes(c.mem_limit_bytes),
    );

    Line::from(vec![
        Span::styled(col_text(&c.name, COL_NAME), base),
        Span::styled(col_text(&truncate_image(&c.image), COL_IMAGE), base),
        Span::styled(col_text(state_str, COL_STATE), state_style),
        Span::styled(col_text(&cpu_str, COL_CPU), base),
        Span::styled(col_text(&mem_str, COL_MEM), base),
        Span::styled(col_text(&format_bytes(c.net_rx_bytes), COL_NET_RX), base),
        Span::styled(col_text(&format_bytes(c.net_tx_bytes), COL_NET_TX), base),
        Span::styled(
            col_text(&format_uptime(now_ms, c.started_at_ms), COL_UPTIME),
            base,
        ),
    ])
}

/// CPU + RX sparklines for the selected container.
fn draw_sparklines(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    containers: &ContainersSnapshot,
    theme: &Theme,
) {
    let rows = sorted_filtered_containers(app, containers);
    let selected = match rows.get(app.containers_selected) {
        Some(c) => c,
        None => return,
    };

    let points = area.width as usize;
    let cpu_data: Vec<u64> = app
        .container_cpu_history(&selected.id)
        .iter()
        .rev()
        .take(points)
        .map(|v| (*v * 10.0).round() as u64)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let rx_data: Vec<u64> = app
        .container_rx_deltas(&selected.id)
        .iter()
        .rev()
        .take(points)
        .copied()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();

    let [cpu_area, rx_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(2)]).areas(area);

    let cpu_label = format!("{} CPU {:.1}%", UP_ARROW, selected.cpu_pct);
    let cpu_sparkline = Sparkline::default()
        .data(&cpu_data)
        .style(Style::default().fg(theme.accent_primary))
        .block(
            Block::default()
                .title(Span::styled(
                    cpu_label,
                    Style::default()
                        .fg(theme.accent_primary)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::NONE),
        );
    frame.render_widget(cpu_sparkline, cpu_area);

    let latest_rx = rx_data.last().copied().unwrap_or(0);
    let rx_label = format!("{} RX {}/tick", DOWN_ARROW, format_bytes(latest_rx));
    let rx_sparkline = Sparkline::default()
        .data(&rx_data)
        .style(Style::default().fg(theme.accent_secondary))
        .block(
            Block::default()
                .title(Span::styled(
                    rx_label,
                    Style::default()
                        .fg(theme.success)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::NONE),
        );
    frame.render_widget(rx_sparkline, rx_area);
}

/// Filter input line (tab-owned so it appears only when filter is active).
fn draw_filter_bar(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let cursor = if app.term_caps.unicode {
        "\u{2588}"
    } else {
        "_"
    };
    let line = Line::from(vec![
        Span::styled(
            "Filter: ",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}{cursor}", app.containers_filter_input),
            Style::default().fg(theme.fg),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Number of rows that would be rendered given current filter.
fn visible_count(app: &AppState, containers: &ContainersSnapshot) -> usize {
    if app.containers_filter_input.is_empty() {
        containers.containers.len()
    } else {
        let f = app.containers_filter_input.to_lowercase();
        containers
            .containers
            .iter()
            .filter(|c| row_matches_filter(c, &f))
            .count()
    }
}

fn row_matches_filter(c: &ContainerSnapshot, needle: &str) -> bool {
    c.name.to_lowercase().contains(needle)
        || c.image.to_lowercase().contains(needle)
        || c.id.to_lowercase().contains(needle)
}

/// Apply current sort + filter to produce the visible rows.
fn sorted_filtered_containers(
    app: &AppState,
    containers: &ContainersSnapshot,
) -> Vec<ContainerSnapshot> {
    let mut rows: Vec<ContainerSnapshot> = if app.containers_filter_input.is_empty() {
        containers.containers.clone()
    } else {
        let f = app.containers_filter_input.to_lowercase();
        containers
            .containers
            .iter()
            .filter(|c| row_matches_filter(c, &f))
            .cloned()
            .collect()
    };

    match app.containers_sort_field {
        ContainerSortField::Name => rows.sort_by(|a, b| a.name.cmp(&b.name)),
        ContainerSortField::Cpu => rows.sort_by(|a, b| {
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        ContainerSortField::Mem => rows.sort_by_key(|c| std::cmp::Reverse(c.mem_used_bytes)),
        ContainerSortField::NetRx => rows.sort_by_key(|c| std::cmp::Reverse(c.net_rx_bytes)),
        ContainerSortField::NetTx => rows.sort_by_key(|c| std::cmp::Reverse(c.net_tx_bytes)),
        ContainerSortField::Uptime => {
            // Older = bigger uptime = smaller started_at_ms.
            rows.sort_by_key(|c| c.started_at_ms);
        }
    }

    let is_asc = matches!(app.containers_sort_order, SortOrder::Asc);
    let is_name = matches!(app.containers_sort_field, ContainerSortField::Name);
    let is_uptime = matches!(app.containers_sort_field, ContainerSortField::Uptime);
    // Defaults: Name ascending, Uptime ascending (oldest first), everything else descending.
    let default_asc = is_name || is_uptime;
    if is_asc != default_asc {
        rows.reverse();
    }

    rows
}

fn sort_label(
    name: &str,
    field: ContainerSortField,
    active: ContainerSortField,
    arrow: &str,
) -> String {
    if std::mem::discriminant(&field) == std::mem::discriminant(&active) {
        format!("{name}{arrow}")
    } else {
        name.to_string()
    }
}

fn col_text(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let truncated: String = s.chars().take(width).collect();
    format!("{truncated:<width$}")
}

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

fn truncate_image(image: &str) -> String {
    const MAX: usize = 30;
    if image.chars().count() <= MAX {
        image.to_string()
    } else {
        let keep = image.chars().take(MAX - 1).collect::<String>();
        format!("{keep}…")
    }
}

fn state_label(state: ContainerState) -> &'static str {
    match state {
        ContainerState::Created => "created",
        ContainerState::Running => "running",
        ContainerState::Paused => "paused",
        ContainerState::Restarting => "restarting",
        ContainerState::Exited => "exited",
        ContainerState::Dead => "dead",
        ContainerState::Removing => "removing",
    }
}

fn state_color(state: ContainerState, theme: &Theme) -> ratatui::style::Color {
    match state {
        ContainerState::Running => theme.success,
        ContainerState::Paused | ContainerState::Restarting | ContainerState::Removing => {
            theme.warning
        }
        ContainerState::Dead => theme.danger,
        ContainerState::Exited | ContainerState::Created => theme.text_dim,
    }
}

/// Human-readable uptime from now_ms - started_at_ms (or `—` when started_at_ms is 0).
fn format_uptime(now_ms: u64, started_at_ms: u64) -> String {
    if started_at_ms == 0 || started_at_ms > now_ms {
        return "—".into();
    }
    let secs = (now_ms - started_at_ms) / 1_000;
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{b:.0}B")
    } else if b < 1024.0 * 1024.0 {
        format!("{:.1}KB", b / 1024.0)
    } else if b < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1}MB", b / (1024.0 * 1024.0))
    } else if b < 1024.0 * 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1}GB", b / (1024.0 * 1024.0 * 1024.0))
    } else {
        format!("{:.1}TB", b / (1024.0 * 1024.0 * 1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_col_text_pad_and_truncate() {
        assert_eq!(col_text("hi", 5), "hi   ");
        assert_eq!(col_text("hello world", 5), "hello");
        assert_eq!(col_text("x", 0), "");
    }

    #[test]
    fn test_truncate_image_short_passthrough() {
        assert_eq!(truncate_image("nginx:latest"), "nginx:latest");
    }

    #[test]
    fn test_truncate_image_long_marked() {
        let long = "registry.example.com/org/very-long-image-name:v1.2.3-rc1";
        let got = truncate_image(long);
        assert_eq!(got.chars().count(), 30);
        assert!(got.ends_with('…'));
    }

    #[test]
    fn test_effective_scroll_above_below_visible() {
        assert_eq!(effective_scroll(2, 5, 10), 2);
        assert_eq!(effective_scroll(15, 0, 10), 6);
        assert_eq!(effective_scroll(5, 3, 10), 3);
        assert_eq!(effective_scroll(5, 3, 0), 0);
    }

    #[test]
    fn test_format_bytes_scales() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(1023), "1023B");
        assert_eq!(format_bytes(1024), "1.0KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0MB");
        assert_eq!(format_bytes(1024u64.pow(3)), "1.0GB");
    }

    #[test]
    fn test_format_uptime_bucket() {
        let now = 10_000_000_000_u64;
        assert_eq!(format_uptime(now, now - 30_000), "30s");
        assert_eq!(format_uptime(now, now - 5 * 60 * 1000), "5m");
        assert_eq!(format_uptime(now, now - 3 * 3600 * 1000), "3h");
        assert_eq!(format_uptime(now, now - 2 * 86_400 * 1000), "2d");
        assert_eq!(format_uptime(now, 0), "—");
        assert_eq!(format_uptime(now, now + 1_000), "—");
    }

    #[test]
    fn test_state_label_and_color_exhaustive() {
        // Match without wildcard — compiler ensures every variant is handled
        // if ContainerState ever gains a case.
        for state in [
            ContainerState::Created,
            ContainerState::Running,
            ContainerState::Paused,
            ContainerState::Restarting,
            ContainerState::Exited,
            ContainerState::Dead,
            ContainerState::Removing,
        ] {
            let _ = state_label(state);
            // state_color takes a Theme; pick any valid palette.
            let theme = Theme::new(crate::terminal::ColorSupport::TrueColor);
            let _ = state_color(state, &theme);
        }
    }

    #[test]
    fn test_sort_label_active_gets_arrow() {
        let label = sort_label(
            "CPU%",
            ContainerSortField::Cpu,
            ContainerSortField::Cpu,
            "▼",
        );
        assert_eq!(label, "CPU%▼");
    }

    #[test]
    fn test_sort_label_inactive_no_arrow() {
        let label = sort_label(
            "CPU%",
            ContainerSortField::Cpu,
            ContainerSortField::Mem,
            "▼",
        );
        assert_eq!(label, "CPU%");
    }

    fn sample_container(id: &str, name: &str, cpu: f32, mem: u64) -> ContainerSnapshot {
        ContainerSnapshot {
            id: id.into(),
            name: name.into(),
            image: "nginx:1.27".into(),
            state: ContainerState::Running,
            status_text: "Up 1 minute".into(),
            cpu_pct: cpu,
            mem_used_bytes: mem,
            mem_limit_bytes: 1024 * 1024 * 1024,
            net_rx_bytes: 0,
            net_tx_bytes: 0,
            block_read_bytes: 0,
            block_write_bytes: 0,
            started_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn test_row_matches_filter_covers_name_image_id() {
        let c = sample_container("abc123", "my-web", 0.0, 0);
        assert!(row_matches_filter(&c, "my-web"));
        assert!(row_matches_filter(&c, "nginx"));
        assert!(row_matches_filter(&c, "abc"));
        assert!(!row_matches_filter(&c, "postgres"));
    }
}
