use criterion::{Criterion, criterion_group, criterion_main};

use muxtop_core::system::SystemSnapshot;

fn bench_snapshot_collect(c: &mut Criterion) {
    // Pre-initialize and refresh once so delta-based metrics are primed.
    let mut sys = sysinfo::System::new_all();
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_all();

    c.bench_function("SystemSnapshot::collect", |b| {
        b.iter(|| {
            sys.refresh_all();
            SystemSnapshot::collect(&sys)
        });
    });
}

criterion_group!(benches, bench_snapshot_collect);
criterion_main!(benches);
