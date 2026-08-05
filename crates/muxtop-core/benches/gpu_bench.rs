//! Criterion benchmarks for the v0.5 GPU pipeline.
//!
//! Two things are worth measuring, and neither is the vendor query itself —
//! that one is dominated by the driver and cannot be benchmarked
//! deterministically on a CI runner with no GPU:
//!
//! 1. **`CompositeGpuEngine` merge** — runs on every 1 Hz tick and is the
//!    only muxtop-authored code on the GPU hot path. It reindexes devices and
//!    remaps process→device pointers, so its cost scales with the number of
//!    GPU processes, not just devices. A machine running 8 GPUs with 64
//!    inference workers is the shape to hold the line on.
//! 2. **`resolve_process_names`** — called from `SystemSnapshot::collect`
//!    once per tick, on the same thread that builds the process table.
//!
//! Run with:
//!
//! ```sh
//! cargo bench -p muxtop-core --bench gpu_bench
//! ```

use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, criterion_group, criterion_main};

use muxtop_core::gpu::{
    GpuBackend, GpuDeviceSnapshot, GpuProcessKind, GpuProcessSnapshot, GpuVendor, GpusSnapshot,
};
use muxtop_core::gpu_engine::{CompositeGpuEngine, GpuEngine, GpuError};

/// Backend replaying a pre-built snapshot, so the benchmark measures the
/// merge rather than any driver I/O.
struct StaticBackend {
    snapshot: GpusSnapshot,
}

#[async_trait::async_trait]
impl GpuEngine for StaticBackend {
    async fn snapshot(&self) -> Result<GpusSnapshot, GpuError> {
        Ok(self.snapshot.clone())
    }
    fn backend(&self) -> GpuBackend {
        GpuBackend::Nvml
    }
}

fn synth_device(index: u32) -> GpuDeviceSnapshot {
    GpuDeviceSnapshot {
        index,
        vendor: GpuVendor::Nvidia,
        backend: GpuBackend::Nvml,
        name: format!("NVIDIA Synthetic {index}"),
        bus_id: format!("0000:{:02x}:00.0", index + 1),
        driver_version: Some("560.35.03".into()),
        utilization_pct: Some((index % 100) as f32),
        mem_utilization_pct: Some((index % 50) as f32),
        mem_used_bytes: Some(6 * 1024 * 1024 * 1024),
        mem_total_bytes: Some(24 * 1024 * 1024 * 1024),
        temperature_c: Some(60.0),
        power_watts: Some(210.0),
        power_limit_watts: Some(450.0),
        graphics_clock_mhz: Some(2520),
        memory_clock_mhz: Some(10501),
        fan_pct: Some(40.0),
        encoder_pct: Some(0.0),
        decoder_pct: Some(0.0),
        supports_process_stats: true,
    }
}

fn synth_process(pid: u32, device_index: u32) -> GpuProcessSnapshot {
    GpuProcessSnapshot {
        pid,
        device_index,
        // Empty, as a real backend leaves it — resolution is the collector's
        // job and is benchmarked separately below.
        name: String::new(),
        kind: GpuProcessKind::Compute,
        mem_bytes: Some(3 * 1024 * 1024 * 1024),
    }
}

/// One backend carrying `devices` devices and `procs_per_device` processes
/// on each.
fn synth_snapshot(devices: u32, procs_per_device: u32) -> GpusSnapshot {
    let mut processes = Vec::new();
    for device_index in 0..devices {
        for p in 0..procs_per_device {
            processes.push(synth_process(device_index * 1000 + p, device_index));
        }
    }
    GpusSnapshot {
        backends: vec![GpuBackend::Nvml],
        available: true,
        devices: (0..devices).map(synth_device).collect(),
        processes,
        detail: String::new(),
    }
}

fn composite_of(backends: usize, devices: u32, procs_per_device: u32) -> CompositeGpuEngine {
    let children: Vec<Arc<dyn GpuEngine + Send + Sync>> = (0..backends)
        .map(|_| {
            Arc::new(StaticBackend {
                snapshot: synth_snapshot(devices, procs_per_device),
            }) as Arc<dyn GpuEngine + Send + Sync>
        })
        .collect();
    CompositeGpuEngine::new(children)
}

fn bench_composite_merge(c: &mut Criterion) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("runtime");

    // The common desktop case: one backend, one GPU, a handful of clients.
    let single = composite_of(1, 1, 4);
    c.bench_function("gpu_composite/1_backend_1_device_4_procs", |b| {
        b.iter(|| {
            let snap = runtime.block_on(single.snapshot()).unwrap();
            black_box(snap);
        });
    });

    // Mixed-vendor laptop: two backends, one device each.
    let mixed = composite_of(2, 1, 4);
    c.bench_function("gpu_composite/2_backends_2_devices_8_procs", |b| {
        b.iter(|| {
            let snap = runtime.block_on(mixed.snapshot()).unwrap();
            black_box(snap);
        });
    });

    // Inference box: 8 GPUs, 8 workers each — the shape where the
    // process→device remap actually costs something.
    let dense = composite_of(1, 8, 8);
    c.bench_function("gpu_composite/1_backend_8_devices_64_procs", |b| {
        b.iter(|| {
            let snap = runtime.block_on(dense.snapshot()).unwrap();
            black_box(snap);
        });
    });
}

fn bench_resolve_process_names(c: &mut Criterion) {
    let snapshot = synth_snapshot(8, 8);

    c.bench_function("gpu_resolve_names/64_procs", |b| {
        b.iter(|| {
            let mut snap = snapshot.clone();
            // Stand-in for the sysinfo lookup: the benchmark measures the
            // traversal, not the process-table implementation.
            snap.resolve_process_names(|pid| Some(format!("worker-{pid}")));
            black_box(snap);
        });
    });
}

criterion_group!(benches, bench_composite_merge, bench_resolve_process_names);
criterion_main!(benches);
