//! GPU tab UI (Alt+6) — v0.5.0.
//!
//! Renders one of two sub-views (Devices / Procs) selected by the `D` / `P`
//! keys, from the latest `GpusSnapshot` carried by the `SystemSnapshot`. The
//! vendor polling loops live in `muxtop-core`; this module just walks vecs.
//!
//! ## Sort + filter
//!
//! Applied at render time via local helpers, exactly as the Kube tab does.
//! A host has single-digit GPUs and rarely more than a few dozen GPU
//! processes, so the per-frame O(n log n) is microsecond-level.
//!
//! ## `—` means unknown, not zero
//!
//! Every metric in `GpuDeviceSnapshot` is `Option`, and this module renders
//! `None` as `—`. That distinction is the whole point of the data model: an
//! idle GPU reports `0 %`, a GPU whose driver cannot report utilisation shows
//! `—`, and conflating them would make the tab lie. See the `muxtop-core::gpu`
//! module doc.
//!
//! ## Sanitisation
//!
//! Device names come from the driver, process names from whatever the user
//! chose to call their binary, and `detail` crosses the wire from the server
//! in `--remote` mode. All three are foreign strings and go through
//! `scrub_ctrl` before reaching a cell — the v0.4.1 lesson was that a render
//! site missed by the sanitizer sweep is a terminal-escape injection point.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use muxtop_core::gpu::{GpuDeviceSnapshot, GpuProcessSnapshot, GpusSnapshot};
use muxtop_core::process::SortOrder;

use crate::app::{AppState, GpuSortField, GpuSubview};
use crate::ui::sanitize::scrub_ctrl;
use crate::ui::theme::Theme;

pub fn draw_gpu_tab(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let snap = app.last_snapshot.as_ref().and_then(|s| s.gpu.as_ref());
    match snap {
        None => draw_waiting(frame, area, theme),
        Some(s) if !s.available => draw_unavailable(frame, area, theme, s),
        Some(s) => draw_active(frame, area, app, theme, s),
    }
}

fn draw_waiting(frame: &mut Frame, area: Rect, theme: &Theme) {
    let line = Line::from(vec![Span::styled(
        " Waiting for GPU data… ",
        Style::default().fg(theme.fg).bg(theme.header_bg),
    )]);
    frame.render_widget(Paragraph::new(line), area);
}

