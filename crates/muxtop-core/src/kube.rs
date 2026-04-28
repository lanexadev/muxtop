//! Kubernetes data model for the Kube tab (v0.4.0).
//!
//! Mirrors the structure of `containers.rs`: plain-data `*Snapshot` structs
//! that the collector publishes through `SystemSnapshot`. The collection
//! logic lives in `cluster_engine.rs` (trait) and `kube_engine.rs` (concrete
//! kube-rs impl, E2); this module is data-only.
//!
//! ## Field ordering is contractual
//!
//! Field order is part of the wire protocol since E3 landed the
//! `Encode`/`Decode` derives. Adding fields at the end is a wire-format
//! break (clients must match the same minor version). This was the lesson
//! from v0.3.1 (`ContainerSnapshot` gaining `id_full` mid-release). Keep
//! new fields at the bottom of the struct and document the break in
//! `CHANGELOG.md` under `### Wire protocol break`.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Lifecycle phase of a Pod, mirroring the Kubernetes core PodPhase enum
/// with two synthetic states (`CrashLoop`, `Terminating`) computed from
/// `containerStatuses[].state` and `metadata.deletionTimestamp` respectively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum PodPhase {
    Pending,
    Running,
    Succeeded,
    Failed,
    /// Synthetic: at least one container is in `CrashLoopBackOff`.
    CrashLoop,
    /// Synthetic: `metadata.deletionTimestamp` is set.
    Terminating,
    Unknown,
}

/// QoS class assigned by the kube-scheduler. Derived from container
/// `resources.requests` / `resources.limits`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum QosClass {
    Guaranteed,
    Burstable,
    BestEffort,
}

/// Aggregated readiness of a Node, derived from
/// `status.conditions[type=Ready]` plus `spec.unschedulable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum NodeStatus {
    Ready,
    NotReady,
    SchedulingDisabled,
    Unknown,
}

/// Update strategy of a Deployment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum DeploymentStrategy {
    RollingUpdate,
    Recreate,
}

/// High-level cluster fingerprint, used to badge the UI.
///
/// Derivation hints:
/// * `serverVersion.gitVersion` containing `kind` → `Kind`.
/// * Node label `k3s.io/hostname` → `K3s` (or `K3d` if container-runtime is `containerd`+kind-style).
/// * Server URL ending in `eks.amazonaws.com` → `Eks`.
/// * `*.gke.goog` → `Gke`. `*.azmk8s.io` → `Aks`. OpenShift annotation → `Openshift`.
/// * Otherwise → `Generic`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ClusterKind {
    Generic,
    Kind,
    K3d,
    K3s,
    Eks,
    Gke,
    Aks,
    Openshift,
}

/// Per-pod snapshot. CPU and memory are `Option` because they require
/// `metrics.k8s.io` to be served by the cluster (cf. `KubeSnapshot::metrics_available`).
///
/// Wire ordering: see module doc — fields below are appended only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct PodSnapshot {
    pub namespace: String,
    pub name: String,
    pub phase: PodPhase,
    /// `(ready, total)` containers in the pod.
    pub ready: (u8, u8),
    /// Sum of restart counts across all containers.
    pub restarts: u32,
    /// Seconds since `metadata.creationTimestamp`.
    pub age_seconds: u64,
    /// Node hosting the pod (empty when not yet scheduled).
    pub node: String,
    /// Live CPU usage in milli-cores. `None` when metrics-server is absent.
    pub cpu_millis: Option<u32>,
    /// Live memory usage in bytes. `None` when metrics-server is absent.
    pub mem_bytes: Option<u64>,
    pub qos: QosClass,
}

/// Per-node snapshot. CPU/memory `*_used_*` are `Option` because they
/// require `metrics.k8s.io` (NodeMetrics).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct NodeSnapshot {
    pub name: String,
    pub status: NodeStatus,
    /// Roles labelled on the node (e.g. `["control-plane", "worker"]`).
    pub roles: Vec<String>,
    pub age_seconds: u64,
    pub kubelet_version: String,
    pub cpu_capacity_millis: u32,
    pub cpu_allocatable_millis: u32,
    /// `None` when metrics-server is absent.
    pub cpu_used_millis: Option<u32>,
    pub mem_capacity_bytes: u64,
    pub mem_allocatable_bytes: u64,
    /// `None` when metrics-server is absent.
    pub mem_used_bytes: Option<u64>,
    pub pod_count: u32,
    pub pod_capacity: u32,
}

