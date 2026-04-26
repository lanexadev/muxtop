//! Concrete `kube-rs`-backed implementation of [`ClusterEngine`].
//!
//! Status: **scaffolded** in S2.2 of v0.4 E2 — the struct, constructors and
//! trait impl exist, but the reflector spawn and metrics-server polling
//! tasks are progressively wired in S2.3..S2.6. Until then, [`KubeEngine`]
//! returns [`KubeSnapshot::unavailable`] and an empty
//! [`ClusterEngine::metrics_available`] response.
//!
//! # Architecture
//!
//! See ADR-04 (`kube-rs vs k8s-openapi direct`, accepted 2026-04-26) and
//! ADR-05 (reflectors push vs poll, written in S2.8) in
//! `.claude/output/forge/32-v04-kubernetes-epics/`.
//!
//! Three reflectors run as detached tokio tasks (each guarded by the engine's
//! `CancellationToken`) and feed three in-memory `kube::runtime::reflector::Store`s
//! for `Pod`, `Node`, `Deployment`. A fourth task polls
//! `metrics.k8s.io/v1beta1` every 5 s to keep `cpu_millis`/`mem_bytes` fresh.
//! [`ClusterEngine::snapshot`] reads from these caches with no network I/O.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::cluster_engine::{ClusterEngine, ClusterError, KubeconfigSource};
use crate::kube::{ClusterKind, KubeSnapshot};

/// Shared metrics-server cache (filled by a 5 s poll task in S2.5).
#[derive(Default)]
pub(crate) struct MetricsCache {
    /// Whether `/apis/metrics.k8s.io/v1beta1` answered the last probe.
    pub available: bool,
    /// `(namespace, pod_name) -> (cpu_millis, mem_bytes)`.
    pub pods: std::collections::HashMap<(String, String), (u32, u64)>,
    /// `node_name -> (cpu_millis, mem_bytes)`.
    pub nodes: std::collections::HashMap<String, (u32, u64)>,
}

/// `kube-rs`-backed [`ClusterEngine`].
///
/// Construction goes through [`KubeEngine::connect`] for production paths or
/// [`KubeEngine::new_for_test`] for unit tests that don't need a real cluster.
pub struct KubeEngine {
    cluster_kind: ClusterKind,
    server_version: Option<String>,
    current_namespace: String,
    metrics: Arc<RwLock<MetricsCache>>,
    /// Cancels the spawned reflector + metrics tasks on drop.
    cancel: CancellationToken,
}

impl KubeEngine {
    /// Production path — picks up the kubeconfig from `source`, builds a
    /// `kube::Client`, and (eventually) spawns the reflector + metrics
    /// tasks.
    ///
    /// **Status (S2.2):** rejects [`KubeconfigSource::None`] up-front; all
    /// other variants return [`ClusterError::Other`] with a "not yet
    /// implemented" message until S2.6 lands the real wiring.
    pub async fn connect(
        source: KubeconfigSource,
        _context: Option<&str>,
        _namespace: Option<&str>,
    ) -> Result<Self, ClusterError> {
        if matches!(source, KubeconfigSource::None) {
            return Err(ClusterError::KubeconfigNotFound);
        }
        // S2.6 will replace this with the real Config / Client / reflector
        // construction. Keeping the surface stable so the caller code can
        // be written in parallel.
        Err(ClusterError::Other(
            "KubeEngine::connect is scaffolded only; real wiring lands in v0.4 E2 S2.6".into(),
        ))
    }

    /// Test constructor that bypasses the network entirely. Wired up in S2.7
    /// once the reflector store types are imported; for now it exposes the
    /// minimal fields required by the trait stub.
    #[doc(hidden)]
    pub fn new_for_test(
        cluster_kind: ClusterKind,
        server_version: Option<String>,
        current_namespace: String,
    ) -> Self {
        Self {
            cluster_kind,
            server_version,
            current_namespace,
            metrics: Arc::new(RwLock::new(MetricsCache::default())),
            cancel: CancellationToken::new(),
        }
    }
}

impl Drop for KubeEngine {
    fn drop(&mut self) {
        // Tells every spawned task to wind down without blocking.
        self.cancel.cancel();
    }
}

#[async_trait]
impl ClusterEngine for KubeEngine {
    async fn snapshot(&self) -> Result<KubeSnapshot, ClusterError> {
        // S2.3 will replace this with a live read from the reflector stores;
        // until then we return the canonical "no data yet" snapshot, but
        // populated with the engine's metadata so the UI can still render
        // the cluster badge.
        let metrics_available = self.metrics.read().await.available;
        let mut snap = KubeSnapshot::unavailable();
        snap.cluster_kind = self.cluster_kind;
        snap.server_version = self.server_version.clone();
        snap.current_namespace = self.current_namespace.clone();
        snap.metrics_available = metrics_available;
        Ok(snap)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_rejects_kubeconfig_none() {
        let res = KubeEngine::connect(KubeconfigSource::None, None, None).await;
        assert!(matches!(res, Err(ClusterError::KubeconfigNotFound)));
    }

    #[tokio::test]
    async fn connect_returns_other_for_unimplemented_source() {
        // Until S2.6 lands the real wiring, every non-None source returns
        // ClusterError::Other. This regression guard ensures the change
        // (S2.6 lifting the stub) is impossible to ship silently.
        let dir = tempfile::tempdir().unwrap();
        let kc = dir.path().join("config");
        std::fs::File::create(&kc).unwrap();

        let res = KubeEngine::connect(KubeconfigSource::Home(kc), None, None).await;
        assert!(matches!(res, Err(ClusterError::Other(_))));
    }

    #[tokio::test]
    async fn new_for_test_yields_unavailable_snapshot() {
        let engine =
            KubeEngine::new_for_test(ClusterKind::Kind, Some("v1.31.0".into()), "default".into());

        let snap = engine.snapshot().await.expect("scaffolded snapshot");
        assert!(!snap.reachable);
        assert!(snap.pods.is_empty());
        assert_eq!(snap.cluster_kind, ClusterKind::Kind);
        assert_eq!(snap.server_version.as_deref(), Some("v1.31.0"));
        assert_eq!(snap.current_namespace, "default");
        assert!(!snap.metrics_available);
    }

    #[tokio::test]
    async fn new_for_test_implements_cluster_engine_trait() {
        // dyn-safety regression guard — same shape as the StubCluster test
        // in cluster_engine.rs but exercises the real production type.
        let engine: Box<dyn ClusterEngine + Send + Sync> = Box::new(KubeEngine::new_for_test(
            ClusterKind::Generic,
            None,
            String::new(),
        ));
        assert_eq!(engine.kind(), ClusterKind::Generic);
        assert!(engine.server_version().is_none());
        assert!(!engine.metrics_available().await);
    }

    #[tokio::test]
    async fn drop_cancels_token() {
        // The CancellationToken is the leash on the (future) reflector and
        // metrics tasks — verify Drop fires the cancellation. We can't
        // observe the spawned tasks here (none in S2.2) so we exercise the
        // token directly via a clone.
        let engine =
            KubeEngine::new_for_test(ClusterKind::Generic, None, "default".into());
        let token = engine.cancel.clone();
        assert!(!token.is_cancelled());
        drop(engine);
        assert!(token.is_cancelled());
    }
}
