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

use crate::container_engine::EnvLookup;

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
}
