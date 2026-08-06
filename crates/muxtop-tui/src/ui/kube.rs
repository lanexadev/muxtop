//! Kubernetes tab UI (Alt+5) — v0.4.0.
//!
//! Renders one of three sub-views (Pods / Nodes / Deployments) selected by
//! the `P` / `N` / `D` keys. The data comes from the latest `KubeSnapshot`
//! carried by the `SystemSnapshot`. The kube-rs poll loop runs in
//! `muxtop-core` and is invisible to this module; we just walk vecs.
//!
//! ## Sort + filter
//!
//! v0.4.0 applied both at render time and bet that per-frame O(n log n)
//! would stay microsecond-level. It does not at the 5 000-pod ceiling
//! `ListParams` allows, and every navigation keypress paid for a full
//! re-filter through `kube_count()`.
//!
//! [`compute_view_indices`] now produces the filtered + sorted order once
//! per state change; `AppState::kube_view_cache` holds it and the draw
//! functions below only index into the snapshot. This is the same contract
//! `sorted_filtered_containers_cache` has for the Containers tab — every
//! mutation of sub-view, filter or sort must refresh the cache.
//!
//! ## Status (v0.4.0)
//!
//! Sub-views, sort cycling, filter, selection scrolling and the ANSI
//! sanitizer (`scrub_ctrl`) are wired here. **Per-row sparklines and the
//! all-namespaces toggle (`A`) are deferred to v0.4.x** — the public state
//! and data model are already in place so the wiring is mechanical.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use muxtop_core::kube::{
    ClusterKind, DeploymentSnapshot, KubeSnapshot, NodeSnapshot, NodeStatus, PodPhase, PodSnapshot,
};

use muxtop_core::process::{SortOrder, contains_ignore_case};

use crate::app::{AppState, KubeSortField, KubeSubview};
use crate::ui::sanitize::scrub_ctrl;
use crate::ui::theme::Theme;

pub fn draw_kube_tab(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let snap = app.last_snapshot.as_ref().and_then(|s| s.kube.as_ref());
    match snap {
        None => draw_waiting(frame, area, theme),
        Some(s) if !s.reachable => draw_unreachable(frame, area, theme, s),
        Some(s) => draw_active(frame, area, app, theme, s),
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

/// Active path: summary line + sub-tab bar + (optional) filter line + table.
fn draw_active(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme, snap: &KubeSnapshot) {
    let show_filter = app.kube_filter_active || !app.kube_filter_input.is_empty();
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
    draw_subtab_bar(frame, chunks[1], theme, app);

    let table_idx = if show_filter {
        draw_filter_bar(frame, chunks[2], theme, app);
        3
    } else {
        2
    };

    match app.kube_subview {
        KubeSubview::Pods => draw_pods(frame, chunks[table_idx], app, theme, snap),
        KubeSubview::Nodes => draw_nodes(frame, chunks[table_idx], app, theme, snap),
        KubeSubview::Deployments => draw_deployments(frame, chunks[table_idx], app, theme, snap),
    }
}

fn draw_summary(frame: &mut Frame, area: Rect, theme: &Theme, snap: &KubeSnapshot) {
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
        // Remote mode carries this field from the server's kubeconfig, so it
        // is no more trustworthy than the pod/node names scrubbed below.
        Span::raw(format!("ns: {}  ", scrub_ctrl(&snap.current_namespace))),
        Span::raw(format!("pods: {}  ", snap.pods.len())),
        Span::raw(format!("nodes: {}  ", snap.nodes.len())),
        Span::raw(format!("deployments: {}  ", snap.deployments.len())),
        metrics_badge,
    ]);
    frame.render_widget(
        Paragraph::new(summary).style(Style::default().bg(theme.header_bg)),
        area,
    );
}

