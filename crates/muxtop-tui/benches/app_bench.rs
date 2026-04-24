use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use muxtop_core::process::ProcessInfo;
use muxtop_core::system::{
    CoreSnapshot, CpuSnapshot, LoadSnapshot, MemorySnapshot, SystemSnapshot,
};
use muxtop_tui::AppState;
use muxtop_tui::app::PaletteState;

fn make_processes(n: usize) -> Vec<ProcessInfo> {
    (0..n)
        .map(|i| ProcessInfo {
            pid: i as u32 + 1,
            parent_pid: if i == 0 {
                None
            } else {
                Some((i / 3) as u32 + 1)
            },
            name: format!("proc-{i}"),
            command: format!("/usr/bin/proc-{i} --flag"),
            user: format!("user{}", i % 10),
            cpu_percent: (i as f32 * 7.3) % 100.0,
            memory_bytes: (i as u64 * 1_048_576) % 16_000_000_000,
            memory_percent: ((i as f64 * 0.1) % 100.0) as f32,
            status: "Running".to_string(),
        })
        .collect()
}

fn make_snapshot(n: usize) -> SystemSnapshot {
    SystemSnapshot {
        cpu: CpuSnapshot {
            global_usage: 42.0,
            cores: (0..8)
                .map(|i| CoreSnapshot {
                    name: format!("cpu{i}"),
                    usage: (i as f32 * 12.5) % 100.0,
                    frequency: 3600,
                })
                .collect(),
        },
        memory: MemorySnapshot {
            total: 16_000_000_000,
            used: 8_000_000_000,
            available: 8_000_000_000,
            swap_total: 4_000_000_000,
            swap_used: 1_000_000_000,
        },
        load: LoadSnapshot {
            one: 2.5,
            five: 1.8,
            fifteen: 1.2,
            uptime_secs: 86400,
        },
        processes: make_processes(n),
        networks: muxtop_core::network::NetworkSnapshot {
            interfaces: vec![],
            total_rx: 0,
            total_tx: 0,
        },
        containers: None,
        timestamp_ms: 0,
    }
}

fn bench_recompute_visible(c: &mut Criterion) {
    let mut group = c.benchmark_group("recompute_visible");
    for size in [100, 500, 1000, 5000] {
        let snap = make_snapshot(size);

        // Flat mode (default)
        group.bench_with_input(BenchmarkId::new("flat", size), &snap, |b, snap| {
            let mut app = AppState::new();
            app.apply_snapshot(snap.clone());
            b.iter(|| app.recompute_visible());
        });

        // Tree mode
        group.bench_with_input(BenchmarkId::new("tree", size), &snap, |b, snap| {
            let mut app = AppState::new();
            app.tree_mode = true;
            app.apply_snapshot(snap.clone());
            b.iter(|| app.recompute_visible());
        });

        // With filter
        group.bench_with_input(BenchmarkId::new("filtered", size), &snap, |b, snap| {
            let mut app = AppState::new();
            app.filter_input = "proc-1".to_string();
            app.apply_snapshot(snap.clone());
            b.iter(|| app.recompute_visible());
        });
    }
    group.finish();
}

fn bench_palette_refilter(c: &mut Criterion) {
    let mut group = c.benchmark_group("palette_refilter");

    group.bench_function("empty_input", |b| {
        let mut ps = PaletteState::new();
        b.iter(|| ps.refilter());
    });

    group.bench_function("short_query", |b| {
        let mut ps = PaletteState::new();
        ps.input = "sort".to_string();
        b.iter(|| ps.refilter());
    });

    group.bench_function("long_query", |b| {
        let mut ps = PaletteState::new();
        ps.input = "toggle tree view".to_string();
        b.iter(|| ps.refilter());
    });

    group.bench_function("no_match", |b| {
        let mut ps = PaletteState::new();
        ps.input = "zzznomatch".to_string();
        b.iter(|| ps.refilter());
    });

    group.finish();
}

criterion_group!(benches, bench_recompute_visible, bench_palette_refilter);
criterion_main!(benches);
