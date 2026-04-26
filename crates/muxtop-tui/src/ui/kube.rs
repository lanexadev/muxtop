//! Kubernetes tab UI (Alt+5) — v0.4.0.
//!
//! Renders a table of pods, nodes, or deployments depending on the active
//! sub-view. The data comes from the latest `KubeSnapshot` carried by the
//! `SystemSnapshot`. The kube-rs poll loop runs in `muxtop-core` and is
//! invisible to this module; we just walk vecs.
//!
//! ## Status (E5 minimal)
//!
//! v0.4.0 ships the **Pods sub-view** with the table layout and degradation
//! states (no-cluster / no-metrics / empty). Nodes and Deployments
//! sub-views, sub-view switching (P/N/D), per-row sparklines, sort cycling
//! and column-arrow indicators are reserved for v0.4.x — the public state
//! and data model are already in place so the wiring is mechanical.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use muxtop_core::kube::{ClusterKind, KubeSnapshot, PodPhase};

use crate::app::AppState;
use crate::ui::sanitize::scrub_ctrl;
use crate::ui::theme::Theme;

pub fn draw_kube_tab(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let snap = app.last_snapshot.as_ref().and_then(|s| s.kube.as_ref());
    match snap {
        None => draw_waiting(frame, area, theme),
        Some(s) if !s.reachable => draw_unreachable(frame, area, theme, s),
        Some(s) => draw_pods(frame, area, theme, s),
    }
}