/// The "no GPU" state, carrying the engine's explanation.
///
/// A bare "no GPU" is useless on a machine that *has* one — the interesting
/// cases are a driver that failed to load, `--no-gpu`, and macOS pending the
/// v0.6 backend. `GpusSnapshot::detail` carries which, so it is rendered
/// rather than discarded.
fn draw_unavailable(frame: &mut Frame, area: Rect, theme: &Theme, snap: &GpusSnapshot) {
    let mut lines = vec![
        Line::from(Span::styled(
            "  No GPU detected",
            Style::default()
                .fg(theme.warning)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if snap.detail.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No NVIDIA or AMD GPU could be queried on this host.",
            Style::default().fg(theme.text_dim),
        )));
    } else {
        // Wraps rather than truncates: the reason is frequently a sentence,
        // and a clipped explanation is no better than none.
        for chunk in wrap_detail(
            &scrub_ctrl(&snap.detail),
            area.width.saturating_sub(4) as usize,
        ) {
            lines.push(Line::from(Span::styled(
                format!("  {chunk}"),
                Style::default().fg(theme.text_dim),
            )));
        }
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Greedy word wrap. `ratatui`'s `Wrap` would do this, but only for a
/// `Paragraph` built from a single string — we interleave styled lines.
fn wrap_detail(text: &str, width: usize) -> Vec<String> {
    if width < 8 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Active path: summary line + sub-tab bar + (optional) filter line + table.
fn draw_active(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme, snap: &GpusSnapshot) {
    let show_filter = app.gpu_filter_active || !app.gpu_filter_input.is_empty();
    let mut constraints = vec![
        Constraint::Length(1), // summary
        Constraint::Length(1), // sub-tab bar
    ];
    if show_filter {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(1));

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    draw_summary(frame, chunks[0], theme, snap);
    draw_subtab_bar(frame, chunks[1], theme, app, snap);

    let table_idx = if show_filter {
        draw_filter_bar(frame, chunks[2], theme, app);
        3
    } else {
        2
    };

    match app.gpu_subview {
        GpuSubview::Devices => draw_devices(frame, chunks[table_idx], app, theme, snap),
        GpuSubview::Procs => draw_procs(frame, chunks[table_idx], app, theme, snap),
    }
}

fn draw_summary(frame: &mut Frame, area: Rect, theme: &Theme, snap: &GpusSnapshot) {
    let backends = snap
        .backends
        .iter()
        .map(|b| b.label())
        .collect::<Vec<_>>()
        .join("+");

    let mut spans = vec![
        Span::styled(
            format!(" GPUs: {}", snap.devices.len()),
            Style::default().fg(theme.accent_primary),
        ),
        Span::raw("  "),
        Span::raw(format!("backend: {backends}  ")),
    ];

    if let Some((used, total)) = snap.total_memory() {
        let pct = if total > 0 {
            used as f64 / total as f64 * 100.0
        } else {
            0.0
        };
        spans.push(Span::raw(format!(
            "vram: {} / {}  ",
            format_bytes(used),
            format_bytes(total)
        )));
        spans.push(Span::styled(
            format!("({pct:.0}%)  "),
            Style::default().fg(theme.gauge_color(pct)),
        ));
    }

    // Mirrors the v0.4 metrics-server badge: state the capability rather than
    // letting an empty table imply an idle GPU.
    if snap.any_process_stats() {
        spans.push(Span::styled(
            format!("procs: {}", snap.processes.len()),
            Style::default().fg(theme.success),
        ));
    } else {
        spans.push(Span::styled(
            "per-process: unsupported",
            Style::default().fg(theme.warning),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.header_bg)),
        area,
    );
}

fn draw_subtab_bar(
    frame: &mut Frame,
    area: Rect,
    theme: &Theme,
    app: &AppState,
    snap: &GpusSnapshot,
) {
    let mut spans = vec![Span::raw(" ")];
    for (idx, sv) in [GpuSubview::Devices, GpuSubview::Procs].iter().enumerate() {
        let (letter, rest) = match sv {
            GpuSubview::Devices => ("D", "evices"),
            GpuSubview::Procs => ("P", "rocs"),
        };
        let active = app.gpu_subview == *sv;
        let style = if active {
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(theme.text_dim)
        };
        spans.push(Span::styled(format!("[{letter}]"), style));
        spans.push(Span::styled(rest, style));
        if idx == 0 {
            spans.push(Span::raw("  "));
        }
    }

    if !snap.any_process_stats() {
        spans.push(Span::styled(
            "   (this backend reports no per-process usage)",
            Style::default().fg(theme.text_dim),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_filter_bar(frame: &mut Frame, area: Rect, theme: &Theme, app: &AppState) {
    let prompt = if app.gpu_filter_active {
        " filter (Esc/Enter to commit): "
    } else {
        " filter: "
    };
    let line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(theme.accent_secondary)),
        Span::styled(
            scrub_ctrl(&app.gpu_filter_input).into_owned(),
            Style::default().fg(theme.fg),
        ),
        if app.gpu_filter_active {
            Span::styled("█", Style::default().fg(theme.fg))
        } else {
            Span::raw("")
        },
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ---- Devices sub-view ---------------------------------------------------

fn draw_devices(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme, snap: &GpusSnapshot) {
    let devices = sort_devices(filter_devices(&snap.devices, &app.gpu_filter_input), app);

    if devices.is_empty() {
        let msg = if app.gpu_filter_input.is_empty() {
            "  No GPU devices."
        } else {
            "  No devices match the filter."
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                msg,
                Style::default().fg(theme.text_dim),
            ))),
            area,
        );
        return;
    }

    let header = make_header(
        &[
            ("#", GpuSortField::DeviceIndex),
            ("NAME", GpuSortField::DeviceName),
            ("VENDOR", GpuSortField::DeviceName),
            ("UTIL", GpuSortField::DeviceUtil),
            ("MEM", GpuSortField::DeviceMem),
            ("MEM%", GpuSortField::DeviceMem),
            ("TEMP", GpuSortField::DeviceTemp),
            ("POWER", GpuSortField::DevicePower),
            ("CLK", GpuSortField::DeviceIndex),
            ("FAN", GpuSortField::DeviceIndex),
            ("ENC/DEC", GpuSortField::DeviceIndex),
        ],
        app,
        theme,
    );

    let visible = devices
        .iter()
        .skip(app.gpu_scroll_offset)
        .take(area.height.saturating_sub(1) as usize);

    let rows = visible.enumerate().map(|(i, d)| {
        let absolute_idx = app.gpu_scroll_offset + i;
        let row_style = row_selection_style(absolute_idx == app.gpu_selected, theme);
        Row::new(vec![
            Cell::from(d.index.to_string()),
            Cell::from(scrub_ctrl(&d.name).into_owned()),
            Cell::from(d.vendor.label()),
            Cell::from(format_pct(d.utilization_pct))
                .style(opt_gauge_style(d.utilization_pct, theme)),
            Cell::from(format_mem_pair(d.mem_used_bytes, d.mem_total_bytes)),
            Cell::from(format_pct(d.mem_pct())).style(opt_gauge_style(d.mem_pct(), theme)),
            Cell::from(format_temp(d.temperature_c)).style(temp_style(d.temperature_c, theme)),
            Cell::from(format_power(d.power_watts, d.power_limit_watts)),
            Cell::from(format_clock(d.graphics_clock_mhz)),
            Cell::from(format_pct(d.fan_pct)),
            Cell::from(format_enc_dec(d.encoder_pct, d.decoder_pct)),
        ])
        .style(row_style)
    });

    let widths = [
        Constraint::Length(3),
        Constraint::Length(28),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(18),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(12),
        Constraint::Length(9),
        Constraint::Length(6),
        Constraint::Length(9),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, area);
}

fn filter_devices<'a>(
    devices: &'a [GpuDeviceSnapshot],
    filter: &str,
) -> Vec<&'a GpuDeviceSnapshot> {
    if filter.is_empty() {
        return devices.iter().collect();
    }
    let f = filter.to_lowercase();
    devices
        .iter()
        .filter(|d| d.name.to_lowercase().contains(&f) || d.vendor.label().contains(&f))
        .collect()
}

fn sort_devices<'a>(
    mut devices: Vec<&'a GpuDeviceSnapshot>,
    app: &AppState,
) -> Vec<&'a GpuDeviceSnapshot> {
    use std::cmp::Ordering;
    let asc = matches!(app.gpu_sort_order, SortOrder::Asc);
    devices.sort_by(|a, b| {
        let ord = match app.gpu_sort_field {
            // Identifier-like fields ascend under the default `Desc` order,
            // matching how the Kube tab treats `PodName`: each field picks
            // its own natural direction and `S`/`I` flips it. GPU 0 first,
            // like `nvidia-smi`.
            GpuSortField::DeviceIndex => a.index.cmp(&b.index),
            GpuSortField::DeviceName => a.name.cmp(&b.name),
            GpuSortField::DeviceUtil => cmp_opt_f32(a.utilization_pct, b.utilization_pct),
            GpuSortField::DeviceMem => cmp_opt_f32(a.mem_pct(), b.mem_pct()),
            GpuSortField::DeviceTemp => cmp_opt_f32(a.temperature_c, b.temperature_c),
            GpuSortField::DevicePower => cmp_opt_f32(a.power_watts, b.power_watts),
            _ => Ordering::Equal,
        };
        if asc { ord.reverse() } else { ord }
    });
    devices
}

