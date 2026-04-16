// Network tab — interface table with bandwidth rates, sparklines, and summary bar.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Sparkline},
};

use super::theme::Theme;
use crate::app::{AppState, NetworkSortField};
use muxtop_core::network::NetworkInterfaceSnapshot;
use muxtop_core::process::SortOrder;

// Fixed column widths.
const COL_IFACE: usize = 14;
const COL_STATUS: usize = 5;
const COL_RX_RATE: usize = 12;
const COL_TX_RATE: usize = 12;
const COL_TOTAL_RX: usize = 10;
const COL_TOTAL_TX: usize = 10;
const COL_ERRORS: usize = 8;

/// Render the Network tab content area.
pub fn draw_network_tab(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
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

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Layout: summary(1) + table(fill) + sparklines(4 optional) + filter(0|1)
    let filter_h = u16::from(app.net_filter_active);
    let sparkline_h = if app.net_selected < app.net_interface_count() {
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

    draw_summary_bar(frame, areas[0], app, theme);

    if areas[1].height >= 2 {
        let [header_area, body_area] =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(areas[1]);
        draw_header(frame, header_area, app, theme);
        draw_body(frame, body_area, app, theme);
    }

    if sparkline_h > 0 && areas.len() > 2 {
        draw_sparklines(frame, areas[2], app, theme);
    }

    if app.net_filter_active {
        let filter_area = areas[areas.len() - 1];
        draw_filter_bar(frame, filter_area, app, theme);
    }
}

/// Summary bar: total bandwidth and interface count.
fn draw_summary_bar(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let snapshot = match &app.last_snapshot {
        Some(s) => s,
        None => return,
    };

    let active = snapshot
        .networks
        .interfaces
        .iter()
        .filter(|i| i.is_up)
        .count();
    let total = snapshot.networks.interfaces.len();

    // Compute total bandwidth from history
    let mut total_rx_rate = 0.0_f64;
    let mut total_tx_rate = 0.0_f64;
    for iface in &snapshot.networks.interfaces {
        total_rx_rate += app.network_history.bandwidth_rx(&iface.name);
        total_tx_rate += app.network_history.bandwidth_tx(&iface.name);
    }

    let line = Line::from(vec![
        Span::styled(
            " Total: ",
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} {}/s", DOWN_ARROW, format_rate(total_rx_rate as u64)),
            Style::default().fg(theme.success),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{} {}/s", UP_ARROW, format_rate(total_tx_rate as u64)),
            Style::default().fg(theme.warning),
        ),
        Span::raw("  "),
        Span::styled(
            format!("| Interfaces: {active} active / {total} total"),
            Style::default().fg(theme.text_dim),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

const DOWN_ARROW: &str = "\u{2193}";
const UP_ARROW: &str = "\u{2191}";

/// Render the column header row with sort indicator.
fn draw_header(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let arrow = if matches!(app.net_sort_order, SortOrder::Desc) {
        "\u{25bc}"
    } else {
        "\u{25b2}"
    };
    let style = Style::default()
        .fg(theme.accent_primary)
        .bg(theme.header_bg)
        .add_modifier(Modifier::BOLD);

    let header = format!(
        "{}{}{}{}{}{}{}",
        col_text(
            &sort_label(
                "INTERFACE",
                NetworkSortField::Name,
                app.net_sort_field,
                arrow
            ),
            COL_IFACE,
        ),
        col_text("STATE", COL_STATUS),
        col_text(
            &sort_label("RX/s", NetworkSortField::RxRate, app.net_sort_field, arrow),
            COL_RX_RATE,
        ),
        col_text(
            &sort_label("TX/s", NetworkSortField::TxRate, app.net_sort_field, arrow),
            COL_TX_RATE,
        ),
        col_text(
            &sort_label(
                "TOTAL RX",
                NetworkSortField::TotalRx,
                app.net_sort_field,
                arrow
            ),
            COL_TOTAL_RX,
        ),
        col_text(
            &sort_label(
                "TOTAL TX",
                NetworkSortField::TotalTx,
                app.net_sort_field,
                arrow
            ),
            COL_TOTAL_TX,
        ),
        col_text(
            &sort_label(
                "ERRORS",
                NetworkSortField::Errors,
                app.net_sort_field,
                arrow
            ),
            COL_ERRORS,
        ),
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(header, style))),
        area,
    );
}

/// Render the interface rows with virtualized scrolling.
fn draw_body(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let vis_h = area.height as usize;
    if vis_h == 0 {
        return;
    }

    let interfaces = sorted_filtered_interfaces(app);
    if interfaces.is_empty() {
        let msg = if app.net_filter_input.is_empty() {
            "No network interfaces found"
        } else {
            "No interfaces match filter"
        };
        let para = Paragraph::new(msg)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim));
        frame.render_widget(para, area);
        return;
    }

    let scroll = effective_scroll(app.net_selected, app.net_scroll_offset, vis_h);
    let end = (scroll + vis_h).min(interfaces.len());

    let lines: Vec<Line<'static>> = (scroll..end)
        .map(|i| {
            let iface = &interfaces[i];
            let rx_rate = app.network_history.bandwidth_rx(&iface.name);
            let tx_rate = app.network_history.bandwidth_tx(&iface.name);
            interface_row(iface, rx_rate, tx_rate, i == app.net_selected, theme, i)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), area);
}

