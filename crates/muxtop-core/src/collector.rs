// Async collection loop (tokio task).
//
// Three concurrent loops:
//  - System loop (configurable tick, default 1 Hz) — produces `SystemSnapshot`s
//    to the mpsc channel. Each tick reads the latest container + kube
//    snapshots (if any) and embeds them into the emitted `SystemSnapshot`.
//  - Container loop (0.5 Hz, only if an engine is provided) — calls
//    `ContainerEngine::list_and_stats()` and writes the result into a shared
//    `Arc<Mutex<Option<ContainersSnapshot>>>`. On error, publishes
//    `ContainersSnapshot::unavailable()` so the UI can render the notice.
//  - Cluster loop (0.2 Hz, only if a cluster engine is provided) — calls
//    `ClusterEngine::snapshot()` and writes the result into a shared
//    `Arc<Mutex<Option<KubeSnapshot>>>`. On error, publishes
//    `KubeSnapshot::unavailable()`. Note that the engine's own internal
//    poll task drives kube-rs LIST/metrics traffic; this collector loop
//    just samples the engine's already-cached state every 5 s.
//
// All loops share a single `CancellationToken`. Shutdown is cooperative.

use std::sync::Arc;
use std::time::Duration;

use sysinfo::{MemoryRefreshKind, ProcessRefreshKind, ProcessesToUpdate};
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cluster_engine::ClusterEngine;
use crate::container_engine::ContainerEngine;
use crate::containers::ContainersSnapshot;
use crate::kube::KubeSnapshot;
use crate::system::SystemSnapshot;

/// Container stats refresh cadence — independent from the system tick.
///
/// 0.5 Hz (2 s) keeps the HTTP cost below ~10 % of a 0.5 Hz budget at 100
/// containers (see forge/23-epic-containers ADR-05).
const CONTAINER_INTERVAL: Duration = Duration::from_secs(2);

/// Cluster snapshot sampling cadence — matches the kube-rs internal poll
/// task interval (see ADR-05 v0.4 in `forge/32-v04-kubernetes-epics/`).
const CLUSTER_INTERVAL: Duration = Duration::from_secs(5);

pub struct Collector {
    sys: sysinfo::System,
    networks: sysinfo::Networks,
    interval: Duration,
    container_engine: Option<Arc<dyn ContainerEngine + Send + Sync>>,
    cluster_engine: Option<Arc<dyn ClusterEngine + Send + Sync>>,
}

impl Collector {
    /// Create a collector that only gathers system snapshots.
    pub fn new(interval: Duration) -> Self {
        Self::with_engines(interval, None, None)
    }

    /// Create a collector that also polls a container engine.
    ///
    /// `container_engine = None` disables the container path entirely — the
    /// emitted `SystemSnapshot.containers` stays `None`.
    pub fn with_container_engine(
        interval: Duration,
        container_engine: Option<Arc<dyn ContainerEngine + Send + Sync>>,
    ) -> Self {
        Self::with_engines(interval, container_engine, None)
    }

    /// Create a collector with both container and cluster engines.
    ///
    /// Either side can be `None` independently. The emitted snapshot's
    /// `containers` / `kube` fields stay `None` for the disabled side.
    pub fn with_engines(
        interval: Duration,
        container_engine: Option<Arc<dyn ContainerEngine + Send + Sync>>,
        cluster_engine: Option<Arc<dyn ClusterEngine + Send + Sync>>,
    ) -> Self {
        Self {
            sys: sysinfo::System::new_all(),
            networks: sysinfo::Networks::new_with_refreshed_list(),
            interval,
            container_engine,
            cluster_engine,
        }
    }

    /// Spawn the collector as one (or two) background tokio tasks.
    ///
    /// The returned `JoinHandle` completes when the SYSTEM loop exits; the
    /// container loop (if present) is driven by the same cancellation token
    /// and joined internally before the handle completes.
    pub fn spawn(
        self,
        tx: mpsc::Sender<SystemSnapshot>,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(Self::run(self, tx, token))
    }

