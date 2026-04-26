//! Concrete `kube-rs`-backed implementation of [`ClusterEngine`].
//!
//! # Architecture
//!
//! See ADR-04 (`kube-rs vs k8s-openapi direct`, accepted 2026-04-26) and
//! ADR-05 (poll vs reflectors, accepted 2026-04-26 — see
//! `.claude/output/forge/32-v04-kubernetes-epics/`).
//!
//! v0.4 ships a **poll-based** design rather than the reflector-based design
//! initially scoped: a single tokio task spawned from [`KubeEngine::connect`]
//! wakes every 5 s, calls `Api::<K>::list()` for Pods / Nodes / Deployments,
//! and writes the raw objects into a shared [`ResourceCache`]. A second task
//! polls `metrics.k8s.io/v1beta1` on the same cadence, filling
//! [`MetricsCache`]. [`ClusterEngine::snapshot`] is therefore CPU-only —
//! it reads both caches and runs the typed-to-snapshot conversion.
//!
//! Reflectors / `kube::runtime::watcher` were considered but deferred to a
//! follow-up (ADR-05): the watcher API in 0.99 has a heavier mock surface
//! that would have doubled the test code, and the poll cadence (5 s) is
//! identical to what the user-facing UI tick uses anyway. If a perf
//! measurement at v0.4.x scale (>1000 pods) shows the LIST traffic is
//! material, switching is mechanically straightforward — it's an internal
//! detail of `KubeEngine`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Node, Pod};
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::cluster_engine::{ClusterEngine, ClusterError, KubeconfigSource};
use crate::kube::{
    ClusterKind, DeploymentSnapshot, DeploymentStrategy, KubeSnapshot, NodeSnapshot, NodeStatus,
    PodPhase, PodSnapshot, QosClass,
};

// ---- Caches --------------------------------------------------------------

/// Raw API objects produced by the 5 s poll loop. Conversion to the wire
/// snapshot types runs in `snapshot()` so we never block on I/O there.
#[derive(Default)]
pub(crate) struct ResourceCache {
    pub pods: Vec<Pod>,
    pub nodes: Vec<Node>,
    pub deployments: Vec<Deployment>,
    /// Milliseconds since Unix epoch when the cache was last written.
    /// Used to derive the [`ClusterError::Stale`] threshold.
    pub last_update_ms: u64,
}

/// Metrics-server cache — populated by the metrics polling task.
///
/// `pods`/`nodes` are looked up from the snapshot conversion path; entries
/// missing from the map render as `cpu_millis = None` / `mem_bytes = None`
/// in the wire snapshot, which the UI surfaces as `—`.
#[derive(Default)]
pub(crate) struct MetricsCache {
    /// Whether `/apis/metrics.k8s.io/v1beta1` answered the last probe.
    pub available: bool,
    /// `(namespace, pod_name) -> (cpu_millis, mem_bytes)`.
    pub pods: HashMap<(String, String), (u32, u64)>,
    /// `node_name -> (cpu_millis, mem_bytes)`.
    pub nodes: HashMap<String, (u32, u64)>,
}

// ---- Engine --------------------------------------------------------------

/// `kube-rs`-backed [`ClusterEngine`].
///
/// Construction goes through [`KubeEngine::connect`] for production paths
/// (S2.6, future commit) or [`KubeEngine::new_for_test`] for unit tests
/// that prepopulate the caches by hand.
pub struct KubeEngine {
    cluster_kind: ClusterKind,
    server_version: Option<String>,
    current_namespace: String,
    resources: Arc<RwLock<ResourceCache>>,
    metrics: Arc<RwLock<MetricsCache>>,
    /// Cancels the spawned poll task on drop.
    cancel: CancellationToken,
}