/// Format a single interface row with fixed-width columns.
fn interface_row(
    iface: &NetworkInterfaceSnapshot,
    rx_rate: f64,
    tx_rate: f64,
    selected: bool,
    theme: &Theme,
    row_idx: usize,
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
    } else if iface.is_up {
        theme.fg
    } else {
        theme.text_dim
    };
    let base = if selected {
        Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(bg).fg(fg)
    };

    let status_str = if iface.is_up { "UP" } else { "DOWN" };
    let status_style = if selected {
        base
    } else if iface.is_up {
        Style::default().fg(theme.success).bg(bg)
    } else {
        Style::default().fg(theme.text_dim).bg(bg)
    };

    let rx_style = if selected {
        base
    } else {
        Style::default().fg(theme.success).bg(bg)
    };
    let tx_style = if selected {
        base
    } else {
        Style::default().fg(theme.warning).bg(bg)
    };

    let total_errors = iface.errors_rx + iface.errors_tx;
    let err_style = if selected {
        base
    } else if total_errors > 0 {
        Style::default().fg(theme.danger).bg(bg)
    } else {
        base
    };

    Line::from(vec![
        Span::styled(col_text(&iface.name, COL_IFACE), base),
        Span::styled(col_text(status_str, COL_STATUS), status_style),
        Span::styled(
            col_text(&format!("{}/s", format_rate(rx_rate as u64)), COL_RX_RATE),
            rx_style,
        ),
        Span::styled(
            col_text(&format!("{}/s", format_rate(tx_rate as u64)), COL_TX_RATE),
            tx_style,
        ),
        Span::styled(col_text(&format_bytes(iface.bytes_rx), COL_TOTAL_RX), base),
        Span::styled(col_text(&format_bytes(iface.bytes_tx), COL_TOTAL_TX), base),
        Span::styled(col_text(&total_errors.to_string(), COL_ERRORS), err_style),
    ])
}