fn draw_subtab_bar(frame: &mut Frame, area: Rect, theme: &Theme, app: &AppState) {
    let mut spans = vec![Span::raw(" ")];
    for (idx, sv) in [
        KubeSubview::Pods,
        KubeSubview::Nodes,
        KubeSubview::Deployments,
    ]
    .iter()
    .enumerate()
    {
        let (label_letter, rest) = match sv {
            KubeSubview::Pods => ("P", "ods"),
            KubeSubview::Nodes => ("N", "odes"),
            KubeSubview::Deployments => ("D", "eployments"),
        };
        let active = app.kube_subview == *sv;
        let style = if active {
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(theme.text_dim)
        };
        spans.push(Span::styled(format!("[{label_letter}]"), style));
        spans.push(Span::styled(rest, style));
        if idx < 2 {
            spans.push(Span::raw("  "));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_filter_bar(frame: &mut Frame, area: Rect, theme: &Theme, app: &AppState) {
    let prompt = if app.kube_filter_active {
        " filter (Esc/Enter to commit): "
    } else {
        " filter: "
    };
    let line = Line::from(vec![
        Span::styled(prompt, Style::default().fg(theme.accent_secondary)),
        Span::styled(
            scrub_ctrl(&app.kube_filter_input).into_owned(),
            Style::default().fg(theme.fg),
        ),
        if app.kube_filter_active {
            Span::styled("█", Style::default().fg(theme.fg))
        } else {
            Span::raw("")
        },
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

// ---- Pods sub-view ------------------------------------------------------

fn draw_pods(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme, snap: &KubeSnapshot) {
    // PERF-03: the filtered+sorted order is computed once per state change
    // in `AppState::recompute_kube_view`, not once per frame. `get` rather
    // than indexing so a cache that briefly trails the snapshot degrades to
    // fewer rows instead of panicking.
    let pods: Vec<&PodSnapshot> = app
        .kube_view_cache
        .iter()
        .filter_map(|&i| snap.pods.get(i))
        .collect();

    if pods.is_empty() {
        let msg = if app.kube_filter_input.is_empty() {
            "  No pods in this cluster."
        } else {
            "  No pods match the filter."
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
            ("NAMESPACE", KubeSortField::PodName), // namespace + name share the Name sort
            ("NAME", KubeSortField::PodName),
            ("READY", KubeSortField::PodCpu), // no dedicated sort
            ("STATUS", KubeSortField::PodPhase),
            ("RESTARTS", KubeSortField::PodRestarts),
            ("AGE", KubeSortField::PodAge),
            ("CPU", KubeSortField::PodCpu),
            ("MEM", KubeSortField::PodMem),
            ("NODE", KubeSortField::PodCpu), // no dedicated sort
        ],
        app,
        theme,
    );

    let visible = pods
        .iter()
        .skip(app.kube_scroll_offset)
        .take(area.height.saturating_sub(1) as usize);

    let rows = visible.enumerate().map(|(i, p)| {
        let phase_style = pod_phase_style(p.phase, theme);
        let absolute_idx = app.kube_scroll_offset + i;
        let row_style = row_selection_style(absolute_idx == app.kube_selected, theme);
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
        .style(row_style)
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
    frame.render_widget(table, area);
}

fn pod_matches(p: &PodSnapshot, needle_lower: &str) -> bool {
    contains_ignore_case(&p.name, needle_lower) || contains_ignore_case(&p.namespace, needle_lower)
}

fn cmp_pods(
    a: &PodSnapshot,
    b: &PodSnapshot,
    field: KubeSortField,
    order: SortOrder,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ord = match field {
        KubeSortField::PodName => a.name.cmp(&b.name),
        KubeSortField::PodCpu => a
            .cpu_millis
            .unwrap_or(0)
            .cmp(&b.cpu_millis.unwrap_or(0))
            .reverse(),
        KubeSortField::PodMem => a
            .mem_bytes
            .unwrap_or(0)
            .cmp(&b.mem_bytes.unwrap_or(0))
            .reverse(),
        KubeSortField::PodRestarts => a.restarts.cmp(&b.restarts).reverse(),
        KubeSortField::PodAge => a.age_seconds.cmp(&b.age_seconds).reverse(),
        KubeSortField::PodPhase => pod_phase_rank(a.phase).cmp(&pod_phase_rank(b.phase)),
        _ => Ordering::Equal, // Out-of-domain → render order = list order
    };
    if matches!(order, SortOrder::Asc) {
        ord.reverse()
    } else {
        ord
    }
}

/// Phase ordering for sort: most-attention-grabbing first (CrashLoop on top).
fn pod_phase_rank(p: PodPhase) -> u8 {
    match p {
        PodPhase::CrashLoop => 0,
        PodPhase::Failed => 1,
        PodPhase::Pending => 2,
        PodPhase::Terminating => 3,
        PodPhase::Running => 4,
        PodPhase::Succeeded => 5,
        PodPhase::Unknown => 6,
    }
}

// ---- Nodes sub-view -----------------------------------------------------

fn draw_nodes(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme, snap: &KubeSnapshot) {
    let nodes: Vec<&NodeSnapshot> = app
        .kube_view_cache
        .iter()
        .filter_map(|&i| snap.nodes.get(i))
        .collect();

    if nodes.is_empty() {
        let msg = if app.kube_filter_input.is_empty() {
            "  No nodes."
        } else {
            "  No nodes match the filter."
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
            ("NAME", KubeSortField::NodeName),
            ("STATUS", KubeSortField::NodeName), // status sort intentionally tied to name
            ("ROLES", KubeSortField::NodeName),
            ("AGE", KubeSortField::NodeAge),
            ("VERSION", KubeSortField::NodeName),
            ("CPU%", KubeSortField::NodeCpuPct),
            ("MEM%", KubeSortField::NodeMemPct),
            ("PODS", KubeSortField::NodePodCount),
        ],
        app,
        theme,
    );

    let visible = nodes
        .iter()
        .skip(app.kube_scroll_offset)
        .take(area.height.saturating_sub(1) as usize);

    let rows = visible.enumerate().map(|(i, n)| {
        let status_style = node_status_style(n.status, theme);
        let absolute_idx = app.kube_scroll_offset + i;
        let row_style = row_selection_style(absolute_idx == app.kube_selected, theme);
        Row::new(vec![
            Cell::from(scrub_ctrl(&n.name).into_owned()),
            Cell::from(node_status_label(n.status)).style(status_style),
            Cell::from(n.roles.join(",")),
            Cell::from(format_age(n.age_seconds)),
            Cell::from(scrub_ctrl(&n.kubelet_version).into_owned()),
            Cell::from(format_pct(n.cpu_used_millis, n.cpu_allocatable_millis)),
            Cell::from(format_pct_u64(n.mem_used_bytes, n.mem_allocatable_bytes)),
            Cell::from(format!("{}/{}", n.pod_count, n.pod_capacity)),
        ])
        .style(row_style)
    });

    let widths = [
        Constraint::Length(30),
        Constraint::Length(20),
        Constraint::Length(20),
        Constraint::Length(8),
        Constraint::Length(15),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(10),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, area);
}

fn node_matches(n: &NodeSnapshot, needle_lower: &str) -> bool {
    contains_ignore_case(&n.name, needle_lower)
}

fn cmp_nodes(
    a: &NodeSnapshot,
    b: &NodeSnapshot,
    field: KubeSortField,
    order: SortOrder,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ord = match field {
        KubeSortField::NodeName => a.name.cmp(&b.name),
        KubeSortField::NodeAge => a.age_seconds.cmp(&b.age_seconds).reverse(),
        KubeSortField::NodePodCount => a.pod_count.cmp(&b.pod_count).reverse(),
        KubeSortField::NodeCpuPct => pct_for(a.cpu_used_millis, a.cpu_allocatable_millis)
            .partial_cmp(&pct_for(b.cpu_used_millis, b.cpu_allocatable_millis))
            .unwrap_or(Ordering::Equal)
            .reverse(),
        KubeSortField::NodeMemPct => pct_for_u64(a.mem_used_bytes, a.mem_allocatable_bytes)
            .partial_cmp(&pct_for_u64(b.mem_used_bytes, b.mem_allocatable_bytes))
            .unwrap_or(Ordering::Equal)
            .reverse(),
        _ => Ordering::Equal,
    };
    if matches!(order, SortOrder::Asc) {
        ord.reverse()
    } else {
        ord
    }
}

fn pct_for(used: Option<u32>, total: u32) -> f32 {
    let Some(u) = used else { return 0.0 };
    if total == 0 {
        0.0
    } else {
        (u as f32 / total as f32) * 100.0
    }
}

fn pct_for_u64(used: Option<u64>, total: u64) -> f32 {
    let Some(u) = used else { return 0.0 };
    if total == 0 {
        0.0
    } else {
        (u as f64 / total as f64 * 100.0) as f32
    }
}

fn format_pct(used: Option<u32>, total: u32) -> String {
    let Some(_) = used else { return "—".into() };
    let pct = pct_for(used, total);
    format!("{pct:.0}%")
}

fn format_pct_u64(used: Option<u64>, total: u64) -> String {
    let Some(_) = used else { return "—".into() };
    let pct = pct_for_u64(used, total);
    format!("{pct:.0}%")
}

fn node_status_label(s: NodeStatus) -> &'static str {
    match s {
        NodeStatus::Ready => "Ready",
        NodeStatus::NotReady => "NotReady",
        NodeStatus::SchedulingDisabled => "SchedDisabled",
        NodeStatus::Unknown => "Unknown",
    }
}

fn node_status_style(s: NodeStatus, theme: &Theme) -> Style {
    match s {
        NodeStatus::Ready => Style::default().fg(theme.success),
        NodeStatus::NotReady => Style::default().fg(theme.danger),
        NodeStatus::SchedulingDisabled => Style::default().fg(theme.warning),
        NodeStatus::Unknown => Style::default().fg(theme.text_dim),
    }
}

// ---- Deployments sub-view ----------------------------------------------

fn draw_deployments(
    frame: &mut Frame,
    area: Rect,
    app: &AppState,
    theme: &Theme,
    snap: &KubeSnapshot,
) {
    let deployments: Vec<&DeploymentSnapshot> = app
        .kube_view_cache
        .iter()
        .filter_map(|&i| snap.deployments.get(i))
        .collect();

    if deployments.is_empty() {
        let msg = if app.kube_filter_input.is_empty() {
            "  No deployments."
        } else {
            "  No deployments match the filter."
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
            ("NAMESPACE", KubeSortField::DeployNamespace),
            ("NAME", KubeSortField::DeployName),
            ("READY", KubeSortField::DeployReadyRatio),
            ("UP-TO-DATE", KubeSortField::DeployReadyRatio),
            ("AVAILABLE", KubeSortField::DeployReadyRatio),
            ("AGE", KubeSortField::DeployAge),
        ],
        app,
        theme,
    );

    let visible = deployments
        .iter()
        .skip(app.kube_scroll_offset)
        .take(area.height.saturating_sub(1) as usize);

    let rows = visible.enumerate().map(|(i, d)| {
        let ready_style = deployment_ready_style(d, theme);
        let absolute_idx = app.kube_scroll_offset + i;
        let row_style = row_selection_style(absolute_idx == app.kube_selected, theme);
        Row::new(vec![
            Cell::from(scrub_ctrl(&d.namespace).into_owned()),
            Cell::from(scrub_ctrl(&d.name).into_owned()),
            Cell::from(format!("{}/{}", d.replicas_ready, d.replicas_desired)).style(ready_style),
            Cell::from(d.replicas_uptodate.to_string()),
            Cell::from(d.replicas_available.to_string()),
            Cell::from(format_age(d.age_seconds)),
        ])
        .style(row_style)
    });

    let widths = [
        Constraint::Length(20),
        Constraint::Length(40),
        Constraint::Length(8),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths).header(header);
    frame.render_widget(table, area);
}

fn deployment_matches(d: &DeploymentSnapshot, needle_lower: &str) -> bool {
    contains_ignore_case(&d.name, needle_lower) || contains_ignore_case(&d.namespace, needle_lower)
}

fn cmp_deployments(
    a: &DeploymentSnapshot,
    b: &DeploymentSnapshot,
    field: KubeSortField,
    order: SortOrder,
) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ord = match field {
        KubeSortField::DeployName => a.name.cmp(&b.name),
        KubeSortField::DeployNamespace => a
            .namespace
            .cmp(&b.namespace)
            .then_with(|| a.name.cmp(&b.name)),
        KubeSortField::DeployAge => a.age_seconds.cmp(&b.age_seconds).reverse(),
        KubeSortField::DeployReadyRatio => {
            let ratio = |x: &DeploymentSnapshot| -> f32 {
                if x.replicas_desired == 0 {
                    1.0
                } else {
                    x.replicas_ready as f32 / x.replicas_desired as f32
                }
            };
            ratio(a).partial_cmp(&ratio(b)).unwrap_or(Ordering::Equal)
        }
        _ => Ordering::Equal,
    };
    if matches!(order, SortOrder::Asc) {
        ord.reverse()
    } else {
        ord
    }
}

/// Indices into the active sub-view's list, filtered and sorted (PERF-03).
///
/// Returning indices rather than clones keeps the cache proportional to the
/// row count instead of the row *size*, and lets the render path borrow
/// straight from the snapshot.
///
/// The filter needle is lowercased once here; per-row matching goes through
/// `contains_ignore_case`, whose ASCII fast path allocates nothing. The
/// previous code lowercased every name and namespace on every render — at
/// the 5 000-pod ceiling that was 10 000 transient Strings per frame, which
/// is exactly what PERF-M2 removed from the other tabs in v0.3.1 and never
/// reached the Kube tab.
pub(crate) fn compute_view_indices(
    snap: &KubeSnapshot,
    subview: KubeSubview,
    filter: &str,
    sort_field: KubeSortField,
    sort_order: SortOrder,
) -> Vec<usize> {
    let needle = filter.to_lowercase();

    match subview {
        KubeSubview::Pods => {
            let mut idx: Vec<usize> = (0..snap.pods.len())
                .filter(|&i| pod_matches(&snap.pods[i], &needle))
                .collect();
            idx.sort_by(|&a, &b| cmp_pods(&snap.pods[a], &snap.pods[b], sort_field, sort_order));
            idx
        }
        KubeSubview::Nodes => {
            let mut idx: Vec<usize> = (0..snap.nodes.len())
                .filter(|&i| node_matches(&snap.nodes[i], &needle))
                .collect();
            idx.sort_by(|&a, &b| cmp_nodes(&snap.nodes[a], &snap.nodes[b], sort_field, sort_order));
            idx
        }
        KubeSubview::Deployments => {
            let mut idx: Vec<usize> = (0..snap.deployments.len())
                .filter(|&i| deployment_matches(&snap.deployments[i], &needle))
                .collect();
            idx.sort_by(|&a, &b| {
                cmp_deployments(
                    &snap.deployments[a],
                    &snap.deployments[b],
                    sort_field,
                    sort_order,
                )
            });
            idx
        }
    }
}

fn deployment_ready_style(d: &DeploymentSnapshot, theme: &Theme) -> Style {
    if d.replicas_desired == 0 {
        return Style::default().fg(theme.text_dim);
    }
    if d.replicas_available == 0 {
        Style::default().fg(theme.danger)
    } else if d.replicas_ready == d.replicas_desired {
        Style::default().fg(theme.success)
    } else {
        Style::default().fg(theme.warning)
    }
}

// ---- Shared helpers -----------------------------------------------------

fn make_header<'a>(cols: &'a [(&'a str, KubeSortField)], app: &AppState, theme: &Theme) -> Row<'a> {
    let cells: Vec<Cell<'a>> = cols
        .iter()
        .map(|(label, field)| {
            let mut text = (*label).to_string();
            if app.kube_sort_field == *field {
                text.push(' ');
                text.push(match app.kube_sort_order {
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
    use muxtop_core::kube::{DeploymentStrategy, KubeSnapshot, NodeStatus, PodSnapshot, QosClass};

    // ---- format helpers ----

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
        assert_eq!(format_mem(Some(512)), "0K");
        assert_eq!(format_mem(Some(2048)), "2K");
        assert_eq!(format_mem(Some(1024 * 1024)), "1.0M");
        assert_eq!(format_mem(Some(2 * 1024 * 1024 * 1024)), "2.0G");
    }

    #[test]
    fn format_pct_zero_total_is_zero() {
        assert_eq!(format_pct(Some(100), 0), "0%");
    }

    #[test]
    fn format_pct_none_used_is_dash() {
        assert_eq!(format_pct(None, 1000), "—");
        assert_eq!(format_pct_u64(None, 1024), "—");
    }

    #[test]
    fn format_pct_typical() {
        assert_eq!(format_pct(Some(500), 1000), "50%");
        assert_eq!(
            format_pct_u64(Some(2 * 1024 * 1024 * 1024), 8 * 1024 * 1024 * 1024),
            "25%"
        );
    }

    // ---- exhaustive label paths ----

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
            let _ = pod_phase_rank(p);
        }
    }

    #[test]
    fn node_status_label_is_exhaustive() {
        for s in [
            NodeStatus::Ready,
            NodeStatus::NotReady,
            NodeStatus::SchedulingDisabled,
            NodeStatus::Unknown,
        ] {
            let _ = node_status_label(s);
            let _ = node_status_style(s, &Theme::default());
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

    // ---- filter ----

    fn make_pods() -> Vec<PodSnapshot> {
        vec![
            sample_pod("default", "nginx-1", PodPhase::Running, 100, 64),
            sample_pod("kube-system", "coredns", PodPhase::Running, 50, 32),
            sample_pod("default", "redis-0", PodPhase::CrashLoop, 0, 16),
        ]
    }

    fn sample_pod(ns: &str, name: &str, phase: PodPhase, cpu: u32, mem_mib: u64) -> PodSnapshot {
        PodSnapshot {
            namespace: ns.into(),
            name: name.into(),
            phase,
            ready: (1, 1),
            restarts: 0,
            age_seconds: 3600,
            node: "node-1".into(),
            cpu_millis: Some(cpu),
            mem_bytes: Some(mem_mib * 1024 * 1024),
            qos: QosClass::Burstable,
        }
    }

    /// Drive the pipeline the render path actually reads, so these tests
    /// exercise the cached ordering rather than a parallel helper.
    fn view(
        pods: &[PodSnapshot],
        filter: &str,
        field: KubeSortField,
        order: SortOrder,
    ) -> Vec<String> {
        let snap = KubeSnapshot {
            cluster_kind: ClusterKind::Generic,
            server_version: None,
            current_namespace: "default".into(),
            reachable: true,
            metrics_available: true,
            pods: pods.to_vec(),
            nodes: Vec::new(),
            deployments: Vec::new(),
        };
        compute_view_indices(&snap, KubeSubview::Pods, filter, field, order)
            .into_iter()
            .map(|i| snap.pods[i].name.clone())
            .collect()
    }

    #[test]
    fn filter_pods_by_name() {
        let names = view(
            &make_pods(),
            "nginx",
            KubeSortField::PodName,
            SortOrder::Desc,
        );
        assert_eq!(names, vec!["nginx-1"]);
    }

    #[test]
    fn filter_pods_by_namespace() {
        let names = view(
            &make_pods(),
            "kube-system",
            KubeSortField::PodName,
            SortOrder::Desc,
        );
        assert_eq!(names, vec!["coredns"]);
    }

    #[test]
    fn filter_pods_empty_returns_all() {
        assert_eq!(
            view(&make_pods(), "", KubeSortField::PodName, SortOrder::Desc).len(),
            3
        );
    }

    #[test]
    fn filter_pods_no_match() {
        assert!(view(&make_pods(), "zzz", KubeSortField::PodName, SortOrder::Desc).is_empty());
    }

    /// PERF-03 swapped `to_lowercase()`-per-row for `contains_ignore_case`;
    /// the filter must stay case-insensitive across that change.
    #[test]
    fn filter_pods_is_case_insensitive() {
        let names = view(
            &make_pods(),
            "NGINX",
            KubeSortField::PodName,
            SortOrder::Desc,
        );
        assert_eq!(names, vec!["nginx-1"]);
    }

    // ---- sort ----

    #[test]
    fn sort_pods_by_cpu_desc() {
        // CPU values: nginx=100, coredns=50, redis=0
        let names = view(&make_pods(), "", KubeSortField::PodCpu, SortOrder::Desc);
        assert_eq!(names, vec!["nginx-1", "coredns", "redis-0"]);
    }

    #[test]
    fn sort_pods_by_phase_puts_crashloop_first() {
        let names = view(&make_pods(), "", KubeSortField::PodPhase, SortOrder::Desc);
        assert_eq!(names[0], "redis-0", "redis-0 is the CrashLoop pod");
    }

    #[test]
    fn sort_pods_asc_inverts_desc() {
        let pods = make_pods();
        let desc = view(&pods, "", KubeSortField::PodCpu, SortOrder::Desc);
        let asc = view(&pods, "", KubeSortField::PodCpu, SortOrder::Asc);
        assert_eq!(desc[0], asc[2]);
        assert_eq!(desc[2], asc[0]);
    }

    // ---- smoke renders ----

    fn populated_snapshot() -> KubeSnapshot {
        KubeSnapshot {
            cluster_kind: ClusterKind::Kind,
            server_version: Some("v1.31.0".into()),
            current_namespace: "default".into(),
            reachable: true,
            metrics_available: true,
            pods: make_pods(),
            nodes: vec![NodeSnapshot {
                name: "node-1".into(),
                status: NodeStatus::Ready,
                roles: vec!["control-plane".into()],
                age_seconds: 86_400,
                kubelet_version: "v1.31.0".into(),
                cpu_capacity_millis: 4_000,
                cpu_allocatable_millis: 3_800,
                cpu_used_millis: Some(1_900),
                mem_capacity_bytes: 8 * 1024 * 1024 * 1024,
                mem_allocatable_bytes: 7_900 * 1024 * 1024,
                mem_used_bytes: Some(2 * 1024 * 1024 * 1024),
                pod_count: 12,
                pod_capacity: 110,
            }],
            deployments: vec![DeploymentSnapshot {
                namespace: "default".into(),
                name: "nginx".into(),
                replicas_desired: 3,
                replicas_ready: 3,
                replicas_uptodate: 3,
                replicas_available: 3,
                age_seconds: 3600,
                strategy: DeploymentStrategy::RollingUpdate,
            }],
        }
    }

    #[test]
    fn smoke_render_unreachable_does_not_panic() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 20);
        let mut term = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        term.draw(|f| {
            let snap = KubeSnapshot::unavailable();
            draw_unreachable(f, f.area(), &theme, &snap);
        })
        .unwrap();
    }

    fn render_subview(sv: KubeSubview) {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(140, 30);
        let mut term = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        let mut app = AppState::new();
        app.switch_kube_subview(sv);
        let snap = populated_snapshot();
        // Mirror what `AppState::recompute_kube_view` does — the render path
        // reads this cache, so a smoke test that left it empty would render
        // the "no rows" branch and prove nothing.
        app.kube_view_cache = compute_view_indices(
            &snap,
            sv,
            &app.kube_filter_input,
            app.kube_sort_field,
            app.kube_sort_order,
        );
        assert!(
            !app.kube_view_cache.is_empty(),
            "fixture should produce rows for {sv:?}"
        );
        term.draw(|f| draw_active(f, f.area(), &app, &theme, &snap))
            .unwrap();
    }

    #[test]
    fn smoke_render_pods_subview() {
        render_subview(KubeSubview::Pods);
    }

    #[test]
    fn smoke_render_nodes_subview() {
        render_subview(KubeSubview::Nodes);
    }

    #[test]
    fn smoke_render_deployments_subview() {
        render_subview(KubeSubview::Deployments);
    }

    /// SEC-03: in remote mode the namespace comes from the server's
    /// kubeconfig, so the summary bar needs the same scrubbing the table
    /// cells already got.
    #[test]
    fn summary_scrubs_hostile_namespace() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(140, 3);
        let mut term = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        let mut snap = populated_snapshot();
        snap.current_namespace = "kube\x1b[2Jsystem".into();

        term.draw(|f| draw_summary(f, f.area(), &theme, &snap))
            .unwrap();

        let buf = term.backend().buffer().clone();
        let rendered: String = (0..buf.area.width)
            .map(|col| buf.cell((col, 0)).map(|c| c.symbol()).unwrap_or(" "))
            .collect();

        assert!(
            !rendered.contains('\x1b'),
            "ESC survived into the summary bar: {rendered:?}"
        );
        assert!(
            rendered.contains("kube?"),
            "the printable part of the namespace should still render: {rendered:?}"
        );
    }

    #[test]
    fn smoke_render_with_filter_active() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(140, 30);
        let mut term = Terminal::new(backend).unwrap();
        let theme = Theme::default();
        let mut app = AppState::new();
        app.kube_filter_active = true;
        app.kube_filter_input = "nginx".into();
        let snap = populated_snapshot();
        app.kube_view_cache = compute_view_indices(
            &snap,
            app.kube_subview,
            &app.kube_filter_input,
            app.kube_sort_field,
            app.kube_sort_order,
        );
        term.draw(|f| draw_active(f, f.area(), &app, &theme, &snap))
            .unwrap();
    }
}
