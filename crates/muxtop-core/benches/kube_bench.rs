//! Criterion benchmark for the Kube snapshot conversion pipeline.
//!
//! v0.4 plan T-816 reserved `kube_snapshot/1000_pods < 50 ms` as the perf
//! contract for the Pod conversion path. The benchmark synthesises 1000
//! `k8s_openapi::api::core::v1::Pod` objects with realistic spread
//! (running / pending / crashloop / various restart counts and ages),
//! optionally seeds the metrics cache, and times the
//! `build_kube_snapshot_for_bench` entry point — which runs the same
//! `pod_to_snapshot` conversion `KubeEngine::snapshot` uses on the hot
//! path.
//!
//! Run with:
//!
//! ```sh
//! cargo bench -p muxtop-core --bench kube_bench
//! ```

use std::collections::HashMap;
use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use serde_json::json;

use k8s_openapi::api::core::v1::Pod;

use muxtop_core::kube_engine::build_kube_snapshot_for_bench;

fn synth_pod(i: usize) -> Pod {
    let phase = match i % 5 {
        0 => "Running",
        1 => "Pending",
        2 => "Succeeded",
        3 => "Failed",
        _ => "Running",
    };
    let crashloop = i.is_multiple_of(17);
    let restarts = (i % 8) as i32;
    let waiting = if crashloop {
        json!({ "waiting": { "reason": "CrashLoopBackOff" } })
    } else {
        json!({ "running": {} })
    };
    let value = json!({
        "metadata": {
            "namespace": format!("ns-{}", i % 10),
            "name": format!("pod-{i:05}"),
            "creationTimestamp": "2026-04-26T00:00:00Z"
        },
        "spec": { "nodeName": format!("node-{}", i % 12) },
        "status": {
            "phase": phase,
            "qosClass": match i % 3 { 0 => "Guaranteed", 1 => "Burstable", _ => "BestEffort" },
            "containerStatuses": [{
                "name": "main",
                "ready": phase == "Running",
                "restartCount": restarts,
                "image": "nginx:1.27",
                "imageID": "",
                "state": waiting
            }]
        }
    });
    serde_json::from_value(value).expect("synthesised pod must parse")
}

fn make_pods(n: usize) -> Vec<Pod> {
    (0..n).map(synth_pod).collect()
}

fn make_metrics(n: usize) -> HashMap<(String, String), (u32, u64)> {
    (0..n)
        .map(|i| {
            (
                (format!("ns-{}", i % 10), format!("pod-{i:05}")),
                (
                    (i * 13 % 2_000) as u32,
                    (128 + (i % 512) as u64) * 1024 * 1024,
                ),
            )
        })
        .collect()
}

fn bench_kube_snapshot_1000(c: &mut Criterion) {
    let pods = make_pods(1_000);
    let metrics = make_metrics(1_000);
    c.bench_function("kube_snapshot/1000_pods/with_metrics", |b| {
        b.iter(|| {
            let snap = build_kube_snapshot_for_bench(pods.clone(), metrics.clone());
            black_box(snap);
        });
    });
}

fn bench_kube_snapshot_1000_no_metrics(c: &mut Criterion) {
    let pods = make_pods(1_000);
    c.bench_function("kube_snapshot/1000_pods/no_metrics", |b| {
        b.iter(|| {
            let snap = build_kube_snapshot_for_bench(pods.clone(), HashMap::new());
            black_box(snap);
        });
    });
}

fn bench_kube_snapshot_100(c: &mut Criterion) {
    let pods = make_pods(100);
    let metrics = make_metrics(100);
    c.bench_function("kube_snapshot/100_pods/with_metrics", |b| {
        b.iter(|| {
            let snap = build_kube_snapshot_for_bench(pods.clone(), metrics.clone());
            black_box(snap);
        });
    });
}

criterion_group!(
    benches,
    bench_kube_snapshot_100,
    bench_kube_snapshot_1000,
    bench_kube_snapshot_1000_no_metrics
);
criterion_main!(benches);