fn draw_waiting(frame: &mut Frame, area: Rect, theme: &Theme) {
    let line = Line::from(vec![Span::styled(
        " Waiting for cluster data… ",
        Style::default().fg(theme.fg).bg(theme.header_bg),
    )]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_unreachable(frame: &mut Frame, area: Rect, theme: &Theme, snap: &KubeSnapshot) {
    let detail = if snap.server_version.is_some() {
        "Cluster reachable but no kubeconfig context active."
    } else {
        "No cluster reachable. Check $KUBECONFIG or `kubectl config use-context`."
    };
    let lines = vec![
        Line::from(Span::styled(
            "  No cluster data",
            Style::default()
                .fg(theme.danger)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("  {detail}"),
            Style::default().fg(theme.text_dim),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_pods(frame: &mut Frame, area: Rect, theme: &Theme, snap: &KubeSnapshot) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    // ---- Summary header ----
    let kind_label = cluster_kind_label(snap.cluster_kind);
    let metrics_badge = if snap.metrics_available {
        Span::styled("metrics-server: on", Style::default().fg(theme.success))
    } else {
        Span::styled("metrics-server: off", Style::default().fg(theme.warning))
    };
    let summary = Line::from(vec![
        Span::styled(
            format!(" Cluster: {kind_label}"),
            Style::default().fg(theme.accent_primary),
        ),
        Span::raw("  "),
        Span::raw(format!("ns: {}  ", snap.current_namespace)),
        Span::raw(format!("pods: {}  ", snap.pods.len())),
        Span::raw(format!("nodes: {}  ", snap.nodes.len())),
        Span::raw(format!("deployments: {}  ", snap.deployments.len())),
        metrics_badge,
    ]);
    frame.render_widget(
        Paragraph::new(summary).style(Style::default().bg(theme.header_bg)),
        chunks[0],
    );

    // ---- Pod table ----
    if snap.pods.is_empty() {
        let line = Line::from(Span::styled(
            "  No pods in this cluster.",
            Style::default().fg(theme.text_dim),
        ));
        frame.render_widget(Paragraph::new(line), chunks[1]);
        return;
    }

    let header = Row::new(vec![
        Cell::from("NAMESPACE"),
        Cell::from("NAME"),
        Cell::from("READY"),
        Cell::from("STATUS"),
        Cell::from("RESTARTS"),
        Cell::from("AGE"),
        Cell::from("CPU"),
        Cell::from("MEM"),
        Cell::from("NODE"),
    ])
    .style(
        Style::default()
            .fg(theme.accent_primary)
            .add_modifier(Modifier::BOLD),
    );

    let rows = snap.pods.iter().take(area.height as usize).map(|p| {
        let phase_style = pod_phase_style(p.phase, theme);
        Row::new(vec![
            Cell::from(scrub_ctrl(&p.namespace).into_owned()),
            Cell::from(scrub_ctrl(&p.name).into_owned()),
            Cell::from(format!("{}/{}", p.ready.0, p.ready.1)),
            Cell::from(pod_phase_label(p.phase)).style(phase_style),
            Cell::from(p.restarts.to_string()),
            Cell::from(format_age(p.age_seconds)),
            Cell::from(format_cpu(p.cpu_millis)),
            Cell::from(format_mem(p.mem_bytes)),
            Cell::from(scrub_ctrl(&p.node).into_owned()),
        ])
    });

    let widths = [
        Constraint::Length(20),
        Constraint::Length(40),
        Constraint::Length(7),
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Length(20),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, chunks[1]);
}

fn cluster_kind_label(k: ClusterKind) -> &'static str {
    match k {
        ClusterKind::Generic => "k8s",
        ClusterKind::Kind => "kind",
        ClusterKind::K3d => "k3d",
        ClusterKind::K3s => "k3s",
        ClusterKind::Eks => "eks",
        ClusterKind::Gke => "gke",
        ClusterKind::Aks => "aks",
        ClusterKind::Openshift => "openshift",
    }
}

fn pod_phase_label(p: PodPhase) -> &'static str {
    match p {
        PodPhase::Pending => "Pending",
        PodPhase::Running => "Running",
        PodPhase::Succeeded => "Succeeded",
        PodPhase::Failed => "Failed",
        PodPhase::CrashLoop => "CrashLoop",
        PodPhase::Terminating => "Terminating",
        PodPhase::Unknown => "Unknown",
    }
}

fn pod_phase_style(p: PodPhase, theme: &Theme) -> Style {
    match p {
        PodPhase::Running => Style::default().fg(theme.success),
        PodPhase::Pending => Style::default().fg(theme.warning),
        PodPhase::Succeeded => Style::default().fg(theme.accent_secondary),
        PodPhase::Failed | PodPhase::CrashLoop => Style::default().fg(theme.danger),
        PodPhase::Terminating => Style::default().fg(theme.text_dim),
        PodPhase::Unknown => Style::default().fg(theme.text_dim),
    }
}

fn format_age(seconds: u64) -> String {
    if seconds >= 86_400 {
        format!("{}d", seconds / 86_400)
    } else if seconds >= 3_600 {
        format!("{}h", seconds / 3_600)
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn format_cpu(millis: Option<u32>) -> String {
    match millis {
        Some(m) if m >= 1_000 => format!("{:.1}", m as f32 / 1_000.0),
        Some(m) => format!("{m}m"),
        None => "—".into(),
    }
}

fn format_mem(bytes: Option<u64>) -> String {
    let Some(b) = bytes else {
        return "—".into();
    };
    let kib = b as f64 / 1_024.0;
    if kib >= 1_048_576.0 {
        format!("{:.1}G", kib / 1_048_576.0)
    } else if kib >= 1_024.0 {
        format!("{:.1}M", kib / 1_024.0)
    } else {
        format!("{:.0}K", kib)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_age_buckets() {
        assert_eq!(format_age(45), "45s");
        assert_eq!(format_age(120), "2m");
        assert_eq!(format_age(3_600), "1h");
        assert_eq!(format_age(90_000), "1d");
    }

    #[test]
    fn format_cpu_buckets() {
        assert_eq!(format_cpu(None), "—");
        assert_eq!(format_cpu(Some(0)), "0m");
        assert_eq!(format_cpu(Some(150)), "150m");
        assert_eq!(format_cpu(Some(1_500)), "1.5");
    }

    #[test]
    fn format_mem_buckets() {
        assert_eq!(format_mem(None), "—");
        // 512 bytes is below 1 KiB → rounds to "0K"
        assert_eq!(format_mem(Some(512)), "0K");
        assert_eq!(format_mem(Some(2048)), "2K");
        assert_eq!(format_mem(Some(1024 * 1024)), "1.0M");
        assert_eq!(format_mem(Some(2 * 1024 * 1024 * 1024)), "2.0G");
    }

    #[test]
    fn pod_phase_label_is_exhaustive() {
        for p in [
            PodPhase::Pending,
            PodPhase::Running,
            PodPhase::Succeeded,
            PodPhase::Failed,
            PodPhase::CrashLoop,
            PodPhase::Terminating,
            PodPhase::Unknown,
        ] {
            let _ = pod_phase_label(p);
        }
    }

    #[test]
    fn cluster_kind_label_is_exhaustive() {
        for k in [
            ClusterKind::Generic,
            ClusterKind::Kind,
            ClusterKind::K3d,
            ClusterKind::K3s,
            ClusterKind::Eks,
            ClusterKind::Gke,
            ClusterKind::Aks,
            ClusterKind::Openshift,
        ] {
            let _ = cluster_kind_label(k);
        }
    }

    #[test]
    fn pod_to_snapshot_label_paths_compile() {
        // Ensures we render every PodPhase variant's style without panic.
        let theme = crate::ui::theme::Theme::default();
        for p in [
            PodPhase::Pending,
            PodPhase::Running,
            PodPhase::Succeeded,
            PodPhase::Failed,
            PodPhase::CrashLoop,
            PodPhase::Terminating,
            PodPhase::Unknown,
        ] {
            let _style = pod_phase_style(p, &theme);
        }
    }

    /// Smoke test: the unreachable / waiting / empty paths render without
    /// panic on a minimal terminal buffer.
    #[test]
    fn smoke_render_unreachable_does_not_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(80, 20);
        let mut term = Terminal::new(backend).unwrap();
        let theme = crate::ui::theme::Theme::default();
        term.draw(|f| {
            let area = f.area();
            let snap = KubeSnapshot::unavailable();
            draw_unreachable(f, area, &theme, &snap);
        })
        .unwrap();
    }

    #[test]
    fn smoke_render_pods_table_does_not_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(120, 30);
        let mut term = Terminal::new(backend).unwrap();
        let theme = crate::ui::theme::Theme::default();
        let pod = muxtop_core::kube::PodSnapshot {
            namespace: "default".into(),
            name: "nginx".into(),
            phase: PodPhase::Running,
            ready: (1, 1),
            restarts: 0,
            age_seconds: 3600,
            node: "node-1".into(),
            cpu_millis: Some(15),
            mem_bytes: Some(64 * 1024 * 1024),
            qos: muxtop_core::kube::QosClass::Burstable,
        };
        let snap = KubeSnapshot {
            cluster_kind: ClusterKind::Kind,
            server_version: Some("v1.31.0".into()),
            current_namespace: "default".into(),
            reachable: true,
            metrics_available: true,
            pods: vec![pod],
            nodes: vec![],
            deployments: vec![],
        };
        term.draw(|f| draw_pods(f, f.area(), &theme, &snap))
            .unwrap();
    }
}