impl KubeEngine {
    /// Production path — picks up the kubeconfig from `source`, builds a
    /// `kube::Client`, and spawns the resource + metrics poll tasks.
    ///
    /// **Status (S2.3):** rejects [`KubeconfigSource::None`] up-front; all
    /// other variants return [`ClusterError::Other`] until S2.6 lands the
    /// real `Config` / `Client` wiring + task spawn. The `snapshot()`
    /// pipeline below is fully implemented and can be exercised today via
    /// [`KubeEngine::new_for_test`].
    pub async fn connect(
        source: KubeconfigSource,
        _context: Option<&str>,
        _namespace: Option<&str>,
    ) -> Result<Self, ClusterError> {
        if matches!(source, KubeconfigSource::None) {
            return Err(ClusterError::KubeconfigNotFound);
        }
        Err(ClusterError::Other(
            "KubeEngine::connect is scaffolded only; real wiring lands in v0.4 E2 S2.6".into(),
        ))
    }

    /// Test constructor that bypasses the network entirely. The caches are
    /// expected to be filled with hand-crafted `Pod` / `Node` / `Deployment`
    /// objects (typically via `serde_json::from_value`) and metrics rows.
    ///
    /// `pub(crate)` because [`ResourceCache`] / [`MetricsCache`] are
    /// implementation details — never exposed to consumers of muxtop-core.
    #[doc(hidden)]
    #[allow(dead_code)] // exercised by the in-module tests; will be used by collector tests in E4
    pub(crate) fn new_for_test(
        cluster_kind: ClusterKind,
        server_version: Option<String>,
        current_namespace: String,
        resources: ResourceCache,
        metrics: MetricsCache,
    ) -> Self {
        Self {
            cluster_kind,
            server_version,
            current_namespace,
            resources: Arc::new(RwLock::new(resources)),
            metrics: Arc::new(RwLock::new(metrics)),
            cancel: CancellationToken::new(),
        }
    }
}

impl Drop for KubeEngine {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[async_trait]
impl ClusterEngine for KubeEngine {
    async fn snapshot(&self) -> Result<KubeSnapshot, ClusterError> {
        let resources = self.resources.read().await;
        let metrics = self.metrics.read().await;

        let now_ms = unix_ms();
        let pods: Vec<PodSnapshot> = resources
            .pods
            .iter()
            .map(|p| pod_to_snapshot(p, &metrics, now_ms))
            .collect();
        let nodes: Vec<NodeSnapshot> = resources
            .nodes
            .iter()
            .map(|n| node_to_snapshot(n, &metrics, now_ms))
            .collect();
        let deployments: Vec<DeploymentSnapshot> = resources
            .deployments
            .iter()
            .map(|d| deployment_to_snapshot(d, now_ms))
            .collect();

        // `reachable` is true iff at least one resource list has been
        // populated by the poll loop (i.e. last_update_ms is non-zero).
        let reachable = resources.last_update_ms > 0;

        Ok(KubeSnapshot {
            cluster_kind: self.cluster_kind,
            server_version: self.server_version.clone(),
            current_namespace: self.current_namespace.clone(),
            reachable,
            metrics_available: metrics.available,
            pods,
            nodes,
            deployments,
        })
    }

    async fn metrics_available(&self) -> bool {
        self.metrics.read().await.available
    }

    fn kind(&self) -> ClusterKind {
        self.cluster_kind
    }

    fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
    }
}

// ---- Conversions ---------------------------------------------------------

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Convert a typed [`Pod`] to a wire [`PodSnapshot`], merging in metrics
/// from `metrics` when present.
pub(crate) fn pod_to_snapshot(pod: &Pod, metrics: &MetricsCache, now_ms: u64) -> PodSnapshot {
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();
    let name = pod.metadata.name.clone().unwrap_or_default();
    let phase = pod_phase_synthetic(pod);
    let ready = pod_ready_ratio(pod);
    let restarts = pod_restart_count(pod);
    let age_seconds = creation_age_seconds(pod.metadata.creation_timestamp.as_ref(), now_ms);
    let node = pod
        .spec
        .as_ref()
        .and_then(|s| s.node_name.clone())
        .unwrap_or_default();
    let qos = pod_qos(pod);

    let metrics_key = (namespace.clone(), name.clone());
    let (cpu_millis, mem_bytes) = match metrics.pods.get(&metrics_key) {
        Some((cpu, mem)) => (Some(*cpu), Some(*mem)),
        None => (None, None),
    };

    PodSnapshot {
        namespace,
        name,
        phase,
        ready,
        restarts,
        age_seconds,
        node,
        cpu_millis,
        mem_bytes,
        qos,
    }
}