// ---- Procs sub-view -----------------------------------------------------

fn draw_procs(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme, snap: &GpusSnapshot) {
    let procs = sort_procs(filter_procs(&snap.processes, &app.gpu_filter_input), app);

    if procs.is_empty() {
        // Three genuinely different situations, three different messages.
        // Rendering "no processes" when the backend simply cannot answer
        // would be a lie of the kind the `Option` model exists to prevent.
        let msg = if !app.gpu_filter_input.is_empty() {
            "  No GPU processes match the filter."
        } else if !snap.any_process_stats() {
            "  Per-process GPU usage is not available from this backend.\n\
             \n  The AMD amdgpu sysfs interface exposes no per-process accounting,\n  \
             and NVML reports it only for NVIDIA devices."
        } else {
            "  No process is currently using the GPU."
        };
        let lines: Vec<Line<'_>> = msg
            .lines()
            .map(|l| Line::from(Span::styled(l, Style::default().fg(theme.text_dim))))
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let header = make_header(
        &[
            ("PID", GpuSortField::ProcPid),
            ("NAME", GpuSortField::ProcName),
            ("GPU", GpuSortField::ProcDevice),
            ("TYPE", GpuSortField::ProcPid),
            ("GPU MEM", GpuSortField::ProcMem),
        ],
        app,
        theme,
    );

    let visible = procs
        .iter()
        .skip(app.gpu_scroll_offset)
        .take(area.height.saturating_sub(1) as usize);

    let rows = visible.enumerate().map(|(i, p)| {
        let absolute_idx = app.gpu_scroll_offset + i;
        let row_style = row_selection_style(absolute_idx == app.gpu_selected, theme);
        // A PID whose name could not be resolved died between the GPU query
        // and the process-table lookup. The row is still worth showing.
        let name = if p.name.is_empty() {
            "—".to_string()
        } else {
            scrub_ctrl(&p.name).into_owned()
        };
        Row::new(vec![
            Cell::from(p.pid.to_string()),
            Cell::from(name),
            Cell::from(p.device_index.to_string()),
            Cell::from(p.kind.label()),
            Cell::from(match p.mem_bytes {
                Some(b) => format_bytes(b),
                None => "—".to_string(),
            }),
        ])
        .style(row_style)
    });

    let widths = [
        Constraint::Length(9),
        Constraint::Length(34),
        Constraint::Length(5),
        Constraint::Length(10),
        Constraint::Length(12),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, area);
}

fn filter_procs<'a>(procs: &'a [GpuProcessSnapshot], filter: &str) -> Vec<&'a GpuProcessSnapshot> {
    if filter.is_empty() {
        return procs.iter().collect();
    }
    let f = filter.to_lowercase();
    procs
        .iter()
        .filter(|p| p.name.to_lowercase().contains(&f) || p.pid.to_string().contains(&f))
        .collect()
}