    async fn run(mut self, tx: mpsc::Sender<SystemSnapshot>, token: CancellationToken) {
        let mut interval = tokio::time::interval(self.interval);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // First tick completes immediately — seed the sysinfo delta baseline
        // so CPU % and network counters report useful values from tick #2.
        interval.tick().await;
        self.sys.refresh_all();
        self.networks.refresh(true);

        // Shared container + kube snapshot slots — written by their
        // respective polling tasks, read by the system loop on every emit.
        let last_containers: Arc<Mutex<Option<ContainersSnapshot>>> = Arc::new(Mutex::new(None));
        let last_kube: Arc<Mutex<Option<KubeSnapshot>>> = Arc::new(Mutex::new(None));

        // Spawn the container polling loop if an engine is configured.
        let container_task = self.container_engine.take().map(|engine| {
            spawn_container_loop(engine, Arc::clone(&last_containers), token.clone())
        });
        // Spawn the cluster polling loop if a kube engine is configured.
        let cluster_task = self
            .cluster_engine
            .take()
            .map(|engine| spawn_cluster_loop(engine, Arc::clone(&last_kube), token.clone()));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // PERF-L2: targeted sysinfo refresh — `refresh_all` rebuilds
                    // disk lists, components, users and a host of other tables
                    // that the TUI never consumes. Limit refresh to the three
                    // subsystems we actually render (memory, CPU usage,
                    // processes). Network is refreshed via `Networks` below.
                    self.sys.refresh_memory_specifics(MemoryRefreshKind::everything());
                    self.sys.refresh_cpu_usage();
                    self.sys.refresh_processes_specifics(
                        ProcessesToUpdate::All,
                        true,
                        ProcessRefreshKind::everything(),
                    );
                    self.networks.refresh(false);

                    let containers = last_containers.lock().await.clone();
                    let kube = last_kube.lock().await.clone();
                    let snapshot =
                        SystemSnapshot::collect(&self.sys, &self.networks, containers, kube);

                    match tx.try_send(snapshot) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            tracing::trace!("channel full, dropping snapshot");
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            tracing::debug!("channel closed, stopping collector");
                            break;
                        }
                    }
                }
                _ = token.cancelled() => {
                    tracing::debug!("collector shutting down");
                    break;
                }
            }
        }

        // Wait for the container + cluster tasks to wind down so the
        // JoinHandle reflects total shutdown.
        if let Some(handle) = container_task {
            let _ = handle.await;
        }
        if let Some(handle) = cluster_task {
            let _ = handle.await;
        }
    }
}

/// Background loop polling the cluster engine at 0.2 Hz (5 s). Writes into
/// the shared slot; publishes `KubeSnapshot::unavailable()` on failure so
/// the UI can render the "no cluster" state.
///
/// Note the engine's own kube-rs poll task drives the actual LIST + metrics
/// traffic; this loop just samples the engine's cached state and forwards
/// it through the snapshot pipeline.
fn spawn_cluster_loop(
    engine: Arc<dyn ClusterEngine + Send + Sync>,
    slot: Arc<Mutex<Option<KubeSnapshot>>>,
    token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLUSTER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match engine.snapshot().await {
                        Ok(snapshot) => {
                            *slot.lock().await = Some(snapshot);
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "cluster engine failed");
                            *slot.lock().await = Some(KubeSnapshot::unavailable());
                        }
                    }
                }
                _ = token.cancelled() => {
                    tracing::debug!("cluster loop shutting down");
                    break;
                }
            }
        }
    })
}