fn pod_phase_synthetic(pod: &Pod) -> PodPhase {
    // Terminating wins over everything else — once metadata.deletionTimestamp
    // is set, the pod is going away regardless of its container states.
    if pod.metadata.deletion_timestamp.is_some() {
        return PodPhase::Terminating;
    }

    // CrashLoop synthesis: any container with state.waiting.reason ==
    // "CrashLoopBackOff".
    if let Some(status) = &pod.status
        && let Some(statuses) = &status.container_statuses
        && statuses.iter().any(|cs| {
            cs.state
                .as_ref()
                .and_then(|s| s.waiting.as_ref())
                .and_then(|w| w.reason.as_deref())
                == Some("CrashLoopBackOff")
        })
    {
        return PodPhase::CrashLoop;
    }

    match pod
        .status
        .as_ref()
        .and_then(|s| s.phase.as_deref())
        .unwrap_or("")
    {
        "Pending" => PodPhase::Pending,
        "Running" => PodPhase::Running,
        "Succeeded" => PodPhase::Succeeded,
        "Failed" => PodPhase::Failed,
        _ => PodPhase::Unknown,
    }
}

fn pod_ready_ratio(pod: &Pod) -> (u8, u8) {
    let statuses = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref());
    match statuses {
        Some(list) => {
            let total = list.len().min(u8::MAX as usize) as u8;
            let ready = list
                .iter()
                .filter(|cs| cs.ready)
                .count()
                .min(u8::MAX as usize) as u8;
            (ready, total)
        }
        None => (0, 0),
    }
}

fn pod_restart_count(pod: &Pod) -> u32 {
    pod.status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .map(|list| list.iter().map(|cs| cs.restart_count.max(0) as u32).sum())
        .unwrap_or(0)
}

fn pod_qos(pod: &Pod) -> QosClass {
    match pod
        .status
        .as_ref()
        .and_then(|s| s.qos_class.as_deref())
        .unwrap_or("")
    {
        "Guaranteed" => QosClass::Guaranteed,
        "Burstable" => QosClass::Burstable,
        _ => QosClass::BestEffort,
    }
}

/// Convert a typed [`Node`] to a wire [`NodeSnapshot`].
pub(crate) fn node_to_snapshot(node: &Node, metrics: &MetricsCache, now_ms: u64) -> NodeSnapshot {
    let name = node.metadata.name.clone().unwrap_or_default();
    let status = node_status_synthetic(node);
    let roles = node_roles(node);
    let age_seconds = creation_age_seconds(node.metadata.creation_timestamp.as_ref(), now_ms);
    let kubelet_version = node
        .status
        .as_ref()
        .and_then(|s| s.node_info.as_ref())
        .map(|info| info.kubelet_version.clone())
        .unwrap_or_default();

    let (cpu_capacity_millis, mem_capacity_bytes, pod_capacity) = node
        .status
        .as_ref()
        .and_then(|s| s.capacity.as_ref())
        .map(|caps| {
            let cpu = caps
                .get("cpu")
                .map(|q| parse_quantity_to_millis(&q.0))
                .unwrap_or(0);
            let mem = caps
                .get("memory")
                .map(|q| parse_quantity_to_bytes(&q.0))
                .unwrap_or(0);
            let pods = caps
                .get("pods")
                .and_then(|q| q.0.parse::<u32>().ok())
                .unwrap_or(0);
            (cpu, mem, pods)
        })
        .unwrap_or((0, 0, 0));

    let (cpu_allocatable_millis, mem_allocatable_bytes) = node
        .status
        .as_ref()
        .and_then(|s| s.allocatable.as_ref())
        .map(|alloc| {
            let cpu = alloc
                .get("cpu")
                .map(|q| parse_quantity_to_millis(&q.0))
                .unwrap_or(0);
            let mem = alloc
                .get("memory")
                .map(|q| parse_quantity_to_bytes(&q.0))
                .unwrap_or(0);
            (cpu, mem)
        })
        .unwrap_or((0, 0));

    let (cpu_used_millis, mem_used_bytes) = match metrics.nodes.get(&name) {
        Some((cpu, mem)) => (Some(*cpu), Some(*mem)),
        None => (None, None),
    };

    NodeSnapshot {
        name,
        status,
        roles,
        age_seconds,
        kubelet_version,
        cpu_capacity_millis,
        cpu_allocatable_millis,
        cpu_used_millis,
        mem_capacity_bytes,
        mem_allocatable_bytes,
        mem_used_bytes,
        pod_count: 0, // Populated in S2.6 once the resource cache has the cluster-wide pod list.
        pod_capacity,
    }
}

