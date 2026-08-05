// Kubernetes tab — read-only Pods / Nodes / Deployments.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::Render;
use super::filter_bar;
use super::inspector::format_age;
use super::sanitize::scrub_ctrl;
use super::widgets::columns::{Align, Column, PRIO_ESSENTIAL, PRIO_HIGH, PRIO_LOW, PRIO_MEDIUM};
use super::widgets::empty::{self, EmptyState};
use super::widgets::table::{self, Cell, Row, Spec};
use super::widgets::{badge, meter};
use crate::app::{KubeSortField, KubeSubview};
use crate::ui::theme::Level;
use muxtop_core::kube::{
    DeploymentSnapshot, KubeSnapshot, NodeSnapshot, NodeStatus, PodPhase, PodSnapshot,
};
use muxtop_core::process::SortOrder;

const POD_COLUMNS: &[Column] = &[
    Column::fixed("NAMESPACE", 18, Align::Left, PRIO_MEDIUM),
    Column::flex("POD", 20, PRIO_ESSENTIAL),
    Column::fixed("PHASE", 15, Align::Left, PRIO_ESSENTIAL),
    Column::fixed("READY", 8, Align::Right, PRIO_HIGH),
    Column::fixed("RESTARTS", 10, Align::Right, PRIO_HIGH),
    Column::fixed("CPU", 8, Align::Right, PRIO_LOW),
    Column::fixed("MEM", 9, Align::Right, PRIO_LOW),
    Column::fixed("AGE", 7, Align::Right, PRIO_MEDIUM),
];

const NODE_COLUMNS: &[Column] = &[
    Column::flex("NODE", 20, PRIO_ESSENTIAL),
    Column::fixed("STATUS", 20, Align::Left, PRIO_ESSENTIAL),
    Column::fixed("ROLES", 16, Align::Left, PRIO_LOW),
    Column::fixed("CPU", 9, Align::Right, PRIO_HIGH),
    Column::fixed("MEM", 9, Align::Right, PRIO_HIGH),
    Column::fixed("PODS", 9, Align::Right, PRIO_MEDIUM),
    Column::fixed("VERSION", 14, Align::Left, PRIO_LOW),
    Column::fixed("AGE", 7, Align::Right, PRIO_MEDIUM),
];

const DEPLOY_COLUMNS: &[Column] = &[
    Column::fixed("NAMESPACE", 18, Align::Left, PRIO_MEDIUM),
    Column::flex("DEPLOYMENT", 20, PRIO_ESSENTIAL),
    Column::fixed("READY", 9, Align::Right, PRIO_ESSENTIAL),
    Column::fixed("UP-TO-DATE", 12, Align::Right, PRIO_HIGH),
    Column::fixed("AVAILABLE", 11, Align::Right, PRIO_HIGH),
    Column::fixed("STRATEGY", 15, Align::Left, PRIO_LOW),
    Column::fixed("AGE", 7, Align::Right, PRIO_MEDIUM),
];

pub fn draw_kube_tab(frame: &mut Frame, area: Rect, r: &Render<'_>) {
    let app = r.app;
    let Some(snap) = app.last_snapshot.as_ref().and_then(|s| s.kube.as_ref()) else {
        empty::render(
            frame,
            area,
            &EmptyState::waiting(&r.ellipsis("Connecting to the cluster")),
            r.theme,
        );
        return;
    };

    if !snap.reachable {
        empty::render(
            frame,
            area,
            &EmptyState::error(
                "No cluster reachable",
                "muxtop found no usable kubeconfig or in-cluster credentials.",
                "Check $KUBECONFIG, or run `kubectl config use-context <name>`.",
            ),
            r.theme,
        );
        return;
    }

    let filter_h = u16::from(app.filter_editing());
    let [context_area, subtab_area, table_area, filter_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(filter_h),
    ])
    .areas(area);

    draw_context(frame, context_area, r, snap);
    draw_subtabs(frame, subtab_area, r, snap);

    match app.kube_subview {
        KubeSubview::Pods => draw_pods(frame, table_area, r, snap),
        KubeSubview::Nodes => draw_nodes(frame, table_area, r, snap),
        KubeSubview::Deployments => draw_deployments(frame, table_area, r, snap),
    }

    if filter_h > 0 {
        filter_bar::draw(frame, filter_area, r, "Filter cluster");
    }
}

