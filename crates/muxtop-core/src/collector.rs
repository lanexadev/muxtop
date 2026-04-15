// Async collection loop (tokio task, 1Hz refresh).

use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::system::SystemSnapshot;

pub struct Collector {
    sys: sysinfo::System,
    interval: Duration,
}

impl Collector {
    pub fn new(interval: Duration) -> Self {
        Self {
            sys: sysinfo::System::new_all(),
            interval,
        }
    }

    /// Spawn the collector as a background tokio task.
    /// Returns a JoinHandle that completes when the token is cancelled.
    pub fn spawn(
        self,
        tx: mpsc::Sender<SystemSnapshot>,
        token: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(Self::run(self, tx, token))
    }

    async fn run(mut self, tx: mpsc::Sender<SystemSnapshot>, token: CancellationToken) {
        let mut interval = tokio::time::interval(self.interval);
        // Don't burst-fire missed ticks when system is under load.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // First tick completes immediately — do an initial refresh to seed
        // the sysinfo delta baseline (needed for accurate CPU percentages).
        interval.tick().await;
        self.sys.refresh_all();

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.sys.refresh_all();
                    let snapshot = SystemSnapshot::collect(&self.sys);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as IoWrite;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// Helper: append a log line to /tmp/muxtop-test-collector.log
    fn log(msg: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/muxtop-test-collector.log")
            .unwrap();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        writeln!(f, "[{ts}] [pid={}] {msg}", std::process::id()).unwrap();
    }

    /// Spawn a collector with a 1-second interval and a fresh cancellation token.
    fn make_collector(
        cap: usize,
    ) -> (
        mpsc::Receiver<SystemSnapshot>,
        tokio::task::JoinHandle<()>,
        CancellationToken,
    ) {
        log(&format!(
            "make_collector: creating Collector(interval=1s, channel_cap={cap})"
        ));
        let (tx, rx) = mpsc::channel(cap);
        let token = CancellationToken::new();
        log("make_collector: creating Collector::new()...");
        let collector = Collector::new(Duration::from_secs(1));
        log("make_collector: Collector created. Spawning task...");
        let handle = collector.spawn(tx, token.clone());
        log("make_collector: task spawned");
        (rx, handle, token)
    }

    /// Receive at least 2 snapshots within 4 seconds.
    #[tokio::test]
    async fn test_collector_produces_snapshots() {
        log("=== test_collector_produces_snapshots START ===");
        let (mut rx, handle, token) = make_collector(4);

        let mut count = 0usize;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);

        loop {
            log(&format!("  waiting for snapshot #{}", count + 1));
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(snap)) => {
                    count += 1;
                    log(&format!(
                        "  received snapshot #{count}: processes={}, cores={}, mem_total={}",
                        snap.processes.len(),
                        snap.cpu.cores.len(),
                        snap.memory.total
                    ));
                    if count >= 2 {
                        break;
                    }
                }
                Ok(None) => {
                    log("  ERROR: channel closed prematurely");
                    panic!("channel closed before receiving 2 snapshots");
                }
                Err(_) => {
                    log(&format!(
                        "  ERROR: timeout after receiving {count} snapshots"
                    ));
                    panic!("timeout: only received {count} snapshots within 4s");
                }
            }
        }

        log("  cancelling token...");
        token.cancel();
        log("  awaiting handle...");
        handle.await.expect("collector task panicked");
        log(&format!(
            "=== test_collector_produces_snapshots END (count={count}) ==="
        ));
        assert!(count >= 2, "expected at least 2 snapshots, got {count}");
    }

    /// Receive one snapshot and assert it has process and CPU core data.
    #[tokio::test]
    async fn test_collector_snapshot_has_data() {
        log("=== test_collector_snapshot_has_data START ===");
        let (mut rx, handle, token) = make_collector(4);

        log("  waiting for first snapshot...");
        let snapshot = tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .expect("timeout waiting for snapshot")
            .expect("channel closed before first snapshot");

        log(&format!(
            "  snapshot received: processes={}, cores={}, cpu_global={:.1}%, mem_used={}/{}",
            snapshot.processes.len(),
            snapshot.cpu.cores.len(),
            snapshot.cpu.global_usage,
            snapshot.memory.used,
            snapshot.memory.total,
        ));

        log("  cancelling token...");
        token.cancel();
        log("  awaiting handle...");
        handle.await.expect("collector task panicked");

        assert!(
            !snapshot.processes.is_empty(),
            "snapshot should contain processes"
        );
        assert!(
            !snapshot.cpu.cores.is_empty(),
            "snapshot should contain CPU cores"
        );
        log("=== test_collector_snapshot_has_data END ===");
    }

    /// Cancel the token after 500 ms; the JoinHandle must complete within 2 s.
    #[tokio::test]
    async fn test_collector_graceful_shutdown() {
        log("=== test_collector_graceful_shutdown START ===");
        let (mut rx, handle, token) = make_collector(4);

        // Drain in the background so the channel doesn't fill up.
        tokio::spawn(async move { while rx.recv().await.is_some() {} });

        log("  sleeping 500ms before cancel...");
        tokio::time::sleep(Duration::from_millis(500)).await;
        log("  cancelling token...");
        token.cancel();

        log("  awaiting handle with 2s timeout...");
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("collector did not shut down within 2s")
            .expect("collector task panicked");
        log("=== test_collector_graceful_shutdown END ===");
    }

    /// Channel cap 1, never read — run for 2s, then cancel. Must not panic.
    #[tokio::test]
    async fn test_collector_channel_backpressure() {
        log("=== test_collector_channel_backpressure START ===");
        let (tx, _rx) = mpsc::channel::<SystemSnapshot>(1);
        let token = CancellationToken::new();
        log("  creating collector...");
        let collector = Collector::new(Duration::from_secs(1));
        log("  spawning collector (channel will be full)...");
        let handle = collector.spawn(tx, token.clone());

        log("  sleeping 2s with full channel...");
        tokio::time::sleep(Duration::from_secs(2)).await;
        log("  cancelling token...");
        token.cancel();

        log("  awaiting handle with 2s timeout...");
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("collector did not shut down within 2s after backpressure test")
            .expect("collector task panicked");
        log("=== test_collector_channel_backpressure END ===");
    }

    /// Two consecutive snapshots should be ~1s apart (tolerance ±500ms).
    #[tokio::test]
    async fn test_collector_respects_interval() {
        log("=== test_collector_respects_interval START ===");
        let (mut rx, handle, token) = make_collector(4);

        log("  waiting for first snapshot...");
        let first = tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .expect("timeout waiting for first snapshot")
            .expect("channel closed before first snapshot");
        log(&format!("  first snapshot at {:?}", first.timestamp));

        log("  waiting for second snapshot...");
        let second = tokio::time::timeout(Duration::from_secs(4), rx.recv())
            .await
            .expect("timeout waiting for second snapshot")
            .expect("channel closed before second snapshot");
        log(&format!("  second snapshot at {:?}", second.timestamp));

        log("  cancelling token...");
        token.cancel();
        log("  awaiting handle...");
        handle.await.expect("collector task panicked");

        let gap = second.timestamp.duration_since(first.timestamp);
        log(&format!("  interval gap: {gap:?}"));
        let min = Duration::from_millis(500);
        let max = Duration::from_millis(1500);

        assert!(gap >= min && gap <= max, "expected gap ~1s, got {:?}", gap);
        log("=== test_collector_respects_interval END ===");
    }
}