/// Draw RX/TX sparklines for the selected interface.
fn draw_sparklines(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let interfaces = sorted_filtered_interfaces(app);
    let selected_iface = match interfaces.get(app.net_selected) {
        Some(iface) => &iface.name,
        None => return,
    };

    let rx_rate = app.network_history.bandwidth_rx(selected_iface);
    let tx_rate = app.network_history.bandwidth_tx(selected_iface);

    let points = area.width as usize;
    let rx_data = app.network_history.sparkline_rx(selected_iface, points);
    let tx_data = app.network_history.sparkline_tx(selected_iface, points);

    let [rx_area, tx_area] =
        Layout::vertical([Constraint::Length(2), Constraint::Length(2)]).areas(area);

    // RX sparkline
    let rx_label = format!("{} RX {}/s", DOWN_ARROW, format_rate(rx_rate as u64));
    let rx_sparkline = Sparkline::default()
        .data(&rx_data)
        .style(Style::default().fg(theme.accent_primary))
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

    // TX sparkline
    let tx_label = format!("{} TX {}/s", UP_ARROW, format_rate(tx_rate as u64));
    let tx_sparkline = Sparkline::default()
        .data(&tx_data)
        .style(Style::default().fg(theme.accent_secondary))
        .block(
            Block::default()
                .title(Span::styled(
                    tx_label,
                    Style::default()
                        .fg(theme.warning)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::NONE),
        );
    frame.render_widget(tx_sparkline, tx_area);
}

/// Render the filter input bar.
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
            format!("{}{cursor}", app.net_filter_input),
            Style::default().fg(theme.fg),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get sorted and filtered interfaces from the current snapshot.
fn sorted_filtered_interfaces(app: &AppState) -> Vec<NetworkInterfaceSnapshot> {
    let Some(ref snapshot) = app.last_snapshot else {
        return Vec::new();
    };

    let mut interfaces: Vec<NetworkInterfaceSnapshot> = if app.net_filter_input.is_empty() {
        snapshot.networks.interfaces.clone()
    } else {
        let filter = app.net_filter_input.to_lowercase();
        snapshot
            .networks
            .interfaces
            .iter()
            .filter(|i| i.name.to_lowercase().contains(&filter))
            .cloned()
            .collect()
    };

    let history = &app.network_history;
    match app.net_sort_field {
        NetworkSortField::Name => {
            interfaces.sort_by(|a, b| a.name.cmp(&b.name));
        }
        NetworkSortField::RxRate => {
            interfaces.sort_by(|a, b| {
                let ra = history.bandwidth_rx(&a.name);
                let rb = history.bandwidth_rx(&b.name);
                rb.partial_cmp(&ra).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        NetworkSortField::TxRate => {
            interfaces.sort_by(|a, b| {
                let ta = history.bandwidth_tx(&a.name);
                let tb = history.bandwidth_tx(&b.name);
                tb.partial_cmp(&ta).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        NetworkSortField::TotalRx => {
            interfaces.sort_by_key(|b| std::cmp::Reverse(b.bytes_rx));
        }
        NetworkSortField::TotalTx => {
            interfaces.sort_by_key(|b| std::cmp::Reverse(b.bytes_tx));
        }
        NetworkSortField::Errors => {
            interfaces.sort_by(|a, b| {
                let ea = a.errors_rx + a.errors_tx;
                let eb = b.errors_rx + b.errors_tx;
                eb.cmp(&ea)
            });
        }
    }

    // Default sort is descending for numeric, ascending for Name.
    // Reverse when user asks for the opposite direction.
    let is_asc = matches!(app.net_sort_order, SortOrder::Asc);
    let is_name = matches!(app.net_sort_field, NetworkSortField::Name);
    if is_asc != is_name {
        interfaces.reverse();
    }

    interfaces
}

/// Build the column header label, appending a sort arrow when this column is active.
fn sort_label(
    name: &str,
    field: NetworkSortField,
    active: NetworkSortField,
    arrow: &str,
) -> String {
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

/// Format bytes/s as human-readable rate (B, KB, MB, GB).
fn format_rate(bytes_per_sec: u64) -> String {
    let rate = bytes_per_sec as f64;
    if rate < 1024.0 {
        format!("{:.0}B", rate)
    } else if rate < 1024.0 * 1024.0 {
        format!("{:.1}KB", rate / 1024.0)
    } else if rate < 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1}MB", rate / (1024.0 * 1024.0))
    } else {
        format!("{:.1}GB", rate / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Format total bytes as human-readable.
fn format_bytes(bytes: u64) -> String {
    let b = bytes as f64;
    if b < 1024.0 {
        format!("{:.0}B", b)
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
    fn test_format_rate_bytes() {
        assert_eq!(format_rate(0), "0B");
        assert_eq!(format_rate(512), "512B");
        assert_eq!(format_rate(1023), "1023B");
    }

    #[test]
    fn test_format_rate_kilobytes() {
        assert_eq!(format_rate(1024), "1.0KB");
        assert_eq!(format_rate(1536), "1.5KB");
    }

    #[test]
    fn test_format_rate_megabytes() {
        assert_eq!(format_rate(1024 * 1024), "1.0MB");
        assert_eq!(format_rate(10 * 1024 * 1024), "10.0MB");
    }

    #[test]
    fn test_format_rate_gigabytes() {
        assert_eq!(format_rate(1024 * 1024 * 1024), "1.0GB");
    }

    #[test]
    fn test_format_bytes_total() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(500), "500B");
        assert_eq!(format_bytes(1024), "1.0KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0GB");
        assert_eq!(format_bytes(1024u64 * 1024 * 1024 * 1024), "1.0TB");
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
    fn test_col_text_zero() {
        assert_eq!(col_text("hello", 0), "");
    }

    #[test]
    fn test_effective_scroll_selected_above() {
        assert_eq!(effective_scroll(2, 5, 10), 2);
    }

    #[test]
    fn test_effective_scroll_selected_below() {
        assert_eq!(effective_scroll(15, 0, 10), 6);
    }

    #[test]
    fn test_effective_scroll_selected_visible() {
        assert_eq!(effective_scroll(5, 3, 10), 3);
    }

    #[test]
    fn test_effective_scroll_zero_height() {
        assert_eq!(effective_scroll(5, 3, 0), 0);
    }

    #[test]
    fn test_sort_label_active() {
        let label = sort_label(
            "RX/s",
            NetworkSortField::RxRate,
            NetworkSortField::RxRate,
            "▼",
        );
        assert_eq!(label, "RX/s▼");
    }

    #[test]
    fn test_sort_label_inactive() {
        let label = sort_label(
            "TX/s",
            NetworkSortField::TxRate,
            NetworkSortField::RxRate,
            "▼",
        );
        assert_eq!(label, "TX/s");
    }
}