/// Per-deployment snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct DeploymentSnapshot {
    pub namespace: String,
    pub name: String,
    pub replicas_desired: u32,
    pub replicas_ready: u32,
    pub replicas_uptodate: u32,
    pub replicas_available: u32,
    pub age_seconds: u64,
    pub strategy: DeploymentStrategy,
}

/// Aggregated cluster snapshot for a single muxtop tick.
///
/// `reachable = false` with empty vecs is the canonical "cluster down /
/// no kubeconfig" state; the TUI uses it to render the placeholder message.
/// `metrics_available = false` is the orthogonal "cluster up but
/// metrics-server missing" state — pod/node tables render with `—` in the
/// CPU/MEM columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct KubeSnapshot {
    pub cluster_kind: ClusterKind,
    /// `Some(...)` when the API server `/version` endpoint responded; `None`
    /// when the cluster could not be reached.
    pub server_version: Option<String>,
    /// Default namespace from the active kubeconfig context.
    pub current_namespace: String,
    pub reachable: bool,
    pub metrics_available: bool,
    pub pods: Vec<PodSnapshot>,
    pub nodes: Vec<NodeSnapshot>,
    pub deployments: Vec<DeploymentSnapshot>,
}

impl KubeSnapshot {
    /// Canonical empty snapshot when no cluster is reachable.
    ///
    /// Mirrors `ContainersSnapshot::unavailable()`. Used by the collector
    /// when `detect_kubeconfig` returns `None` or the engine fails to
    /// connect.
    pub fn unavailable() -> Self {
        Self {
            cluster_kind: ClusterKind::Generic,
            server_version: None,
            current_namespace: String::new(),
            reachable: false,
            metrics_available: false,
            pods: Vec::new(),
            nodes: Vec::new(),
            deployments: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pod() -> PodSnapshot {
        PodSnapshot {
            namespace: "default".into(),
            name: "nginx-7b9c6b8f4d-x9p2t".into(),
            phase: PodPhase::Running,
            ready: (2, 2),
            restarts: 0,
            age_seconds: 3600,
            node: "node-1".into(),
            cpu_millis: Some(15),
            mem_bytes: Some(128 * 1024 * 1024),
            qos: QosClass::Burstable,
        }
    }

    fn sample_node() -> NodeSnapshot {
        NodeSnapshot {
            name: "node-1".into(),
            status: NodeStatus::Ready,
            roles: vec!["control-plane".into(), "worker".into()],
            age_seconds: 86_400,
            kubelet_version: "v1.31.0".into(),
            cpu_capacity_millis: 4_000,
            cpu_allocatable_millis: 3_800,
            cpu_used_millis: Some(420),
            mem_capacity_bytes: 8 * 1024 * 1024 * 1024,
            mem_allocatable_bytes: 7_900 * 1024 * 1024,
            mem_used_bytes: Some(2 * 1024 * 1024 * 1024),
            pod_count: 12,
            pod_capacity: 110,
        }
    }

    fn sample_deployment() -> DeploymentSnapshot {
        DeploymentSnapshot {
            namespace: "default".into(),
            name: "nginx".into(),
            replicas_desired: 3,
            replicas_ready: 3,
            replicas_uptodate: 3,
            replicas_available: 3,
            age_seconds: 3600,
            strategy: DeploymentStrategy::RollingUpdate,
        }
    }

    #[test]
    fn pod_snapshot_clone_and_equality() {
        let original = sample_pod();
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn node_snapshot_clone_and_equality() {
        let original = sample_node();
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn deployment_snapshot_clone_and_equality() {
        let original = sample_deployment();
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }

    #[test]
    fn kube_snapshot_unavailable_is_empty_and_unreachable() {
        let s = KubeSnapshot::unavailable();
        assert!(!s.reachable);
        assert!(!s.metrics_available);
        assert!(s.pods.is_empty());
        assert!(s.nodes.is_empty());
        assert!(s.deployments.is_empty());
        assert!(s.current_namespace.is_empty());
        assert!(s.server_version.is_none());
        assert_eq!(s.cluster_kind, ClusterKind::Generic);
    }

    #[test]
    fn pod_phase_is_exhaustive() {
        // Exhaustive match without wildcard — if a new variant is added the
        // compiler flags this test, forcing the UI / sort code to be updated.
        for phase in [
            PodPhase::Pending,
            PodPhase::Running,
            PodPhase::Succeeded,
            PodPhase::Failed,
            PodPhase::CrashLoop,
            PodPhase::Terminating,
            PodPhase::Unknown,
        ] {
            let _label: &'static str = match phase {
                PodPhase::Pending => "pending",
                PodPhase::Running => "running",
                PodPhase::Succeeded => "succeeded",
                PodPhase::Failed => "failed",
                PodPhase::CrashLoop => "crashloop",
                PodPhase::Terminating => "terminating",
                PodPhase::Unknown => "unknown",
            };
        }
    }

    #[test]
    fn node_status_is_exhaustive() {
        for st in [
            NodeStatus::Ready,
            NodeStatus::NotReady,
            NodeStatus::SchedulingDisabled,
            NodeStatus::Unknown,
        ] {
            let _label: &'static str = match st {
                NodeStatus::Ready => "ready",
                NodeStatus::NotReady => "not-ready",
                NodeStatus::SchedulingDisabled => "sched-disabled",
                NodeStatus::Unknown => "unknown",
            };
        }
    }

    #[test]
    fn cluster_kind_is_exhaustive() {
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
            let _label: &'static str = match k {
                ClusterKind::Generic => "generic",
                ClusterKind::Kind => "kind",
                ClusterKind::K3d => "k3d",
                ClusterKind::K3s => "k3s",
                ClusterKind::Eks => "eks",
                ClusterKind::Gke => "gke",
                ClusterKind::Aks => "aks",
                ClusterKind::Openshift => "openshift",
            };
        }
    }

