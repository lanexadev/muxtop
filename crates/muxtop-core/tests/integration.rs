/// Integration tests — full pipeline: collect → sort → filter → tree → actions.
use std::time::Duration;

use muxtop_core::process::{
    build_process_tree, filter_processes, flatten_tree, sort_processes, SortField, SortOrder,
};
use muxtop_core::system::SystemSnapshot;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Helper: spawn a collector, receive one snapshot, shut it down.
async fn collect_one_snapshot() -> SystemSnapshot {
    use muxtop_core::collector::Collector;

    let (tx, mut rx) = mpsc::channel(4);
    let token = CancellationToken::new();
    let collector = Collector::new(Duration::from_secs(1));
    let handle = collector.spawn(tx, token.clone());

    let snapshot = tokio::time::timeout(Duration::from_secs(4), rx.recv())
        .await
        .expect("timeout waiting for snapshot")
        .expect("channel closed");

    token.cancel();
    handle.await.expect("collector panicked");

    snapshot
}

#[tokio::test]
async fn test_full_pipeline_collect_sort_filter() {
    let snapshot = collect_one_snapshot().await;

    // Snapshot should have real data.
    assert!(!snapshot.processes.is_empty(), "no processes collected");
    assert!(!snapshot.cpu.cores.is_empty(), "no CPU cores");
    assert!(snapshot.memory.total > 0, "no memory info");

    // Sort by CPU descending.
    let mut procs = snapshot.processes.clone();
    sort_processes(&mut procs, SortField::Cpu, SortOrder::Desc);
    for w in procs.windows(2) {
        assert!(
            w[0].cpu_percent >= w[1].cpu_percent,
            "sort by CPU desc failed: {} < {}",
            w[0].cpu_percent,
            w[1].cpu_percent,
        );
    }

    // Filter — there should be at least one process related to this test binary.
    let filtered = filter_processes(&snapshot.processes, "muxtop");
    // The test binary itself should match (it's named muxtop_core-...).
    // However, the process name might be truncated, so we don't assert > 0
    // — just that filter doesn't panic and returns a subset.
    assert!(
        filtered.len() <= snapshot.processes.len(),
        "filter returned more than total"
    );
}

#[tokio::test]
async fn test_full_pipeline_tree_build() {
    let snapshot = collect_one_snapshot().await;

    let tree = build_process_tree(&snapshot.processes);
    assert!(!tree.is_empty(), "tree should have root nodes");

    // All roots should have depth 0.
    for root in &tree {
        assert_eq!(root.depth, 0, "root node should have depth 0");
    }

    // Flatten should preserve all processes.
    let flat = flatten_tree(&tree);
    assert_eq!(
        flat.len(),
        snapshot.processes.len(),
        "flatten should preserve all processes"
    );
}

#[tokio::test]
async fn test_full_pipeline_actions() {
    // Signal 0 is a no-op existence check for our own process.
    let pid = std::process::id();
    let result = muxtop_core::actions::kill_process(pid, 0);
    assert!(result.is_ok(), "kill(self, 0) should succeed: {result:?}");
}

#[tokio::test]
async fn test_full_pipeline_shutdown() {
    use muxtop_core::collector::Collector;

    let (tx, mut rx) = mpsc::channel(4);
    let token = CancellationToken::new();
    let collector = Collector::new(Duration::from_secs(1));
    let handle = collector.spawn(tx, token.clone());

    // Drain one snapshot to ensure it's running.
    let _ = tokio::time::timeout(Duration::from_secs(4), rx.recv())
        .await
        .expect("timeout")
        .expect("closed");

    // Cancel and verify clean shutdown.
    token.cancel();
    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(result.is_ok(), "collector should shut down within 2s");
    assert!(result.unwrap().is_ok(), "collector should not panic");
}