fn node_status_synthetic(node: &Node) -> NodeStatus {
    if node.spec.as_ref().and_then(|s| s.unschedulable) == Some(true) {
        return NodeStatus::SchedulingDisabled;
    }
    let conditions = node.status.as_ref().and_then(|s| s.conditions.as_ref());
    if let Some(conditions) = conditions {
        for cond in conditions {
            if cond.type_ == "Ready" {
                return match cond.status.as_str() {
                    "True" => NodeStatus::Ready,
                    "False" => NodeStatus::NotReady,
                    _ => NodeStatus::Unknown,
                };
            }
        }
    }
    NodeStatus::Unknown
}

fn node_roles(node: &Node) -> Vec<String> {
    let mut roles = Vec::new();
    if let Some(labels) = &node.metadata.labels {
        for k in labels.keys() {
            if let Some(role) = k.strip_prefix("node-role.kubernetes.io/")
                && !role.is_empty()
            {
                roles.push(role.to_string());
            }
        }
    }
    roles.sort();
    roles
}

/// Convert a typed [`Deployment`] to a wire [`DeploymentSnapshot`].
pub(crate) fn deployment_to_snapshot(d: &Deployment, now_ms: u64) -> DeploymentSnapshot {
    let namespace = d.metadata.namespace.clone().unwrap_or_default();
    let name = d.metadata.name.clone().unwrap_or_default();
    let age_seconds = creation_age_seconds(d.metadata.creation_timestamp.as_ref(), now_ms);

    let replicas_desired = d.spec.as_ref().and_then(|s| s.replicas).unwrap_or(0).max(0) as u32;

    let (replicas_ready, replicas_uptodate, replicas_available) = d
        .status
        .as_ref()
        .map(|s| {
            (
                s.ready_replicas.unwrap_or(0).max(0) as u32,
                s.updated_replicas.unwrap_or(0).max(0) as u32,
                s.available_replicas.unwrap_or(0).max(0) as u32,
            )
        })
        .unwrap_or((0, 0, 0));

    let strategy = d
        .spec
        .as_ref()
        .and_then(|s| s.strategy.as_ref())
        .and_then(|st| st.type_.as_deref())
        .map(|t| match t {
            "Recreate" => DeploymentStrategy::Recreate,
            _ => DeploymentStrategy::RollingUpdate,
        })
        .unwrap_or(DeploymentStrategy::RollingUpdate);

    DeploymentSnapshot {
        namespace,
        name,
        replicas_desired,
        replicas_ready,
        replicas_uptodate,
        replicas_available,
        age_seconds,
        strategy,
    }
}

// ---- Quantity parsing ----------------------------------------------------

/// Parse a Kubernetes `Quantity` string into milli-cores.
///
/// Inputs we accept:
/// * `"4"` → 4_000 (4 cores)
/// * `"2000m"` → 2_000
/// * `"100m"` → 100
/// * `"0.5"` → 500
/// * Anything else → 0 (logged at the call site if needed).
pub(crate) fn parse_quantity_to_millis(raw: &str) -> u32 {
    let s = raw.trim();
    if let Some(stripped) = s.strip_suffix('m') {
        return stripped.parse::<u32>().unwrap_or(0);
    }
    if let Ok(int) = s.parse::<u32>() {
        return int.saturating_mul(1000);
    }
    if let Ok(float) = s.parse::<f64>() {
        return (float * 1000.0).round() as u32;
    }
    0
}

