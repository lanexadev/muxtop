use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use muxtop_core::process::{
    ProcessInfo, SortField, SortOrder, build_process_tree, filter_processes, flatten_tree,
    sort_processes,
};

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

fn bench_sort(c: &mut Criterion) {
    let mut group = c.benchmark_group("sort_processes");
    for size in [100, 500, 1000, 5000] {
        let procs = make_processes(size);
        group.bench_with_input(BenchmarkId::new("cpu_desc", size), &procs, |b, procs| {
            b.iter_batched(
                || procs.clone(),
                |mut p| sort_processes(&mut p, SortField::Cpu, SortOrder::Desc),
                criterion::BatchSize::SmallInput,
            );
        });
        group.bench_with_input(BenchmarkId::new("name_asc", size), &procs, |b, procs| {
            b.iter_batched(
                || procs.clone(),
                |mut p| sort_processes(&mut p, SortField::Name, SortOrder::Asc),
                criterion::BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_filter(c: &mut Criterion) {
    let mut group = c.benchmark_group("filter_processes");
    for size in [100, 500, 1000, 5000] {
        let procs = make_processes(size);
        group.bench_with_input(BenchmarkId::new("match_some", size), &procs, |b, procs| {
            b.iter(|| filter_processes(procs, "proc-1"));
        });
        group.bench_with_input(BenchmarkId::new("match_none", size), &procs, |b, procs| {
            b.iter(|| filter_processes(procs, "zzznomatch"));
        });
        group.bench_with_input(
            BenchmarkId::new("empty_pattern", size),
            &procs,
            |b, procs| {
                b.iter(|| filter_processes(procs, ""));
            },
        );
    }
    group.finish();
}

fn bench_tree(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_process_tree");
    for size in [100, 500, 1000, 5000] {
        let procs = make_processes(size);
        group.bench_with_input(BenchmarkId::new("build", size), &procs, |b, procs| {
            b.iter(|| build_process_tree(procs));
        });
    }
    group.finish();
}

fn bench_flatten(c: &mut Criterion) {
    let mut group = c.benchmark_group("flatten_tree");
    for size in [100, 500, 1000] {
        let procs = make_processes(size);
        let tree = build_process_tree(&procs);
        group.bench_with_input(BenchmarkId::new("flatten", size), &tree, |b, tree| {
            b.iter(|| flatten_tree(tree));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_sort, bench_filter, bench_tree, bench_flatten);
criterion_main!(benches);
