// General tab — the dashboard.
//
// 0.4 drew CPU bars, memory bars, one info line, and then a `Constraint::Min(0)`
// of nothing: on a 50-row terminal roughly half of this tab was blank. The space
// now carries the summary that makes a tabbed monitor worth having — including
// the cross-tab "Workloads" card, which is where a future GPU summary lands with
// no layout work at all.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::Render;
use super::sanitize::scrub_ctrl;
use super::widgets::empty::{self, EmptyState};
use super::widgets::{meter, panel, spark};
use crate::terminal::Breakpoint;
use crate::ui::theme::Level;
use muxtop_core::system::SystemSnapshot;

pub fn draw_general_tab(frame: &mut Frame, area: Rect, r: &Render<'_>) {
    let Some(snapshot) = r.app.last_snapshot.as_ref() else {
        let waiting = r.ellipsis("Waiting for data");
        empty::render(frame, area, &EmptyState::waiting(&waiting), r.theme);
        return;
    };

    let two_column = r.breakpoint >= Breakpoint::Md && area.width >= 96;
    let cpu_h = cpu_height(snapshot, area.height, two_column);
    let mem_h = if snapshot.memory.swap_total > 0 { 4 } else { 3 };

    let [top, middle, bottom] = Layout::vertical([
        Constraint::Length(cpu_h),
        Constraint::Length(mem_h),
        Constraint::Fill(1),
    ])
    .areas(area);

    if two_column {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(60), Constraint::Fill(1)]).areas(top);
        draw_cpu(frame, left, r, snapshot);
        draw_load(frame, right, r, snapshot);

        let [left, right] =
            Layout::horizontal([Constraint::Percentage(60), Constraint::Fill(1)]).areas(middle);
        draw_memory(frame, left, r, snapshot);
        draw_network(frame, right, r, snapshot);
    } else {
        draw_cpu(frame, top, r, snapshot);
        draw_memory(frame, middle, r, snapshot);
    }

    if bottom.height >= 3 {
        if two_column {
            let [left, right] =
                Layout::horizontal([Constraint::Percentage(60), Constraint::Fill(1)]).areas(bottom);
            draw_top_processes(frame, left, r, snapshot);
            draw_workloads(frame, right, r, snapshot);
        } else {
            draw_top_processes(frame, bottom, r, snapshot);
        }
    }
}

/// How tall the CPU panel should be: enough for its cores, but never more than
/// half the screen — the panels below it carry information too.
fn cpu_height(snapshot: &SystemSnapshot, available: u16, two_column: bool) -> u16 {
    let cores = snapshot.cpu.cores.len() as u16;
    if cores == 0 {
        return 3;
    }
    let rows = if cores > 8 || two_column {
        cores.div_ceil(2)
    } else {
        cores
    };
    (rows + 2).clamp(3, (available / 2).max(3))
}

// ---------------------------------------------------------------------------
// Panels
// ---------------------------------------------------------------------------

fn draw_cpu(frame: &mut Frame, area: Rect, r: &Render<'_>, snapshot: &SystemSnapshot) {
    let block = panel::block(Some("CPU"), false, r.theme, r.glyphs);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let cores = &snapshot.cpu.cores;
    if cores.is_empty() {
        return;
    }

    // Two columns when there are many cores, or when the panel is wide enough
    // that one core per row would waste most of it.
    let columns = if cores.len() > inner.height as usize || inner.width >= 64 {
        2
    } else {
        1
    };

    if columns == 1 {
        let lines: Vec<Line<'static>> = cores
            .iter()
            .take(inner.height as usize)
            .map(|c| core_bar(&c.name, f64::from(c.usage), inner.width, r))
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    } else {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Fill(1)]).areas(inner);
        let mid = cores.len().div_ceil(2);
        let build = |slice: &[muxtop_core::system::CoreSnapshot], width: u16, height: u16| {
            slice
                .iter()
                .take(height as usize)
                .map(|c| core_bar(&c.name, f64::from(c.usage), width, r))
                .collect::<Vec<_>>()
        };
        frame.render_widget(
            Paragraph::new(build(&cores[..mid], left.width, left.height)),
            left,
        );
        frame.render_widget(
            Paragraph::new(build(&cores[mid..], right.width, right.height)),
            right,
        );
    }
}