fn sort_procs<'a>(
    mut procs: Vec<&'a GpuProcessSnapshot>,
    app: &AppState,
) -> Vec<&'a GpuProcessSnapshot> {
    use std::cmp::Ordering;
    let asc = matches!(app.gpu_sort_order, SortOrder::Asc);
    procs.sort_by(|a, b| {
        let ord = match app.gpu_sort_field {
            // Identifier-like fields ascend; magnitude-like fields descend.
            GpuSortField::ProcPid => a.pid.cmp(&b.pid),
            GpuSortField::ProcName => a.name.cmp(&b.name),
            GpuSortField::ProcMem => a
                .mem_bytes
                .unwrap_or(0)
                .cmp(&b.mem_bytes.unwrap_or(0))
                .reverse(),
            GpuSortField::ProcDevice => a
                .device_index
                .cmp(&b.device_index)
                // Stable secondary key so rows on the same GPU keep a fixed
                // order instead of shuffling between frames.
                .then_with(|| a.pid.cmp(&b.pid)),
            _ => Ordering::Equal,
        };
        if asc { ord.reverse() } else { ord }
    });
    procs
}

// ---- Shared helpers -----------------------------------------------------

/// Order two optional metrics, biggest first, treating `None` as smallest.
///
/// A device that cannot report temperature sinks to the bottom of a
/// temperature sort rather than pretending to be at 0 °C and topping an
/// ascending one.
fn cmp_opt_f32(a: Option<f32>, b: Option<f32>) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal).reverse(),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn make_header<'a>(cols: &'a [(&'a str, GpuSortField)], app: &AppState, theme: &Theme) -> Row<'a> {
    let cells: Vec<Cell<'a>> = cols
        .iter()
        .map(|(label, field)| {
            let mut text = (*label).to_string();
            if app.gpu_sort_field == *field {
                text.push(' ');
                text.push(match app.gpu_sort_order {
                    SortOrder::Asc => '↑',
                    SortOrder::Desc => '↓',
                });
            }
            Cell::from(text)
        })
        .collect();
    Row::new(cells).style(
        Style::default()
            .fg(theme.accent_primary)
            .add_modifier(Modifier::BOLD),
    )
}

fn row_selection_style(selected: bool, theme: &Theme) -> Style {
    if selected {
        Style::default()
            .bg(theme.selection_bg)
            .fg(theme.selection_fg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

/// Colour an optional percentage with the shared gauge ramp; unknown values
/// stay dim rather than borrowing the "healthy green" of a real 0 %.
fn opt_gauge_style(pct: Option<f32>, theme: &Theme) -> Style {
    match pct {
        Some(p) => Style::default().fg(theme.gauge_color(p as f64)),
        None => Style::default().fg(theme.text_dim),
    }
}

/// GPU temperature ramp. Thresholds are GPU-specific rather than the generic
/// `gauge_color` ones: 80 °C is unremarkable for a GPU under load, where the
/// same number on a CPU would be alarming.
fn temp_style(temp: Option<f32>, theme: &Theme) -> Style {
    match temp {
        None => Style::default().fg(theme.text_dim),
        Some(t) if t >= 90.0 => Style::default().fg(theme.danger),
        Some(t) if t >= 80.0 => Style::default().fg(theme.warning),
        Some(_) => Style::default().fg(theme.success),
    }
}

fn format_pct(pct: Option<f32>) -> String {
    match pct {
        Some(p) => format!("{p:.0}%"),
        None => "—".to_string(),
    }
}

fn format_temp(temp: Option<f32>) -> String {
    match temp {
        Some(t) => format!("{t:.0}°C"),
        None => "—".to_string(),
    }
}

fn format_clock(mhz: Option<u32>) -> String {
    match mhz {
        Some(m) => format!("{m} MHz"),
        None => "—".to_string(),
    }
}

/// `210W/450W`, or just `210W` when no cap is reported, or `—`.
fn format_power(watts: Option<f32>, limit: Option<f32>) -> String {
    match (watts, limit) {
        (Some(w), Some(l)) => format!("{w:.0}W/{l:.0}W"),
        (Some(w), None) => format!("{w:.0}W"),
        (None, _) => "—".to_string(),
    }
}

/// `6.0G/24.0G`, degrading to whichever half is known.
fn format_mem_pair(used: Option<u64>, total: Option<u64>) -> String {
    match (used, total) {
        (Some(u), Some(t)) => format!("{}/{}", format_bytes(u), format_bytes(t)),
        (Some(u), None) => format_bytes(u),
        (None, Some(t)) => format!("—/{}", format_bytes(t)),
        (None, None) => "—".to_string(),
    }
}

/// `0%/12%` — the two NVML video-engine counters in one column. Both are
/// absent on AMD, so the whole cell collapses to `—` there.
fn format_enc_dec(enc: Option<f32>, dec: Option<f32>) -> String {
    match (enc, dec) {
        (None, None) => "—".to_string(),
        (e, d) => format!("{}/{}", format_pct(e), format_pct(d)),
    }
}

/// Binary-prefix byte formatter, matching the Containers tab's convention.
fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{bytes}B")
    } else if b < KIB * KIB {
        format!("{:.1}K", b / KIB)
    } else if b < KIB * KIB * KIB {
        format!("{:.1}M", b / (KIB * KIB))
    } else {
        format!("{:.1}G", b / (KIB * KIB * KIB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use muxtop_core::gpu::{GpuBackend, GpuProcessKind, GpuVendor};

    fn device(index: u32, name: &str) -> GpuDeviceSnapshot {
        GpuDeviceSnapshot {
            index,
            vendor: GpuVendor::Nvidia,
            backend: GpuBackend::Nvml,
            name: name.into(),
            bus_id: "0000:01:00.0".into(),
            driver_version: None,
            utilization_pct: Some(50.0),
            mem_utilization_pct: Some(20.0),
            mem_used_bytes: Some(6 * 1024 * 1024 * 1024),
            mem_total_bytes: Some(24 * 1024 * 1024 * 1024),
            temperature_c: Some(60.0),
            power_watts: Some(200.0),
            power_limit_watts: Some(450.0),
            graphics_clock_mhz: Some(2000),
            memory_clock_mhz: Some(9500),
            fan_pct: Some(40.0),
            encoder_pct: Some(0.0),
            decoder_pct: Some(10.0),
            supports_process_stats: true,
        }
    }

    // ---- formatters: the `—` contract ------------------------------------

    #[test]
    fn unknown_metrics_render_as_a_dash_not_zero() {
        // The single most important rendering rule in this tab.
        assert_eq!(format_pct(None), "—");
        assert_eq!(format_temp(None), "—");
        assert_eq!(format_clock(None), "—");
        assert_eq!(format_power(None, Some(450.0)), "—");
        assert_eq!(format_mem_pair(None, None), "—");
        assert_eq!(format_enc_dec(None, None), "—");

        // And a real zero is still a zero.
        assert_eq!(format_pct(Some(0.0)), "0%");
        assert_eq!(format_temp(Some(0.0)), "0°C");
    }

    #[test]
    fn power_degrades_when_no_cap_is_reported() {
        assert_eq!(format_power(Some(210.4), Some(450.0)), "210W/450W");
        assert_eq!(format_power(Some(210.4), None), "210W");
    }

    #[test]
    fn mem_pair_degrades_on_either_side() {
        assert_eq!(
            format_mem_pair(Some(1024 * 1024 * 1024), Some(2 * 1024 * 1024 * 1024)),
            "1.0G/2.0G"
        );
        assert_eq!(format_mem_pair(Some(1024), None), "1.0K");
        assert_eq!(format_mem_pair(None, Some(1024)), "—/1.0K");
    }

    #[test]
    fn enc_dec_shows_partial_data() {
        // One engine reporting and the other not is real on some cards.
        assert_eq!(format_enc_dec(Some(0.0), Some(12.0)), "0%/12%");
        assert_eq!(format_enc_dec(Some(5.0), None), "5%/—");
    }

    #[test]
    fn format_bytes_uses_binary_prefixes() {
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(2048), "2.0K");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0M");
        assert_eq!(format_bytes(24 * 1024 * 1024 * 1024), "24.0G");
    }

    // ---- sorting ----------------------------------------------------------

    #[test]
    fn unknown_metrics_sink_to_the_bottom() {
        use std::cmp::Ordering;
        // Descending by temperature: a device that reports nothing must not
        // outrank one that reports 90 °C.
        assert_eq!(cmp_opt_f32(Some(90.0), None), Ordering::Less);
        assert_eq!(cmp_opt_f32(None, Some(10.0)), Ordering::Greater);
        assert_eq!(cmp_opt_f32(None, None), Ordering::Equal);
        assert_eq!(cmp_opt_f32(Some(90.0), Some(10.0)), Ordering::Less);
    }

    #[test]
    fn cmp_opt_f32_survives_nan() {
        use std::cmp::Ordering;
        // `partial_cmp` returns None for NaN; the fallback must not panic and
        // must not produce an inconsistent comparator (which would make
        // `sort_by` panic on some inputs).
        assert_eq!(cmp_opt_f32(Some(f32::NAN), Some(1.0)), Ordering::Equal);
    }

    #[test]
    fn devices_sort_by_index_ascending_by_default() {
        let mut app = AppState::new();
        app.gpu_sort_field = GpuSortField::DeviceIndex;
        app.gpu_sort_order = SortOrder::Desc;

        let devices = [device(2, "c"), device(0, "a"), device(1, "b")];
        let sorted = sort_devices(devices.iter().collect(), &app);
        let indices: Vec<u32> = sorted.iter().map(|d| d.index).collect();
        assert_eq!(
            indices,
            vec![0, 1, 2],
            "GPU 0 must come first — users refer to GPUs by index"
        );
    }

    #[test]
    fn devices_sort_by_utilisation_puts_the_busiest_first() {
        let mut app = AppState::new();
        app.gpu_sort_field = GpuSortField::DeviceUtil;
        app.gpu_sort_order = SortOrder::Desc;

        let mut idle = device(0, "idle");
        idle.utilization_pct = Some(2.0);
        let mut busy = device(1, "busy");
        busy.utilization_pct = Some(97.0);
        let devices = [idle, busy];

        let sorted = sort_devices(devices.iter().collect(), &app);
        assert_eq!(sorted[0].name, "busy");
    }

    #[test]
    fn procs_sort_by_memory_puts_the_hog_first() {
        let mut app = AppState::new();
        app.gpu_sort_field = GpuSortField::ProcMem;
        app.gpu_sort_order = SortOrder::Desc;

        let procs = [
            GpuProcessSnapshot {
                pid: 1,
                device_index: 0,
                name: "small".into(),
                kind: GpuProcessKind::Compute,
                mem_bytes: Some(1024),
            },
            GpuProcessSnapshot {
                pid: 2,
                device_index: 0,
                name: "huge".into(),
                kind: GpuProcessKind::Compute,
                mem_bytes: Some(8 * 1024 * 1024 * 1024),
            },
        ];

        let sorted = sort_procs(procs.iter().collect(), &app);
        assert_eq!(sorted[0].name, "huge");
    }

    // ---- filtering --------------------------------------------------------

    #[test]
    fn device_filter_matches_name_and_vendor() {
        let devices = vec![device(0, "NVIDIA GeForce RTX 4090")];
        assert_eq!(filter_devices(&devices, "").len(), 1);
        assert_eq!(filter_devices(&devices, "rtx").len(), 1);
        assert_eq!(filter_devices(&devices, "nvidia").len(), 1);
        assert_eq!(filter_devices(&devices, "radeon").len(), 0);
    }

    #[test]
    fn proc_filter_matches_pid_as_well_as_name() {
        // A user chasing a runaway job usually has the PID, not the name.
        let procs = vec![GpuProcessSnapshot {
            pid: 4242,
            device_index: 0,
            name: "ollama".into(),
            kind: GpuProcessKind::Compute,
            mem_bytes: Some(1024),
        }];
        assert_eq!(filter_procs(&procs, "4242").len(), 1);
        assert_eq!(filter_procs(&procs, "olla").len(), 1);
        assert_eq!(filter_procs(&procs, "9999").len(), 0);
    }

    // ---- detail wrapping --------------------------------------------------

    #[test]
    fn detail_wraps_to_width() {
        let text = "Apple Silicon GPU monitoring is not implemented yet planned for v0.6";
        let lines = wrap_detail(text, 20);
        assert!(lines.len() > 1, "long detail should wrap");
        for line in &lines {
            assert!(line.chars().count() <= 20, "line too long: {line:?}");
        }
        // No word is lost in the wrap.
        assert_eq!(lines.join(" "), text);
    }

    #[test]
    fn detail_wrapping_survives_a_tiny_terminal() {
        let lines = wrap_detail("some reason", 3);
        assert_eq!(lines, vec!["some reason".to_string()]);
    }

    #[test]
    fn detail_wrapping_handles_empty_input() {
        assert_eq!(wrap_detail("", 40), vec![String::new()]);
    }
}
