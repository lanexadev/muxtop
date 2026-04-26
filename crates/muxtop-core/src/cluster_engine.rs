//! Cluster engine abstraction: `ClusterEngine` async trait + kubeconfig
//! source detection for Kubernetes clusters.
//!
//! This module defines the interface only. The concrete `kube-rs`-based
//! implementation lives in `KubeEngine` (E2, not yet implemented).
//!
//! Refresh rate contract: the Collector is expected to invoke
//! [`ClusterEngine::snapshot`] at **0.2 Hz** (once every 5 s, see ADR-05).
//! The real fetching cost is amortised by `kube_runtime` reflectors which
//! maintain push-based caches; `snapshot()` reads from the in-memory store.
//!
//! # Detection chain
//!
//! [`detect_kubeconfig`] tries, in order:
//! 1. `$KUBECONFIG` environment variable (path or `:`-separated list — kube-rs
//!    handles the multiplex; we keep the raw value).
//! 2. `~/.kube/config` (typical user kubeconfig).
//! 3. In-cluster ServiceAccount: existence of
//!    `/var/run/secrets/kubernetes.io/serviceaccount/token`.
//!
//! Returns [`KubeconfigSource::None`] if nothing is found. Callers treat this
//! as a `KubeSnapshot` with `reachable = false`.
//!
//! Detection never performs I/O beyond `Path::exists` and reading the env —
//! reachability of the cluster is the concrete engine's job (E2).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use thiserror::Error;

use crate::container_engine::EnvLookup;
use crate::kube::{ClusterKind, KubeSnapshot};

/// Errors raised by a [`ClusterEngine`] implementation.
///
/// Bridges into [`CoreError`](crate::error::CoreError) via `#[from]`.
///
/// Granularity matches the v0.3 `EngineError` (cf. ADR-01) but adds two
/// Kubernetes-specific variants: per-resource RBAC ([`Self::Forbidden`]) and
/// metrics-server absence ([`Self::MetricsUnavailable`]). [`Self::Stale`]
/// captures the case where the watcher reflector lost its event stream and
/// the cached state is older than the freshness contract.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ClusterError {
    /// No kubeconfig source could be resolved (no `$KUBECONFIG`, no
    /// `~/.kube/config`, no in-cluster ServiceAccount).
    #[error("no kubeconfig found")]
    KubeconfigNotFound,

    /// The Kubernetes API server could not be reached (DNS, TCP, TLS, or
    /// `/version` probe failed).
    #[error("cluster unreachable: {0}")]
    Unreachable(String),

    /// RBAC denied a list/watch on a specific resource. `namespace` is
    /// `None` for cluster-scoped resources (Nodes) or all-namespaces
    /// queries.
    #[error("forbidden: cannot access {resource}{}", match namespace {
        Some(ns) => format!(" in namespace {ns}"),
        None => String::new(),
    })]
    Forbidden {
        resource: &'static str,
        namespace: Option<String>,
    },

    /// `metrics.k8s.io/v1beta1` is not served by this cluster (metrics-server
    /// missing or disabled). Renders CPU/MEM columns as `—` in the UI.
    #[error("metrics-server unavailable on this cluster")]
    MetricsUnavailable,

    /// The cached snapshot is older than the freshness contract — the
    /// watcher reflector likely lost its event stream and the relist hasn't
    /// completed yet.
    #[error("snapshot stale (last update {since_secs}s ago)")]
    Stale { since_secs: u64 },

    /// Generic engine-reported error carrying its message verbatim.
    #[error("cluster engine error: {0}")]
    Other(String),
}

/// A resolved source for the Kubernetes client configuration.
///
/// Not serialized — kept local to the server/client process that owns the
/// engine instance. **The kubeconfig content itself never crosses the wire**
/// (cf. v0.4 plan, T-824 / T-854 anti-leak guards).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KubeconfigSource {
    /// Path resolved from `$KUBECONFIG`. May be a single file or a
    /// `:`-separated list — left raw, kube-rs unifies them.
    Env(PathBuf),
    /// Default user kubeconfig at `~/.kube/config`.
    Home(PathBuf),
    /// In-cluster service-account credentials at
    /// `/var/run/secrets/kubernetes.io/serviceaccount/`.
    InCluster,
    /// No kubeconfig found anywhere.
    None,
}

