//! Heap allocation profiler for muxtop-tui hot paths.
//!
//! Runs each hot path a fixed number of iterations under the `dhat` global
//! allocator and reports total allocations, peak heap, and bytes allocated.
//! Pair with `dh_view` (<https://valgrind.org/docs/manual/dh-manual.html>) on
//! the generated `dhat-heap.json` for a drill-down view.
//!
//! Run with: `cargo run --example alloc_profile --release -p muxtop-tui`

use muxtop_core::process::{ProcessInfo, SortField, SortOrder, sort_processes};
use muxtop_core::system::{
    CoreSnapshot, CpuSnapshot, LoadSnapshot, MemorySnapshot, SystemSnapshot,
};
use muxtop_tui::AppState;
use muxtop_tui::app::PaletteState;

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

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

fn report(label: &str, iters: u64, stats_before: dhat::HeapStats, stats_after: dhat::HeapStats) {
    let total_blocks = stats_after.total_blocks - stats_before.total_blocks;
    let total_bytes = stats_after.total_bytes - stats_before.total_bytes;
    let per_iter_blocks = total_blocks / iters;
    let per_iter_bytes = total_bytes / iters;
    println!(
        "{label:>36}  {total_blocks:>10} allocs  {total_bytes:>12} B  \
         │  per iter: {per_iter_blocks:>5} allocs  {per_iter_bytes:>7} B"
    );
}

fn section(title: &str) {
    println!(
        "\n── {title} {:─<width$}",
        "",
        width = 80usize.saturating_sub(title.len() + 4)
    );
}

fn profile_palette_refilter(iters: u64) {
    section("palette_refilter");
    let mut ps = PaletteState::new();

    // empty input
    ps.input.clear();
    let before = dhat::HeapStats::get();
    for _ in 0..iters {
        ps.refilter();
    }
    let after = dhat::HeapStats::get();
    report("empty_input", iters, before, after);

    // short query
    ps.input = "sort".to_string();
    let before = dhat::HeapStats::get();
    for _ in 0..iters {
        ps.refilter();
    }
    let after = dhat::HeapStats::get();
    report("short_query (\"sort\")", iters, before, after);

    // no match
    ps.input = "zzznomatch".to_string();
    let before = dhat::HeapStats::get();
    for _ in 0..iters {
        ps.refilter();
    }
    let after = dhat::HeapStats::get();
    report("no_match", iters, before, after);
}

fn profile_sort_processes(iters: u64) {
    section("sort_processes (5000 procs)");
    let procs = make_processes(5000);

    let before = dhat::HeapStats::get();
    for _ in 0..iters {
        let mut p = procs.clone();
        sort_processes(&mut p, SortField::Cpu, SortOrder::Desc);
    }
    let after = dhat::HeapStats::get();
    report("cpu_desc (incl. clone)", iters, before, after);

    let before = dhat::HeapStats::get();
    for _ in 0..iters {
        let mut p = procs.clone();
        sort_processes(&mut p, SortField::Name, SortOrder::Asc);
    }
    let after = dhat::HeapStats::get();
    report("name_asc (incl. clone)", iters, before, after);
}

fn profile_apply_snapshot(iters: u64) {
    section("apply_snapshot + recompute_visible (1000 procs)");
    let snap = make_snapshot(1000);

    // Flat mode
    let mut app = AppState::new();
    let before = dhat::HeapStats::get();
    for _ in 0..iters {
        app.apply_snapshot(snap.clone());
        app.recompute_visible();
    }
    let after = dhat::HeapStats::get();
    report("flat (incl. snap clone)", iters, before, after);

    // Tree mode
    let mut app = AppState::new();
    app.tree_mode = true;
    let before = dhat::HeapStats::get();
    for _ in 0..iters {
        app.apply_snapshot(snap.clone());
        app.recompute_visible();
    }
    let after = dhat::HeapStats::get();
    report("tree (incl. snap clone)", iters, before, after);
}

fn main() {
    let _profiler = dhat::Profiler::new_heap();

    println!("muxtop-tui allocation profile (dhat)");
    println!("Writing detailed trace to dhat-heap.json on exit.");

    // dhat intercepts every allocation, so iteration counts stay modest:
    // we want accurate per-iter numbers, not a speed benchmark.
    profile_palette_refilter(10_000);
    profile_sort_processes(20);
    profile_apply_snapshot(20);

    println!("\nTotals across all profiled sections (includes setup + tear-down allocations):");
    let final_stats = dhat::HeapStats::get();
    println!(
        "  max heap bytes : {}\n  max blocks     : {}\n  total blocks   : {}\n  total bytes    : {}",
        final_stats.max_bytes,
        final_stats.max_blocks,
        final_stats.total_blocks,
        final_stats.total_bytes,
    );
}