/// Cluster identity and — crucially — the active namespace scope, which in 0.4
/// was invisible until you pressed `A` and read the status message.
fn draw_context(frame: &mut Frame, area: Rect, r: &Render<'_>, snap: &KubeSnapshot) {
    let mut spans = vec![
        Span::styled(format!(" {:?} ", snap.cluster_kind), r.theme.accent_fill()),
        Span::styled("  ", r.theme.body()),
    ];
    spans.extend(badge::chip(
        "ns",
        &scrub_ctrl(&snap.current_namespace),
        r.theme,
    ));
    if let Some(version) = snap.server_version.as_ref() {
        spans.push(Span::styled(
            format!("   {}", scrub_ctrl(version)),
            r.theme.subtle(),
        ));
    }
    if !snap.metrics_available {
        spans.push(Span::styled(
            "   no metrics-server ",
            r.theme.level_style(Level::Warning),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Sub-view selector, rendered as real tabs with counts.
fn draw_subtabs(frame: &mut Frame, area: Rect, r: &Render<'_>, snap: &KubeSnapshot) {
    let counts = [
        (KubeSubview::Pods, "P", snap.pods.len()),
        (KubeSubview::Nodes, "N", snap.nodes.len()),
        (KubeSubview::Deployments, "D", snap.deployments.len()),
    ];
    let mut spans = Vec::with_capacity(counts.len() * 3);
    for (sv, key, count) in counts {
        let active = sv == r.app.kube_subview;
        spans.push(Span::styled(
            format!(" {key} "),
            if active {
                r.theme.key()
            } else {
                r.theme.subtle()
            },
        ));
        spans.push(Span::styled(
            format!("{} {count}  ", sv.label()),
            if active {
                r.theme.accent()
            } else {
                r.theme.dim()
            },
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

// ---------------------------------------------------------------------------
// Pods
// ---------------------------------------------------------------------------

fn draw_pods(frame: &mut Frame, area: Rect, r: &Render<'_>, snap: &KubeSnapshot) {
    let app = r.app;
    let f = app.kube_filter_input.to_lowercase();
    let mut pods: Vec<&PodSnapshot> = snap
        .pods
        .iter()
        .filter(|p| {
            f.is_empty()
                || p.name.to_lowercase().contains(&f)
                || p.namespace.to_lowercase().contains(&f)
        })
        .collect();
    sort_pods(&mut pods, app.kube_sort_field, app.kube_sort_order);

    let spec = Spec {
        columns: POD_COLUMNS,
        sort_col: pod_sort_column(app.kube_sort_field),
        descending: matches!(app.kube_sort_order, SortOrder::Desc),
        total: pods.len(),
        selected: app.kube_selected,
        scroll: app.kube_scroll_offset,
        col_scroll: app.col_scroll,
        empty: if f.is_empty() {
            EmptyState::empty("No pods", Some("Nothing is scheduled in this scope."))
        } else {
            EmptyState::no_match("No matching pods")
        },
    };

    table::draw(frame, area, r, &spec, |idx| match pods.get(idx) {
        Some(p) => pod_row(p, r),
        None => Row::new(Vec::new()),
    });
}

fn pod_row(p: &PodSnapshot, r: &Render<'_>) -> Row {
    let level = phase_level(p.phase);
    let dash = r.glyphs.none.to_string();
    let ready_ok = p.ready.0 == p.ready.1 && p.ready.1 > 0;

    Row::new(vec![
        Cell::new(scrub_ctrl(&p.namespace).into_owned()),
        Cell::new(scrub_ctrl(&p.name).into_owned()),
        Cell::colored(
            format!(
                "{} {}",
                badge::marker(level, r.glyphs),
                phase_label(p.phase)
            ),
            r.theme.level_color(level),
        ),
        if ready_ok {
            Cell::new(format!("{}/{}", p.ready.0, p.ready.1))
        } else {
            Cell::colored(format!("{}/{}", p.ready.0, p.ready.1), r.theme.warning)
        },
        if p.restarts > 0 {
            Cell::colored(p.restarts.to_string(), r.theme.warning)
        } else {
            Cell::new("0")
        },
        // A missing metric renders as a dash, not as zero: "no metrics-server"
        // and "idle" are very different statements.
        p.cpu_millis
            .map_or_else(|| Cell::new(dash.clone()), |m| Cell::new(format!("{m}m"))),
        p.mem_bytes.map_or_else(
            || Cell::new(dash.clone()),
            |b| Cell::new(meter::human_bytes(b)),
        ),
        Cell::new(format_age(p.age_seconds)),
    ])
}

fn phase_level(phase: PodPhase) -> Level {
    match phase {
        PodPhase::Running => Level::Success,
        PodPhase::Succeeded => Level::Neutral,
        PodPhase::Pending | PodPhase::Terminating => Level::Info,
        PodPhase::Failed | PodPhase::CrashLoop => Level::Error,
        PodPhase::Unknown => Level::Warning,
    }
}

fn phase_label(phase: PodPhase) -> &'static str {
    match phase {
        PodPhase::Pending => "Pending",
        PodPhase::Running => "Running",
        PodPhase::Succeeded => "Succeeded",
        PodPhase::Failed => "Failed",
        PodPhase::CrashLoop => "CrashLoop",
        PodPhase::Terminating => "Terminating",
        PodPhase::Unknown => "Unknown",
    }
}

fn pod_sort_column(field: KubeSortField) -> Option<usize> {
    Some(match field {
        KubeSortField::PodName => 1,
        KubeSortField::PodPhase => 2,
        KubeSortField::PodRestarts => 4,
        KubeSortField::PodCpu => 5,
        KubeSortField::PodMem => 6,
        KubeSortField::PodAge => 7,
        _ => return None,
    })
}

fn sort_pods(pods: &mut [&PodSnapshot], field: KubeSortField, order: SortOrder) {
    match field {
        KubeSortField::PodName => {
            pods.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)))
        }
        KubeSortField::PodPhase => pods.sort_by_key(|p| phase_rank(p.phase)),
        KubeSortField::PodRestarts => pods.sort_by_key(|p| std::cmp::Reverse(p.restarts)),
        KubeSortField::PodCpu => pods.sort_by_key(|p| std::cmp::Reverse(p.cpu_millis.unwrap_or(0))),
        KubeSortField::PodMem => pods.sort_by_key(|p| std::cmp::Reverse(p.mem_bytes.unwrap_or(0))),
        KubeSortField::PodAge => pods.sort_by_key(|p| std::cmp::Reverse(p.age_seconds)),
        _ => pods.sort_by_key(|p| std::cmp::Reverse(p.cpu_millis.unwrap_or(0))),
    }
    if matches!(order, SortOrder::Asc) {
        pods.reverse();
    }
}

/// Sort key that puts what is broken first — a pod in CrashLoopBackOff is the
/// reason you opened this tab.
fn phase_rank(phase: PodPhase) -> u8 {
    match phase {
        PodPhase::CrashLoop => 0,
        PodPhase::Failed => 1,
        PodPhase::Unknown => 2,
        PodPhase::Pending => 3,
        PodPhase::Terminating => 4,
        PodPhase::Running => 5,
        PodPhase::Succeeded => 6,
    }
}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

fn draw_nodes(frame: &mut Frame, area: Rect, r: &Render<'_>, snap: &KubeSnapshot) {
    let app = r.app;
    let f = app.kube_filter_input.to_lowercase();
    let mut nodes: Vec<&NodeSnapshot> = snap
        .nodes
        .iter()
        .filter(|n| f.is_empty() || n.name.to_lowercase().contains(&f))
        .collect();
    sort_nodes(&mut nodes, app.kube_sort_field, app.kube_sort_order);

    // Nodes are a cluster-scoped resource: there is no namespaced variant, so
    // an empty list under a namespace scope means missing permissions, not an
    // empty cluster. Saying just "No nodes" would be misleading.
    let empty_state = if !f.is_empty() {
        EmptyState::no_match("No matching nodes")
    } else if snap.current_namespace.is_empty() || snap.current_namespace == "all" {
        EmptyState::empty("No nodes", None)
    } else {
        EmptyState::error(
            "No nodes visible",
            "Nodes are cluster-scoped: there is no namespaced variant.",
            "Listing them needs cluster-wide access; Pods and Deployments are unaffected.",
        )
    };

    let spec = Spec {
        columns: NODE_COLUMNS,
        sort_col: node_sort_column(app.kube_sort_field),
        descending: matches!(app.kube_sort_order, SortOrder::Desc),
        total: nodes.len(),
        selected: app.kube_selected,
        scroll: app.kube_scroll_offset,
        col_scroll: app.col_scroll,
        empty: empty_state,
    };

    table::draw(frame, area, r, &spec, |idx| match nodes.get(idx) {
        Some(n) => node_row(n, r),
        None => Row::new(Vec::new()),
    });
}

fn node_row(n: &NodeSnapshot, r: &Render<'_>) -> Row {
    let level = match n.status {
        NodeStatus::Ready => Level::Success,
        NodeStatus::SchedulingDisabled => Level::Warning,
        NodeStatus::NotReady => Level::Error,
        NodeStatus::Unknown => Level::Warning,
    };
    let dash = r.glyphs.none.to_string();

    let cpu = n.cpu_used_millis.map(|used| {
        let pct = if n.cpu_allocatable_millis > 0 {
            f64::from(used) / f64::from(n.cpu_allocatable_millis) * 100.0
        } else {
            0.0
        };
        (format!("{pct:.0}%"), pct)
    });
    let mem = n.mem_used_bytes.map(|used| {
        let pct = if n.mem_allocatable_bytes > 0 {
            used as f64 / n.mem_allocatable_bytes as f64 * 100.0
        } else {
            0.0
        };
        (format!("{pct:.0}%"), pct)
    });

    Row::new(vec![
        Cell::new(scrub_ctrl(&n.name).into_owned()),
        Cell::colored(
            format!("{} {:?}", badge::marker(level, r.glyphs), n.status),
            r.theme.level_color(level),
        ),
        Cell::new(if n.roles.is_empty() {
            dash.clone()
        } else {
            n.roles.join(",")
        }),
        cpu.map_or_else(
            || Cell::new(dash.clone()),
            |(label, pct)| Cell::colored(label, r.theme.gauge_color(pct)),
        ),
        mem.map_or_else(
            || Cell::new(dash.clone()),
            |(label, pct)| Cell::colored(label, r.theme.gauge_color(pct)),
        ),
        Cell::new(format!("{}/{}", n.pod_count, n.pod_capacity)),
        Cell::new(scrub_ctrl(&n.kubelet_version).into_owned()),
        Cell::new(format_age(n.age_seconds)),
    ])
}

fn node_sort_column(field: KubeSortField) -> Option<usize> {
    Some(match field {
        KubeSortField::NodeName => 0,
        KubeSortField::NodeCpuPct => 3,
        KubeSortField::NodeMemPct => 4,
        KubeSortField::NodePodCount => 5,
        KubeSortField::NodeAge => 7,
        _ => return None,
    })
}

fn sort_nodes(nodes: &mut [&NodeSnapshot], field: KubeSortField, order: SortOrder) {
    match field {
        KubeSortField::NodeName => nodes.sort_by(|a, b| a.name.cmp(&b.name)),
        KubeSortField::NodeMemPct => {
            nodes.sort_by_key(|n| std::cmp::Reverse(n.mem_used_bytes.unwrap_or(0)))
        }
        KubeSortField::NodePodCount => nodes.sort_by_key(|n| std::cmp::Reverse(n.pod_count)),
        KubeSortField::NodeAge => nodes.sort_by_key(|n| std::cmp::Reverse(n.age_seconds)),
        _ => nodes.sort_by_key(|n| std::cmp::Reverse(n.cpu_used_millis.unwrap_or(0))),
    }
    if matches!(order, SortOrder::Asc) {
        nodes.reverse();
    }
}

// ---------------------------------------------------------------------------
// Deployments
// ---------------------------------------------------------------------------

fn draw_deployments(frame: &mut Frame, area: Rect, r: &Render<'_>, snap: &KubeSnapshot) {
    let app = r.app;
    let f = app.kube_filter_input.to_lowercase();
    let mut deploys: Vec<&DeploymentSnapshot> = snap
        .deployments
        .iter()
        .filter(|d| {
            f.is_empty()
                || d.name.to_lowercase().contains(&f)
                || d.namespace.to_lowercase().contains(&f)
        })
        .collect();
    sort_deployments(&mut deploys, app.kube_sort_field, app.kube_sort_order);

    let spec = Spec {
        columns: DEPLOY_COLUMNS,
        sort_col: deploy_sort_column(app.kube_sort_field),
        descending: matches!(app.kube_sort_order, SortOrder::Desc),
        total: deploys.len(),
        selected: app.kube_selected,
        scroll: app.kube_scroll_offset,
        col_scroll: app.col_scroll,
        empty: if f.is_empty() {
            EmptyState::empty("No deployments", None)
        } else {
            EmptyState::no_match("No matching deployments")
        },
    };

    table::draw(frame, area, r, &spec, |idx| match deploys.get(idx) {
        Some(d) => deploy_row(d, r),
        None => Row::new(Vec::new()),
    });
}

fn deploy_row(d: &DeploymentSnapshot, r: &Render<'_>) -> Row {
    let healthy = d.replicas_ready == d.replicas_desired;
    let ready = format!("{}/{}", d.replicas_ready, d.replicas_desired);

    Row::new(vec![
        Cell::new(scrub_ctrl(&d.namespace).into_owned()),
        Cell::new(scrub_ctrl(&d.name).into_owned()),
        if healthy {
            Cell::colored(ready, r.theme.success)
        } else {
            Cell::colored(ready, r.theme.warning)
        },
        Cell::new(d.replicas_uptodate.to_string()),
        Cell::new(d.replicas_available.to_string()),
        Cell::new(format!("{:?}", d.strategy)),
        Cell::new(format_age(d.age_seconds)),
    ])
}

fn deploy_sort_column(field: KubeSortField) -> Option<usize> {
    Some(match field {
        KubeSortField::DeployNamespace => 0,
        KubeSortField::DeployName => 1,
        KubeSortField::DeployReadyRatio => 2,
        KubeSortField::DeployAge => 6,
        _ => return None,
    })
}

fn sort_deployments(deploys: &mut [&DeploymentSnapshot], field: KubeSortField, order: SortOrder) {
    match field {
        KubeSortField::DeployNamespace => {
            deploys.sort_by(|a, b| (&a.namespace, &a.name).cmp(&(&b.namespace, &b.name)))
        }
        KubeSortField::DeployReadyRatio => deploys.sort_by(|a, b| {
            ratio(a)
                .partial_cmp(&ratio(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        KubeSortField::DeployAge => deploys.sort_by_key(|d| std::cmp::Reverse(d.age_seconds)),
        _ => deploys.sort_by(|a, b| a.name.cmp(&b.name)),
    }
    if matches!(order, SortOrder::Asc) {
        deploys.reverse();
    }
}

fn ratio(d: &DeploymentSnapshot) -> f64 {
    if d.replicas_desired == 0 {
        1.0
    } else {
        f64::from(d.replicas_ready) / f64::from(d.replicas_desired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, Tab};
    use crate::ui::test_support::*;
    use muxtop_core::kube::{ClusterKind, DeploymentStrategy, QosClass};

    fn kube_app(reachable: bool, metrics: bool, namespace: &str) -> AppState {
        let pods = vec![
            PodSnapshot {
                namespace: "default".into(),
                name: "api-7d9f".into(),
                phase: PodPhase::Running,
                ready: (1, 1),
                restarts: 0,
                age_seconds: 7200,
                node: "node-1".into(),
                cpu_millis: metrics.then_some(120),
                mem_bytes: metrics.then_some(256 * 1024 * 1024),
                qos: QosClass::Burstable,
            },
            PodSnapshot {
                namespace: "default".into(),
                name: "worker-broken".into(),
                phase: PodPhase::CrashLoop,
                ready: (0, 1),
                restarts: 17,
                age_seconds: 600,
                node: "node-2".into(),
                cpu_millis: None,
                mem_bytes: None,
                qos: QosClass::BestEffort,
            },
        ];
        let nodes = vec![NodeSnapshot {
            name: "node-1".into(),
            status: NodeStatus::Ready,
            roles: vec!["control-plane".into()],
            age_seconds: 500_000,
            kubelet_version: "v1.30.2".into(),
            cpu_capacity_millis: 4000,
            cpu_allocatable_millis: 3800,
            cpu_used_millis: metrics.then_some(1900),
            mem_capacity_bytes: 8 << 30,
            mem_allocatable_bytes: 7 << 30,
            mem_used_bytes: metrics.then_some(3 << 30),
            pod_count: 12,
            pod_capacity: 110,
        }];
        let deployments = vec![DeploymentSnapshot {
            namespace: "default".into(),
            name: "api".into(),
            replicas_desired: 3,
            replicas_ready: 2,
            replicas_uptodate: 3,
            replicas_available: 2,
            age_seconds: 90_000,
            strategy: DeploymentStrategy::RollingUpdate,
        }];

        let mut snap = snapshot();
        snap.kube = Some(KubeSnapshot {
            cluster_kind: ClusterKind::K3s,
            server_version: Some("v1.30.2+k3s1".into()),
            current_namespace: namespace.into(),
            reachable,
            metrics_available: metrics,
            pods,
            nodes,
            deployments,
        });
        let mut app = AppState::new();
        app.tab = Tab::Kube;
        app.apply_snapshot(snap);
        app
    }

    #[test]
    fn pods_view_lists_pods() {
        let app = kube_app(true, true, "default");
        let text = all_text(&render_with(&app, 160, 30));
        assert!(text.contains("POD"));
        assert!(text.contains("api-7d9f"));
        assert!(text.contains("Running"));
    }

    #[test]
    fn namespace_scope_is_always_visible() {
        // 0.4 hid the scope until you pressed `A` and read a status message.
        let app = kube_app(true, true, "kube-system");
        let text = all_text(&render_with(&app, 160, 30));
        assert!(text.contains("ns: kube-system"), "scope missing:\n{text}");
    }

    #[test]
    fn subview_bar_shows_counts_and_keys() {
        let app = kube_app(true, true, "default");
        let text = all_text(&render_with(&app, 160, 30));
        assert!(text.contains("Pods 2"));
        assert!(text.contains("Nodes 1"));
        assert!(text.contains("Deployments 1"));
    }

    #[test]
    fn missing_metrics_are_dashes_not_zeroes() {
        let app = kube_app(true, false, "default");
        let text = all_text(&render_with(&app, 160, 30));
        assert!(text.contains("no metrics-server"));
        assert!(
            text.contains('—'),
            "an unmeasured value must not read as zero:\n{text}"
        );
    }

    #[test]
    fn broken_pods_are_visible() {
        let app = kube_app(true, true, "default");
        let text = all_text(&render_with(&app, 160, 30));
        assert!(text.contains("CrashLoop"));
        assert!(text.contains("17"), "restart count missing");
    }

    #[test]
    fn unreachable_cluster_explains_itself() {
        let app = kube_app(false, false, "default");
        let text = all_text(&render_with(&app, 100, 24));
        assert!(text.contains("No cluster reachable"));
        assert!(text.contains("KUBECONFIG"));
    }

    #[test]
    fn nodes_view_explains_the_cluster_scope_requirement() {
        let mut app = kube_app(true, true, "kube-system");
        app.switch_kube_subview(KubeSubview::Nodes);
        if let Some(k) = app.last_snapshot.as_mut().and_then(|s| s.kube.as_mut()) {
            k.nodes.clear();
        }
        let text = all_text(&render_with(&app, 120, 24));
        assert!(
            text.contains("cluster-scoped"),
            "misleading empty state:\n{text}"
        );
    }

    #[test]
    fn deployments_view_flags_a_partial_rollout() {
        let mut app = kube_app(true, true, "default");
        app.switch_kube_subview(KubeSubview::Deployments);
        let text = all_text(&render_with(&app, 140, 24));
        assert!(text.contains("DEPLOYMENT"));
        assert!(text.contains("2/3"));
    }

    #[test]
    fn crashloop_sorts_to_the_top_of_a_phase_sort() {
        assert!(phase_rank(PodPhase::CrashLoop) < phase_rank(PodPhase::Running));
        assert!(phase_rank(PodPhase::Failed) < phase_rank(PodPhase::Succeeded));
    }

    #[test]
    fn renders_under_every_profile_and_size() {
        for sv in [
            KubeSubview::Pods,
            KubeSubview::Nodes,
            KubeSubview::Deployments,
        ] {
            let mut app = kube_app(true, true, "default");
            app.switch_kube_subview(sv);
            for (w, h) in [(1u16, 1u16), (40, 8), (80, 24), (200, 50)] {
                let _ = render_with(&app, w, h);
            }
            for (color, unicode) in all_profiles() {
                let _ = render_caps(&mut app, 140, 30, color, unicode);
            }
        }
    }
}