/// Parse a Kubernetes `Quantity` string into bytes.
///
/// Suffixes recognised: `Ki`, `Mi`, `Gi`, `Ti` (binary IEC) and `K`, `M`,
/// `G`, `T` (decimal SI). `n`/`u`/`m` (sub-unit) are intentionally not
/// supported — they don't appear in capacity/allocatable for memory.
pub(crate) fn parse_quantity_to_bytes(raw: &str) -> u64 {
    let s = raw.trim();
    let multipliers: &[(&str, u64)] = &[
        ("Ti", 1u64 << 40),
        ("Gi", 1u64 << 30),
        ("Mi", 1u64 << 20),
        ("Ki", 1u64 << 10),
        ("T", 1_000_000_000_000),
        ("G", 1_000_000_000),
        ("M", 1_000_000),
        ("K", 1_000),
    ];
    for (suffix, mult) in multipliers {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return stripped
                .parse::<u64>()
                .map(|n| n.saturating_mul(*mult))
                .unwrap_or(0);
        }
    }
    s.parse::<u64>().unwrap_or(0)
}

// ---- Time helpers --------------------------------------------------------

fn creation_age_seconds(
    creation: Option<&k8s_openapi::apimachinery::pkg::apis::meta::v1::Time>,
    now_ms: u64,
) -> u64 {
    use k8s_openapi::chrono::Utc;
    let Some(t) = creation else { return 0 };
    let created_ms = t.0.with_timezone(&Utc).timestamp_millis();
    if created_ms <= 0 {
        return 0;
    }
    let created_ms = created_ms as u64;
    now_ms.saturating_sub(created_ms) / 1000
}

// ---- Cluster kind heuristic ---------------------------------------------

/// Derive a [`ClusterKind`] from the API server `gitVersion` string.
///
/// Heuristics are intentionally ASCII-cheap (substring match on the
/// lowercased version string) — false positives are acceptable since
/// `cluster_kind` only drives a UI badge.
#[allow(dead_code)] // wired into KubeEngine::connect in S2.6; exercised by tests now.
pub(crate) fn cluster_kind_from_git_version(git_version: &str) -> ClusterKind {
    let v = git_version.to_ascii_lowercase();
    if v.contains("eks") {
        return ClusterKind::Eks;
    }
    if v.contains("gke") {
        return ClusterKind::Gke;
    }
    if v.contains("aks") {
        return ClusterKind::Aks;
    }
    if v.contains("k3d") {
        return ClusterKind::K3d;
    }
    if v.contains("k3s") {
        return ClusterKind::K3s;
    }
    if v.contains("kind") {
        return ClusterKind::Kind;
    }
    if v.contains("openshift") {
        return ClusterKind::Openshift;
    }
    ClusterKind::Generic
}