/// Abstraction over a running Kubernetes cluster (Docker Desktop, kind,
/// k3s, EKS, GKE, AKS, OpenShift, plain kubeadm, …).
///
/// Implementations MUST be safe to share across tokio tasks
/// (`Send + Sync + 'static`).
///
/// See ADR-04 (`kube-rs vs k8s-openapi direct`): the production implementation
/// `KubeEngine` (E2) wraps `kube::Client` + `kube_runtime::reflector::Store`s
/// for Pod / Node / Deployment, plus a 5 s polling loop against
/// `metrics.k8s.io/v1beta1`. `snapshot()` reads from the in-memory stores
/// and is therefore CPU-only — the network cost is borne by the watchers.
///
/// See ADR-01 (v0.3): `#[async_trait]` is used to keep the trait object-safe;
/// the Collector holds `Option<Arc<dyn ClusterEngine + Send + Sync>>`.
#[async_trait]
pub trait ClusterEngine: Send + Sync {
    /// Build a fresh snapshot of the cluster's pods, nodes and deployments
    /// from the in-memory reflector caches.
    ///
    /// Implementations MUST NOT block on network I/O here — use the watcher
    /// stream + metrics polling tasks for that. This call is on the hot
    /// path of the collector tick and budgeted to < 50 ms even with 1000+
    /// pods (cf. v0.4 plan T-816).
    async fn snapshot(&self) -> Result<KubeSnapshot, ClusterError>;

    /// Probe the API discovery for `metrics.k8s.io/v1beta1`. Result is cached
    /// by the implementation (typically 60 s) since this drives a UI badge.
    async fn metrics_available(&self) -> bool;

    /// Reported cluster fingerprint, used to label the UI.
    fn kind(&self) -> ClusterKind;

    /// `serverVersion.gitVersion` from the discovery API. `None` until the
    /// initial probe succeeds.
    fn server_version(&self) -> Option<&str>;
}

/// Pure, injectable kubeconfig detection — the real [`detect_kubeconfig`] is
/// a thin wrapper over this.
///
/// Precedence:
/// 1. `env.var("KUBECONFIG")` if non-empty.
/// 2. `home_kubeconfig` if `Some(p)` and `p.exists()`.
/// 3. `in_cluster_token` if it exists on disk.
/// 4. Otherwise [`KubeconfigSource::None`].
pub fn detect_kubeconfig_with<E: EnvLookup>(
    env: &E,
    home_kubeconfig: Option<&Path>,
    in_cluster_token: &Path,
) -> KubeconfigSource {
    if let Some(raw) = env.var("KUBECONFIG") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return KubeconfigSource::Env(PathBuf::from(trimmed));
        }
    }
    if let Some(home) = home_kubeconfig
        && home.exists()
    {
        return KubeconfigSource::Home(home.to_path_buf());
    }
    if in_cluster_token.exists() {
        return KubeconfigSource::InCluster;
    }
    KubeconfigSource::None
}