fn core_bar(name: &str, usage: f64, width: u16, r: &Render<'_>) -> Line<'static> {
    meter::bar_line(
        name,
        6,
        &format!("{usage:.1}%"),
        usage,
        width,
        r.theme,
        r.glyphs,
    )
}

fn draw_memory(frame: &mut Frame, area: Rect, r: &Render<'_>, snapshot: &SystemSnapshot) {
    let block = panel::block(Some("Memory"), false, r.theme, r.glyphs);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mem = &snapshot.memory;
    let mut lines = vec![mem_bar("Mem", mem.used, mem.total, inner.width, r)];
    if mem.swap_total > 0 {
        lines.push(mem_bar(
            "Swap",
            mem.swap_used,
            mem.swap_total,
            inner.width,
            r,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn mem_bar(label: &str, used: u64, total: u64, width: u16, r: &Render<'_>) -> Line<'static> {
    let pct = if total > 0 {
        (used as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let info = format!(
        "{pct:.0}%  {}/{}",
        meter::human_bytes(used),
        meter::human_bytes(total)
    );
    meter::bar_line(label, 6, &info, pct, width, r.theme, r.glyphs)
}

fn draw_load(frame: &mut Frame, area: Rect, r: &Render<'_>, snapshot: &SystemSnapshot) {
    let block = panel::block(Some("Load"), false, r.theme, r.glyphs);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let cores = snapshot.cpu.cores.len().max(1) as f64;
    let load = &snapshot.load;
    // Load is only meaningful against the core count — 4.0 is a catastrophe on
    // a 2-core box and a quiet afternoon on a 64-core one.
    let lines: Vec<Line<'static>> = [("1m", load.one), ("5m", load.five), ("15m", load.fifteen)]
        .into_iter()
        .take(inner.height as usize)
        .map(|(label, value)| {
            let pct = (value / cores * 100.0).clamp(0.0, 100.0);
            meter::bar_line(
                label,
                4,
                &format!("{value:.2}"),
                pct,
                inner.width,
                r.theme,
                r.glyphs,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_network(frame: &mut Frame, area: Rect, r: &Render<'_>, _snapshot: &SystemSnapshot) {
    let block = panel::block(Some("Network"), false, r.theme, r.glyphs);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width < 12 {
        return;
    }

    let history = &r.app.network_history;
    let interfaces = r.app.visible_interfaces();
    // The busiest interface is the one worth graphing on a summary screen.
    let Some(busiest) = interfaces.iter().max_by(|a, b| {
        (history.bandwidth_rx(&a.name) + history.bandwidth_tx(&a.name))
            .partial_cmp(&(history.bandwidth_rx(&b.name) + history.bandwidth_tx(&b.name)))
            .unwrap_or(std::cmp::Ordering::Equal)
    }) else {
        return;
    };

    let width = inner.width.saturating_sub(12);
    let rx = history.sparkline_rx(&busiest.name, width as usize);
    let tx = history.sparkline_tx(&busiest.name, width as usize);
    let scale = rx
        .iter()
        .chain(tx.iter())
        .copied()
        .max()
        .unwrap_or(1)
        .max(1);

    let mut lines = Vec::with_capacity(2);
    for (arrow, data, rate, color) in [
        (
            r.glyphs.arrow_down,
            rx,
            history.bandwidth_rx(&busiest.name),
            r.theme.success,
        ),
        (
            r.glyphs.arrow_up,
            tx,
            history.bandwidth_tx(&busiest.name),
            r.theme.info,
        ),
    ] {
        let mut spans = vec![Span::styled(
            format!("{arrow} {:>9} ", meter::human_rate(rate as u64)),
            r.theme.dim(),
        )];
        spans.extend(
            spark::line(
                &data,
                width,
                Some(scale),
                spark::Tint::Flat,
                color,
                r.theme,
                r.glyphs,
            )
            .spans,
        );
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_top_processes(frame: &mut Frame, area: Rect, r: &Render<'_>, snapshot: &SystemSnapshot) {
    let block = panel::block(Some("Top processes"), false, r.theme, r.glyphs);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut procs: Vec<_> = snapshot.processes.iter().collect();
    procs.sort_by(|a, b| {
        b.cpu_percent
            .partial_cmp(&a.cpu_percent)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut lines = vec![Line::from(Span::styled(
        format!("{:>6} {:>6}  COMMAND", "CPU%", "MEM%"),
        r.theme.dim(),
    ))];
    let cmd_w = (inner.width as usize).saturating_sub(15);
    for p in procs.iter().take(inner.height.saturating_sub(1) as usize) {
        let command = scrub_ctrl(&p.command);
        lines.push(Line::from(vec![
            Span::styled(
                format!("{:>6.1} ", p.cpu_percent),
                ratatui::style::Style::default().fg(r.theme.gauge_color(f64::from(p.cpu_percent))),
            ),
            Span::styled(format!("{:>6.1}  ", p.memory_percent), r.theme.body()),
            Span::styled(
                r.glyphs.truncate(&command, cmd_w).into_owned(),
                r.theme.body(),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Cross-tab summary: what the other tabs would tell you, without switching.
fn draw_workloads(frame: &mut Frame, area: Rect, r: &Render<'_>, snapshot: &SystemSnapshot) {
    let block = panel::block(Some("Workloads"), false, r.theme, r.glyphs);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(4);

    let running = snapshot
        .processes
        .iter()
        .filter(|p| p.status == "Running")
        .count();
    lines.push(row(
        "Processes",
        format!(
            "{} {} {running} running",
            snapshot.processes.len(),
            r.glyphs.sep
        ),
        Level::Neutral,
        r,
    ));

    lines.push(match snapshot.containers.as_ref() {
        None => row("Containers", "no engine".into(), Level::Neutral, r),
        Some(cs) if !cs.daemon_up => row("Containers", "daemon down".into(), Level::Error, r),
        Some(cs) => {
            let up = cs
                .containers
                .iter()
                .filter(|c| c.state == muxtop_core::containers::ContainerState::Running)
                .count();
            row(
                "Containers",
                format!(
                    "{up} running {} {} total",
                    r.glyphs.sep,
                    cs.containers.len()
                ),
                Level::Success,
                r,
            )
        }
    });

    lines.push(match snapshot.kube.as_ref() {
        None => row("Kubernetes", "not configured".into(), Level::Neutral, r),
        Some(k) if !k.reachable => row("Kubernetes", "unreachable".into(), Level::Error, r),
        Some(k) => {
            let broken = k
                .pods
                .iter()
                .filter(|p| {
                    matches!(
                        p.phase,
                        muxtop_core::kube::PodPhase::CrashLoop
                            | muxtop_core::kube::PodPhase::Failed
                    )
                })
                .count();
            let level = if broken > 0 {
                Level::Error
            } else {
                Level::Success
            };
            let text = if broken > 0 {
                format!("{} pods {} {broken} failing", k.pods.len(), r.glyphs.sep)
            } else {
                format!(
                    "{} pods {} {} nodes",
                    k.pods.len(),
                    r.glyphs.sep,
                    k.nodes.len()
                )
            };
            row("Kubernetes", text, level, r)
        }
    });

    let visible: Vec<Line<'static>> = lines.into_iter().take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(visible), inner);
}

fn row(label: &str, value: String, level: Level, r: &Render<'_>) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<12}"), r.theme.dim()),
        Span::styled(value, r.theme.level_style(level)),
    ])
}

#[cfg(test)]
mod tests {

    use crate::app::{AppState, Tab};
    use crate::ui::test_support::*;

    fn app() -> AppState {
        let mut app = app_with_data();
        app.tab = Tab::General;
        app
    }

    #[test]
    fn dashboard_shows_cpu_and_memory() {
        let text = all_text(&render_with(&app(), 120, 40));
        assert!(text.contains("CPU"));
        assert!(text.contains("cpu0"));
        assert!(text.contains("Memory"));
        assert!(text.contains("Mem"));
        assert!(text.contains("Swap"));
    }

    #[test]
    fn dashboard_fills_the_screen_instead_of_leaving_it_blank() {
        // The 0.4 layout absorbed the remaining height with nothing in it.
        let text = all_text(&render_with(&app(), 140, 40));
        assert!(text.contains("Top processes"), "wasted space:\n{text}");
        assert!(text.contains("Workloads"));
        assert!(text.contains("Load"));
        assert!(text.contains("Network"));
    }

    #[test]
    fn workloads_card_summarises_the_other_tabs() {
        let text = all_text(&render_with(&app(), 140, 40));
        assert!(text.contains("Processes"));
        assert!(text.contains("Containers"));
        assert!(text.contains("Kubernetes"));
        assert!(text.contains("no engine"), "an absent engine must say so");
    }

    #[test]
    fn narrow_terminals_stack_into_one_column() {
        let text = all_text(&render_with(&app(), 70, 30));
        assert!(text.contains("CPU"));
        assert!(text.contains("Memory"));
        // The side cards are dropped rather than squeezed into nothing.
        assert!(!text.contains("Workloads"));
    }

    #[test]
    fn swap_row_is_hidden_when_there_is_no_swap() {
        let mut snap = snapshot();
        snap.memory.swap_total = 0;
        snap.memory.swap_used = 0;
        let mut app = AppState::new();
        app.tab = Tab::General;
        app.apply_snapshot(snap);
        assert!(!all_text(&render_with(&app, 120, 40)).contains("Swap"));
    }

    #[test]
    fn many_cores_use_two_columns_without_overflowing() {
        let mut snap = snapshot();
        snap.cpu.cores = (0..64)
            .map(|i| muxtop_core::system::CoreSnapshot {
                name: format!("cpu{i}"),
                usage: (i as f32) % 100.0,
                frequency: 3600,
            })
            .collect();
        let mut app = AppState::new();
        app.tab = Tab::General;
        app.apply_snapshot(snap);
        let buf = render_with(&app, 160, 50);
        let text = all_text(&buf);
        assert!(text.contains("cpu0"));
        // Nothing may spill past the panel.
        for row in 0..buf.area.height {
            assert!(line_text(&buf, row).chars().count() <= 160);
        }
    }

    #[test]
    fn zero_cores_does_not_panic() {
        let mut snap = snapshot();
        snap.cpu.cores.clear();
        let mut app = AppState::new();
        app.tab = Tab::General;
        app.apply_snapshot(snap);
        let _ = render_with(&app, 120, 40);
    }

    #[test]
    fn renders_under_every_profile_and_size() {
        let mut app = app();
        for (w, h) in [(1u16, 1u16), (40, 6), (60, 20), (80, 24), (200, 60)] {
            let _ = render_with(&app, w, h);
        }
        for (color, unicode) in all_profiles() {
            let _ = render_caps(&mut app, 140, 40, color, unicode);
        }
    }
}