// ---- Tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_engine_for_test() -> KubeEngine {
        KubeEngine::new_for_test(
            ClusterKind::Generic,
            None,
            String::new(),
            ResourceCache::default(),
            MetricsCache::default(),
        )
    }

    // ---- connect stubs ----

    #[tokio::test]
    async fn connect_rejects_kubeconfig_none() {
        let res = KubeEngine::connect(KubeconfigSource::None, None, None).await;
        assert!(matches!(res, Err(ClusterError::KubeconfigNotFound)));
    }

    #[tokio::test]
    async fn connect_returns_other_for_unimplemented_source() {
        let dir = tempfile::tempdir().unwrap();
        let kc = dir.path().join("config");
        std::fs::File::create(&kc).unwrap();

        let res = KubeEngine::connect(KubeconfigSource::Home(kc), None, None).await;
        assert!(matches!(res, Err(ClusterError::Other(_))));
    }

    // ---- empty engine ----

    #[tokio::test]
    async fn empty_engine_yields_unreachable_snapshot() {
        let engine = empty_engine_for_test();
        let snap = engine.snapshot().await.expect("snapshot");
        assert!(!snap.reachable);
        assert!(snap.pods.is_empty());
        assert!(snap.nodes.is_empty());
        assert!(snap.deployments.is_empty());
        assert!(!snap.metrics_available);
    }

    #[tokio::test]
    async fn engine_implements_cluster_engine_trait() {
        let engine: Box<dyn ClusterEngine + Send + Sync> = Box::new(empty_engine_for_test());
        assert_eq!(engine.kind(), ClusterKind::Generic);
        assert!(engine.server_version().is_none());
        assert!(!engine.metrics_available().await);
    }

    #[tokio::test]
    async fn drop_cancels_token() {
        let engine = empty_engine_for_test();
        let token = engine.cancel.clone();
        assert!(!token.is_cancelled());
        drop(engine);
        assert!(token.is_cancelled());
    }

    // ---- pod conversions ----

    fn pod_from_json(value: serde_json::Value) -> Pod {
        serde_json::from_value(value).expect("valid Pod JSON")
    }

    #[test]
    fn pod_to_snapshot_running() {
        let p = pod_from_json(json!({
            "metadata": { "namespace": "default", "name": "nginx-1" },
            "spec": { "nodeName": "node-1" },
            "status": {
                "phase": "Running",
                "qosClass": "Burstable",
                "containerStatuses": [
                    { "name": "main", "ready": true, "restartCount": 0,
                      "image": "nginx:1.27", "imageID": "", "state": {"running": {}} }
                ]
            }
        }));
        let metrics = MetricsCache::default();
        let snap = pod_to_snapshot(&p, &metrics, 0);
        assert_eq!(snap.namespace, "default");
        assert_eq!(snap.name, "nginx-1");
        assert_eq!(snap.phase, PodPhase::Running);
        assert_eq!(snap.ready, (1, 1));
        assert_eq!(snap.restarts, 0);
        assert_eq!(snap.node, "node-1");
        assert_eq!(snap.qos, QosClass::Burstable);
        assert!(snap.cpu_millis.is_none());
        assert!(snap.mem_bytes.is_none());
    }

    #[test]
    fn pod_to_snapshot_crashloop_synth() {
        let p = pod_from_json(json!({
            "metadata": { "namespace": "default", "name": "broken" },
            "status": {
                "phase": "Running",
                "containerStatuses": [
                    { "name": "main", "ready": false, "restartCount": 7,
                      "image": "x", "imageID": "",
                      "state": { "waiting": { "reason": "CrashLoopBackOff" } } }
                ]
            }
        }));
        let metrics = MetricsCache::default();
        let snap = pod_to_snapshot(&p, &metrics, 0);
        assert_eq!(snap.phase, PodPhase::CrashLoop);
        assert_eq!(snap.restarts, 7);
        assert_eq!(snap.ready, (0, 1));
    }

    #[test]
    fn pod_to_snapshot_terminating_synth() {
        let p = pod_from_json(json!({
            "metadata": {
                "namespace": "default",
                "name": "going-away",
                "deletionTimestamp": "2026-04-26T00:00:00Z"
            },
            "status": { "phase": "Running" }
        }));
        let metrics = MetricsCache::default();
        let snap = pod_to_snapshot(&p, &metrics, 0);
        assert_eq!(snap.phase, PodPhase::Terminating);
    }

    #[test]
    fn pod_to_snapshot_metrics_injection() {
        let p = pod_from_json(json!({
            "metadata": { "namespace": "default", "name": "instrumented" },
            "status": { "phase": "Running" }
        }));
        let mut pods = HashMap::new();
        pods.insert(
            ("default".into(), "instrumented".into()),
            (42, 128 * 1024 * 1024),
        );
        let metrics = MetricsCache {
            available: true,
            pods,
            ..Default::default()
        };
        let snap = pod_to_snapshot(&p, &metrics, 0);
        assert_eq!(snap.cpu_millis, Some(42));
        assert_eq!(snap.mem_bytes, Some(128 * 1024 * 1024));
    }

    #[test]
    fn pod_to_snapshot_unknown_phase_falls_through() {
        let p = pod_from_json(json!({
            "metadata": { "namespace": "x", "name": "y" },
            "status": { "phase": "WeirdPhase" }
        }));
        let metrics = MetricsCache::default();
        let snap = pod_to_snapshot(&p, &metrics, 0);
        assert_eq!(snap.phase, PodPhase::Unknown);
    }

    // ---- node conversions ----

    fn node_from_json(value: serde_json::Value) -> Node {
        serde_json::from_value(value).expect("valid Node JSON")
    }

    #[test]
    fn node_to_snapshot_basic() {
        let n = node_from_json(json!({
            "metadata": {
                "name": "node-1",
                "labels": {
                    "node-role.kubernetes.io/control-plane": "",
                    "node-role.kubernetes.io/worker": "",
                    "kubernetes.io/hostname": "node-1"
                }
            },
            "spec": {},
            "status": {
                "capacity": { "cpu": "4", "memory": "8Gi", "pods": "110" },
                "allocatable": { "cpu": "3800m", "memory": "7900Mi", "pods": "110" },
                "conditions": [
                    { "type": "Ready", "status": "True", "lastTransitionTime": "2026-04-26T00:00:00Z", "lastHeartbeatTime": "2026-04-26T00:00:00Z" }
                ],
                "nodeInfo": {
                    "kubeletVersion": "v1.31.0",
                    "architecture": "amd64",
                    "bootID": "",
                    "containerRuntimeVersion": "containerd://1.7",
                    "kernelVersion": "6.1",
                    "kubeProxyVersion": "v1.31.0",
                    "machineID": "",
                    "operatingSystem": "linux",
                    "osImage": "linux",
                    "systemUUID": ""
                }
            }
        }));
        let metrics = MetricsCache::default();
        let snap = node_to_snapshot(&n, &metrics, 0);
        assert_eq!(snap.name, "node-1");
        assert_eq!(snap.status, NodeStatus::Ready);
        assert_eq!(
            snap.roles,
            vec!["control-plane".to_string(), "worker".to_string()]
        );
        assert_eq!(snap.kubelet_version, "v1.31.0");
        assert_eq!(snap.cpu_capacity_millis, 4_000);
        assert_eq!(snap.cpu_allocatable_millis, 3_800);
        assert_eq!(snap.mem_capacity_bytes, 8u64 * 1024 * 1024 * 1024);
        assert_eq!(snap.mem_allocatable_bytes, 7_900u64 * 1024 * 1024);
        assert_eq!(snap.pod_capacity, 110);
        assert!(snap.cpu_used_millis.is_none());
    }

    #[test]
    fn node_unschedulable_is_scheduling_disabled() {
        let n = node_from_json(json!({
            "metadata": { "name": "node-x" },
            "spec": { "unschedulable": true },
            "status": {
                "conditions": [
                    { "type": "Ready", "status": "True", "lastTransitionTime": "2026-04-26T00:00:00Z", "lastHeartbeatTime": "2026-04-26T00:00:00Z" }
                ]
            }
        }));
        let metrics = MetricsCache::default();
        let snap = node_to_snapshot(&n, &metrics, 0);
        assert_eq!(snap.status, NodeStatus::SchedulingDisabled);
    }

    #[test]
    fn node_metrics_injection() {
        let n = node_from_json(json!({
            "metadata": { "name": "node-1" },
            "spec": {},
            "status": {}
        }));
        let mut nodes = HashMap::new();
        nodes.insert("node-1".into(), (420, 2u64 * 1024 * 1024 * 1024));
        let metrics = MetricsCache {
            available: true,
            nodes,
            ..Default::default()
        };
        let snap = node_to_snapshot(&n, &metrics, 0);
        assert_eq!(snap.cpu_used_millis, Some(420));
        assert_eq!(snap.mem_used_bytes, Some(2u64 * 1024 * 1024 * 1024));
    }

    // ---- deployment conversions ----

    fn deployment_from_json(value: serde_json::Value) -> Deployment {
        serde_json::from_value(value).expect("valid Deployment JSON")
    }

    #[test]
    fn deployment_to_snapshot_basic() {
        let d = deployment_from_json(json!({
            "metadata": { "namespace": "default", "name": "nginx" },
            "spec": {
                "replicas": 3,
                "selector": { "matchLabels": { "app": "nginx" } },
                "strategy": { "type": "RollingUpdate" },
                "template": { "metadata": {}, "spec": { "containers": [] } }
            },
            "status": {
                "readyReplicas": 3,
                "updatedReplicas": 3,
                "availableReplicas": 3
            }
        }));
        let snap = deployment_to_snapshot(&d, 0);
        assert_eq!(snap.namespace, "default");
        assert_eq!(snap.name, "nginx");
        assert_eq!(snap.replicas_desired, 3);
        assert_eq!(snap.replicas_ready, 3);
        assert_eq!(snap.replicas_uptodate, 3);
        assert_eq!(snap.replicas_available, 3);
        assert_eq!(snap.strategy, DeploymentStrategy::RollingUpdate);
    }

    #[test]
    fn deployment_to_snapshot_recreate_strategy() {
        let d = deployment_from_json(json!({
            "metadata": { "namespace": "default", "name": "rec" },
            "spec": {
                "replicas": 1,
                "selector": { "matchLabels": { "a": "b" } },
                "strategy": { "type": "Recreate" },
                "template": { "metadata": {}, "spec": { "containers": [] } }
            },
            "status": {}
        }));
        let snap = deployment_to_snapshot(&d, 0);
        assert_eq!(snap.strategy, DeploymentStrategy::Recreate);
    }

    // ---- quantity parsing ----

    #[test]
    fn parse_quantity_millis_cases() {
        assert_eq!(parse_quantity_to_millis("4"), 4_000);
        assert_eq!(parse_quantity_to_millis("2000m"), 2_000);
        assert_eq!(parse_quantity_to_millis("100m"), 100);
        assert_eq!(parse_quantity_to_millis("0.5"), 500);
        assert_eq!(parse_quantity_to_millis("1.5"), 1_500);
        assert_eq!(parse_quantity_to_millis(""), 0);
        assert_eq!(parse_quantity_to_millis("garbage"), 0);
    }

    #[test]
    fn parse_quantity_bytes_cases() {
        assert_eq!(parse_quantity_to_bytes("8Gi"), 8u64 * 1024 * 1024 * 1024);
        assert_eq!(parse_quantity_to_bytes("7900Mi"), 7_900u64 * 1024 * 1024);
        assert_eq!(parse_quantity_to_bytes("1Ti"), 1u64 << 40);
        assert_eq!(parse_quantity_to_bytes("1024Ki"), 1_024 * 1_024);
        assert_eq!(parse_quantity_to_bytes("1G"), 1_000_000_000);
        assert_eq!(parse_quantity_to_bytes("1024"), 1_024);
        assert_eq!(parse_quantity_to_bytes(""), 0);
        assert_eq!(parse_quantity_to_bytes("garbage"), 0);
    }

    // ---- cluster kind heuristic ----

    #[test]
    fn cluster_kind_from_git_version_cases() {
        assert_eq!(
            cluster_kind_from_git_version("v1.31.0"),
            ClusterKind::Generic
        );
        assert_eq!(
            cluster_kind_from_git_version("v1.31.0-eks-abcd123"),
            ClusterKind::Eks
        );
        assert_eq!(
            cluster_kind_from_git_version("v1.31.0-gke.1700"),
            ClusterKind::Gke
        );
        assert_eq!(
            cluster_kind_from_git_version("v1.30.0+aks"),
            ClusterKind::Aks
        );
        assert_eq!(
            cluster_kind_from_git_version("v1.30.0+k3s1"),
            ClusterKind::K3s
        );
        assert_eq!(
            cluster_kind_from_git_version("v1.31.0-kind"),
            ClusterKind::Kind
        );
        assert_eq!(
            cluster_kind_from_git_version("v1.27.0+openshift"),
            ClusterKind::Openshift
        );
    }

    // ---- end-to-end snapshot ----

    #[tokio::test]
    async fn snapshot_with_populated_caches_is_reachable() {
        let pod = pod_from_json(json!({
            "metadata": { "namespace": "default", "name": "hello" },
            "spec": {},
            "status": { "phase": "Running" }
        }));
        let resources = ResourceCache {
            pods: vec![pod],
            last_update_ms: unix_ms(),
            ..Default::default()
        };

        let metrics = MetricsCache {
            available: true,
            ..Default::default()
        };

        let engine = KubeEngine::new_for_test(
            ClusterKind::Kind,
            Some("v1.31.0".into()),
            "default".into(),
            resources,
            metrics,
        );
        let snap = engine.snapshot().await.unwrap();
        assert!(snap.reachable);
        assert!(snap.metrics_available);
        assert_eq!(snap.pods.len(), 1);
        assert_eq!(snap.cluster_kind, ClusterKind::Kind);
        assert_eq!(snap.server_version.as_deref(), Some("v1.31.0"));
        assert_eq!(snap.current_namespace, "default");
    }
}