/// Resolve the production kubeconfig source using the real environment and
/// canonical paths.
///
/// * `home_kubeconfig` = `dirs::home_dir().map(|h| h.join(".kube/config"))`
/// * `in_cluster_token` = `/var/run/secrets/kubernetes.io/serviceaccount/token`
pub fn detect_kubeconfig() -> KubeconfigSource {
    use crate::container_engine::StdEnv;

    let env = StdEnv;
    let home_kubeconfig: Option<PathBuf> = dirs::home_dir().map(|h| h.join(".kube/config"));
    let in_cluster_token =
        Path::new("/var/run/secrets/kubernetes.io/serviceaccount/token");

    detect_kubeconfig_with(&env, home_kubeconfig.as_deref(), in_cluster_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs::File;
    use tempfile::tempdir;

    /// Test double for `EnvLookup`. Identical idiom to
    /// `container_engine::tests::FakeEnv` — duplicated locally to keep test
    /// modules independent (the upstream one is not `pub`).
    #[derive(Default)]
    struct FakeEnv {
        vars: HashMap<String, String>,
    }

    impl FakeEnv {
        fn with(mut self, key: &str, value: &str) -> Self {
            self.vars.insert(key.into(), value.into());
            self
        }
    }

    impl EnvLookup for FakeEnv {
        fn var(&self, name: &str) -> Option<String> {
            self.vars.get(name).cloned()
        }
    }

    // -------- detect_kubeconfig_with --------

    #[test]
    fn detect_with_env_var_returns_env() {
        let env = FakeEnv::default().with("KUBECONFIG", "/etc/k8s/admin.conf");
        let dir = tempdir().unwrap();
        let token = dir.path().join("nope-token");

        let result = detect_kubeconfig_with(&env, None, &token);
        assert_eq!(
            result,
            KubeconfigSource::Env(PathBuf::from("/etc/k8s/admin.conf"))
        );
    }

    #[test]
    fn detect_with_env_var_supports_colon_separated_list() {
        // We keep the raw string; kube-rs handles the colon-split.
        let env = FakeEnv::default().with("KUBECONFIG", "/a/config:/b/config");
        let dir = tempdir().unwrap();
        let token = dir.path().join("nope-token");

        let result = detect_kubeconfig_with(&env, None, &token);
        assert_eq!(
            result,
            KubeconfigSource::Env(PathBuf::from("/a/config:/b/config"))
        );
    }

    #[test]
    fn detect_with_empty_env_var_falls_through_to_home() {
        let env = FakeEnv::default().with("KUBECONFIG", "");
        let dir = tempdir().unwrap();
        let home_config = dir.path().join("config");
        File::create(&home_config).unwrap();
        let token = dir.path().join("nope-token");

        let result = detect_kubeconfig_with(&env, Some(&home_config), &token);
        assert_eq!(result, KubeconfigSource::Home(home_config));
    }

    #[test]
    fn detect_with_whitespace_only_env_var_falls_through() {
        let env = FakeEnv::default().with("KUBECONFIG", "   ");
        let dir = tempdir().unwrap();
        let token = dir.path().join("nope-token");

        let result = detect_kubeconfig_with(&env, None, &token);
        assert_eq!(result, KubeconfigSource::None);
    }

    #[test]
    fn detect_with_home_kubeconfig_returns_home() {
        let env = FakeEnv::default();
        let dir = tempdir().unwrap();
        let home_config = dir.path().join("config");
        File::create(&home_config).unwrap();
        let token = dir.path().join("nope-token");

        let result = detect_kubeconfig_with(&env, Some(&home_config), &token);
        assert_eq!(result, KubeconfigSource::Home(home_config));
    }

    #[test]
    fn detect_with_home_kubeconfig_missing_falls_through() {
        let env = FakeEnv::default();
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let token = dir.path().join("nope-token");

        let result = detect_kubeconfig_with(&env, Some(&missing), &token);
        assert_eq!(result, KubeconfigSource::None);
    }

    #[test]
    fn detect_with_in_cluster_token_returns_in_cluster() {
        let env = FakeEnv::default();
        let dir = tempdir().unwrap();
        let token = dir.path().join("token");
        File::create(&token).unwrap();

        let result = detect_kubeconfig_with(&env, None, &token);
        assert_eq!(result, KubeconfigSource::InCluster);
    }

    #[test]
    fn detect_with_returns_none_when_nothing_present() {
        let env = FakeEnv::default();
        let dir = tempdir().unwrap();
        let nope_home = dir.path().join("nope-home");
        let nope_token = dir.path().join("nope-token");

        let result = detect_kubeconfig_with(&env, Some(&nope_home), &nope_token);
        assert_eq!(result, KubeconfigSource::None);
    }

    #[test]
    fn detect_priority_env_beats_home_and_in_cluster() {
        // All three sources active — env wins.
        let env = FakeEnv::default().with("KUBECONFIG", "/explicit/config");
        let dir = tempdir().unwrap();
        let home = dir.path().join("config");
        File::create(&home).unwrap();
        let token = dir.path().join("token");
        File::create(&token).unwrap();

        let result = detect_kubeconfig_with(&env, Some(&home), &token);
        assert_eq!(
            result,
            KubeconfigSource::Env(PathBuf::from("/explicit/config"))
        );
    }

    #[test]
    fn detect_priority_home_beats_in_cluster() {
        // Env empty, home and in-cluster both present — home wins.
        let env = FakeEnv::default();
        let dir = tempdir().unwrap();
        let home = dir.path().join("config");
        File::create(&home).unwrap();
        let token = dir.path().join("token");
        File::create(&token).unwrap();

        let result = detect_kubeconfig_with(&env, Some(&home), &token);
        assert_eq!(result, KubeconfigSource::Home(home));
    }

    // -------- integration: real detect_kubeconfig() is callable --------

    #[test]
    fn detect_kubeconfig_does_not_panic() {
        // Just ensure the production wrapper is sound; result depends on the
        // host so we don't pin it.
        let _ = detect_kubeconfig();
    }

    // -------- ClusterError --------

    #[test]
    fn cluster_error_display_is_informative() {
        let variants: Vec<ClusterError> = vec![
            ClusterError::KubeconfigNotFound,
            ClusterError::Unreachable("dns failed".into()),
            ClusterError::Forbidden {
                resource: "pods",
                namespace: Some("kube-system".into()),
            },
            ClusterError::Forbidden {
                resource: "nodes",
                namespace: None,
            },
            ClusterError::MetricsUnavailable,
            ClusterError::Stale { since_secs: 42 },
            ClusterError::Other("kaboom".into()),
        ];
        for err in &variants {
            let msg = format!("{err}");
            assert!(!msg.is_empty(), "empty Display for {err:?}");
        }
        // Spot-check that contextual content is surfaced.
        assert!(format!("{}", variants[1]).contains("dns failed"));
        let scoped = format!("{}", variants[2]);
        assert!(scoped.contains("pods"));
        assert!(scoped.contains("kube-system"));
        let cluster_scoped = format!("{}", variants[3]);
        assert!(cluster_scoped.contains("nodes"));
        assert!(!cluster_scoped.contains("namespace"));
        assert!(format!("{}", variants[5]).contains("42"));
    }

    #[test]
    fn cluster_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ClusterError>();
    }

    // -------- ClusterEngine trait shape --------

    /// Trivially-correct stub used only to assert the trait shape compiles
    /// in a `Box<dyn ClusterEngine>` (object-safety regression guard).
    /// The real impl lives in `kube_engine.rs` (E2).
    struct StubCluster;

    #[async_trait::async_trait]
    impl ClusterEngine for StubCluster {
        async fn snapshot(&self) -> Result<KubeSnapshot, ClusterError> {
            Ok(KubeSnapshot::unavailable())
        }

        async fn metrics_available(&self) -> bool {
            false
        }

        fn kind(&self) -> ClusterKind {
            ClusterKind::Generic
        }

        fn server_version(&self) -> Option<&str> {
            None
        }
    }

    #[test]
    fn cluster_engine_is_object_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn ClusterEngine>();

        // Build through the dyn pointer — proves dyn-safety end-to-end.
        let _boxed: Box<dyn ClusterEngine + Send + Sync> = Box::new(StubCluster);
    }

    #[tokio::test]
    async fn cluster_engine_stub_returns_unavailable() {
        let stub: Box<dyn ClusterEngine + Send + Sync> = Box::new(StubCluster);
        let snap = stub.snapshot().await.expect("stub never errors");
        assert!(!snap.reachable);
        assert!(snap.pods.is_empty());
        assert_eq!(stub.kind(), ClusterKind::Generic);
        assert!(stub.server_version().is_none());
        assert!(!stub.metrics_available().await);
    }
}