/// Background loop polling the container engine at 0.5 Hz. Writes into the
/// shared slot; publishes `ContainersSnapshot::unavailable()` on failure so
/// the UI can render the "no daemon" state.
fn spawn_container_loop(
    engine: Arc<dyn ContainerEngine + Send + Sync>,
    slot: Arc<Mutex<Option<ContainersSnapshot>>>,
    token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CONTAINER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match engine.list_and_stats().await {
                        Ok(containers) => {
                            let snapshot = ContainersSnapshot {
                                engine: engine.kind(),
                                daemon_up: true,
                                containers,
                            };
                            *slot.lock().await = Some(snapshot);
                        }
                        Err(err) => {
                            tracing::warn!(error = %err, "container engine failed");
                            *slot.lock().await = Some(ContainersSnapshot::unavailable());
                        }
                    }
                }
                _ = token.cancelled() => {
                    tracing::debug!("container loop shutting down");
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container_engine::EngineError;
    use crate::containers::{ContainerSnapshot, ContainerState, EngineKind};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::Duration;

    // ─── mock engine ──────────────────────────────────────────────────────

    /// Test double for `ContainerEngine`: returns a static list and counts
    /// invocations. `fail_mode = true` makes `list_and_stats` return an
    /// error so we can exercise the degraded path.
    struct MockEngine {
        kind: EngineKind,
        call_count: AtomicUsize,
        fail_mode: AtomicBool,
    }

    impl MockEngine {
        fn new(kind: EngineKind) -> Self {
            Self {
                kind,
                call_count: AtomicUsize::new(0),
                fail_mode: AtomicBool::new(false),
            }
        }

        fn sample_container() -> ContainerSnapshot {
            ContainerSnapshot {
                id: "abc123".into(),
                id_full: "abc123".to_string() + &"0".repeat(58),
                name: "mock-svc".into(),
                image: "mock:latest".into(),
                state: ContainerState::Running,
                status_text: "Up 1 minute".into(),
                cpu_pct: 2.5,
                mem_used_bytes: 128 * 1024 * 1024,
                mem_limit_bytes: 512 * 1024 * 1024,
                net_rx_bytes: 1024,
                net_tx_bytes: 512,
                block_read_bytes: 0,
                block_write_bytes: 0,
                started_at_ms: 1_700_000_000_000,
            }
        }
    }

    #[async_trait]
    impl ContainerEngine for MockEngine {
        async fn list_and_stats(&self) -> Result<Vec<ContainerSnapshot>, EngineError> {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            if self.fail_mode.load(Ordering::Relaxed) {
                return Err(EngineError::ConnectFailed("mock failure".into()));
            }
            Ok(vec![Self::sample_container()])
        }
        async fn stop(&self, _id: &str, _t: Option<u64>) -> Result<(), EngineError> {
            Ok(())
        }
        async fn kill(&self, _id: &str) -> Result<(), EngineError> {
            Ok(())
        }
        async fn restart(&self, _id: &str) -> Result<(), EngineError> {
            Ok(())
        }
        fn kind(&self) -> EngineKind {
            self.kind
        }
    }

    fn make_collector(
        cap: usize,
    ) -> (
        mpsc::Receiver<SystemSnapshot>,
        tokio::task::JoinHandle<()>,
        CancellationToken,
    ) {
        let (tx, rx) = mpsc::channel(cap);
        let token = CancellationToken::new();
        let collector = Collector::new(Duration::from_secs(1));
        let handle = collector.spawn(tx, token.clone());
        (rx, handle, token)
    }

    // ─── Pre-existing system-only tests ───────────────────────────────────

    #[tokio::test]
    async fn test_collector_produces_snapshots() {
        let (mut rx, handle, token) = make_collector(4);

        let mut count = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);

        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(_)) => {
                    count += 1;
                    if count >= 2 {
                        break;
                    }
                }
                Ok(None) => panic!("channel closed before receiving 2 snapshots"),
                Err(_) => panic!("timeout: only received {count} snapshots within 4s"),
            }
        }

        token.cancel();
        handle.await.expect("collector task panicked");
        assert!(count >= 2, "expected at least 2 snapshots, got {count}");
    }

    #[tokio::test]
    async fn test_collector_snapshot_has_data() {
        let (mut rx, handle, token) = make_collector(4);

        let snapshot = tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .expect("timeout waiting for snapshot")
            .expect("channel closed before first snapshot");

        token.cancel();
        handle.await.expect("collector task panicked");

        assert!(
            !snapshot.processes.is_empty(),
            "snapshot should contain processes"
        );
        assert!(
            !snapshot.cpu.cores.is_empty(),
            "snapshot should contain CPU cores"
        );
        // With no engine, containers stays None.
        assert!(snapshot.containers.is_none());
    }

    #[tokio::test]
    async fn test_collector_graceful_shutdown() {
        let (mut rx, handle, token) = make_collector(4);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        tokio::time::sleep(Duration::from_millis(500)).await;
        token.cancel();

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("collector did not shut down within 2s")
            .expect("collector task panicked");
    }

    #[tokio::test]
    async fn test_collector_channel_backpressure() {
        let (tx, _rx) = mpsc::channel::<SystemSnapshot>(1);
        let token = CancellationToken::new();
        let collector = Collector::new(Duration::from_secs(1));
        let handle = collector.spawn(tx, token.clone());

        tokio::time::sleep(Duration::from_secs(2)).await;
        token.cancel();

        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("collector did not shut down within 2s after backpressure test")
            .expect("collector task panicked");
    }

    #[tokio::test]
    async fn test_collector_respects_interval() {
        let (mut rx, handle, token) = make_collector(4);

        let first = tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .expect("timeout waiting for first snapshot")
            .expect("channel closed before first snapshot");

        let second = tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .expect("timeout waiting for second snapshot")
            .expect("channel closed before second snapshot");

        token.cancel();
        handle.await.expect("collector task panicked");

        let gap_ms = second.timestamp_ms.saturating_sub(first.timestamp_ms);
        assert!(
            (500..=1500).contains(&gap_ms),
            "expected gap ~1000ms, got {gap_ms}ms"
        );
    }

    // ─── Container integration tests (E4) ─────────────────────────────────

    /// With a MockEngine injected, after ~2 s the containers field should be
    /// populated with a successful snapshot.
    #[tokio::test]
    async fn test_collector_populates_containers_with_engine() {
        let engine: Arc<dyn ContainerEngine + Send + Sync> =
            Arc::new(MockEngine::new(EngineKind::Docker));
        let (tx, mut rx) = mpsc::channel(8);
        let token = CancellationToken::new();
        let collector = Collector::with_container_engine(Duration::from_millis(300), Some(engine));
        let handle = collector.spawn(tx, token.clone());

        // Drain snapshots for up to 5 s, looking for one where `containers` is
        // populated with a running container. The container task fires on a
        // 2 s cadence, so the first populated snapshot arrives around the 2 s
        // mark.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut populated: Option<SystemSnapshot> = None;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(snap)) => {
                    if snap.containers.is_some() {
                        populated = Some(snap);
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        token.cancel();
        // Don't strictly require the handle to join within the cancel deadline
        // — the container loop may have been mid-tick — but it should have
        // finished by the time the test teardown runs.
        let _ = tokio::time::timeout(Duration::from_secs(3), handle).await;

        let snap = populated.expect("no snapshot with containers populated within 5 s");
        let cs = snap.containers.unwrap();
        assert!(cs.daemon_up);
        assert_eq!(cs.engine, EngineKind::Docker);
        assert_eq!(cs.containers.len(), 1);
        assert_eq!(cs.containers[0].name, "mock-svc");
    }

    /// When the engine fails, the container snapshot should be the
    /// `unavailable()` sentinel, not an absence — so the UI knows to render
    /// the "no daemon" state explicitly.
    #[tokio::test]
    async fn test_collector_publishes_unavailable_on_engine_error() {
        let engine = Arc::new(MockEngine::new(EngineKind::Docker));
        engine.fail_mode.store(true, Ordering::Relaxed);
        let engine_dyn: Arc<dyn ContainerEngine + Send + Sync> = engine.clone();
        let (tx, mut rx) = mpsc::channel(8);
        let token = CancellationToken::new();
        let collector =
            Collector::with_container_engine(Duration::from_millis(300), Some(engine_dyn));
        let handle = collector.spawn(tx, token.clone());

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut observed: Option<SystemSnapshot> = None;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(snap)) => {
                    if snap.containers.is_some() {
                        observed = Some(snap);
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        token.cancel();
        let _ = tokio::time::timeout(Duration::from_secs(3), handle).await;

        let snap = observed.expect("no containers-populated snapshot within 5 s");
        let cs = snap.containers.unwrap();
        assert!(!cs.daemon_up);
        assert!(cs.containers.is_empty());
        assert_eq!(cs.engine, EngineKind::Unknown);
        // Ensure the engine was actually called.
        assert!(engine.call_count.load(Ordering::Relaxed) >= 1);
    }

    /// Sanity: without an engine, the published snapshots keep `containers =
    /// None` forever. Guards against accidentally defaulting to `Some(..)`.
    #[tokio::test]
    async fn test_collector_without_engine_keeps_containers_none() {
        let (mut rx, handle, token) = make_collector(4);

        for _ in 0..3 {
            if let Some(snap) = tokio::time::timeout(Duration::from_secs(4), rx.recv())
                .await
                .expect("timeout")
            {
                assert!(
                    snap.containers.is_none(),
                    "unexpected containers snapshot without engine"
                );
            } else {
                break;
            }
        }

        token.cancel();
        handle.await.expect("collector task panicked");
    }
}
