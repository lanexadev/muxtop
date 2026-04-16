use criterion::{Criterion, criterion_group, criterion_main};

use muxtop_core::network::{NetworkHistory, NetworkInterfaceSnapshot, NetworkSnapshot};

fn make_snapshot(rx: u64, tx: u64) -> NetworkSnapshot {
    NetworkSnapshot {
        interfaces: vec![
            NetworkInterfaceSnapshot {
                name: "eth0".into(),
                bytes_rx: rx,
                bytes_tx: tx,
                packets_rx: rx / 100,
                packets_tx: tx / 100,
                errors_rx: 0,
                errors_tx: 0,
                mac_address: "00:11:22:33:44:55".into(),
                is_up: true,
            },
            NetworkInterfaceSnapshot {
                name: "wlan0".into(),
                bytes_rx: rx / 2,
                bytes_tx: tx / 2,
                packets_rx: rx / 200,
                packets_tx: tx / 200,
                errors_rx: 0,
                errors_tx: 0,
                mac_address: "aa:bb:cc:dd:ee:ff".into(),
                is_up: true,
            },
        ],
        total_rx: rx + rx / 2,
        total_tx: tx + tx / 2,
    }
}

fn bench_network_snapshot_collect(c: &mut Criterion) {
    let mut networks = sysinfo::Networks::new_with_refreshed_list();
    std::thread::sleep(std::time::Duration::from_millis(200));
    networks.refresh(true);

    c.bench_function("NetworkSnapshot::collect", |b| {
        b.iter(|| {
            networks.refresh(true);
            NetworkSnapshot::collect(&networks)
        });
    });
}

fn bench_network_history_push_60(c: &mut Criterion) {
    c.bench_function("NetworkHistory::push_60", |b| {
        b.iter(|| {
            let mut history = NetworkHistory::new(60);
            for i in 0..60u64 {
                history.push(make_snapshot(i * 1_000_000, i * 500_000));
            }
            history
        });
    });
}

fn bench_network_bandwidth_calc(c: &mut Criterion) {
    let mut history = NetworkHistory::new(60);
    for i in 0..60u64 {
        history.push(make_snapshot(i * 1_000_000, i * 500_000));
    }

    c.bench_function("NetworkHistory::bandwidth_calc", |b| {
        b.iter(|| {
            let _ = history.bandwidth_rx("eth0");
            let _ = history.bandwidth_tx("eth0");
            let _ = history.bandwidth_rx("wlan0");
            let _ = history.bandwidth_tx("wlan0");
            let _ = history.sparkline_rx("eth0", 60);
            let _ = history.sparkline_tx("eth0", 60);
        });
    });
}

criterion_group!(
    benches,
    bench_network_snapshot_collect,
    bench_network_history_push_60,
    bench_network_bandwidth_calc
);
criterion_main!(benches);