    #[test]
    fn qos_class_is_exhaustive() {
        for q in [
            QosClass::Guaranteed,
            QosClass::Burstable,
            QosClass::BestEffort,
        ] {
            let _label: &'static str = match q {
                QosClass::Guaranteed => "guaranteed",
                QosClass::Burstable => "burstable",
                QosClass::BestEffort => "besteffort",
            };
        }
    }

    #[test]
    fn deployment_strategy_is_exhaustive() {
        for s in [
            DeploymentStrategy::RollingUpdate,
            DeploymentStrategy::Recreate,
        ] {
            let _label: &'static str = match s {
                DeploymentStrategy::RollingUpdate => "rolling",
                DeploymentStrategy::Recreate => "recreate",
            };
        }
    }

    // ---- E3 wire-protocol round-trip + anti-leak guards ----

    use bincode::config;

    #[test]
    fn pod_snapshot_round_trip() {
        let original = sample_pod();
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (PodSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn node_snapshot_round_trip() {
        let original = sample_node();
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (NodeSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn deployment_snapshot_round_trip() {
        let original = sample_deployment();
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (DeploymentSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn kube_snapshot_round_trip_populated() {
        let original = KubeSnapshot {
            cluster_kind: ClusterKind::Kind,
            server_version: Some("v1.31.0+kind".into()),
            current_namespace: "default".into(),
            reachable: true,
            metrics_available: true,
            pods: vec![sample_pod(); 25],
            nodes: vec![sample_node(); 3],
            deployments: vec![sample_deployment(); 5],
        };
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (KubeSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn kube_snapshot_round_trip_unavailable() {
        let original = KubeSnapshot::unavailable();
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (KubeSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    /// Anti-leak guard — belt-and-suspenders. The current `KubeSnapshot`
    /// shape contains zero credential-shaped strings, so this test is a
    /// regression tripwire: if a future commit adds a field that carries
    /// a token, kubeconfig content, or any TLS material, this test fires.
    #[test]
    fn kube_snapshot_wire_does_not_carry_secrets() {
        let snap = KubeSnapshot {
            cluster_kind: ClusterKind::Generic,
            server_version: Some("v1.31.0".into()),
            current_namespace: "default".into(),
            reachable: true,
            metrics_available: true,
            pods: vec![sample_pod()],
            nodes: vec![sample_node()],
            deployments: vec![sample_deployment()],
        };
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&snap, cfg).expect("encode");
        let haystack = String::from_utf8_lossy(&bytes).to_string();

        let forbidden = [
            "BEGIN PRIVATE KEY",
            "BEGIN RSA PRIVATE KEY",
            "BEGIN EC PRIVATE KEY",
            "BEGIN CERTIFICATE",
            "Bearer ",
            "client-certificate-data:",
            "client-key-data:",
            "certificate-authority-data:",
            "exec:",
        ];
        for needle in forbidden {
            assert!(
                !haystack.contains(needle),
                "wire-encoded KubeSnapshot leaked credential token `{needle}`"
            );
        }
    }
}
