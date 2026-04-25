use bincode::config;
use criterion::{Criterion, criterion_group, criterion_main};

use muxtop_core::containers::{ContainerSnapshot, ContainerState, ContainersSnapshot, EngineKind};
use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
use muxtop_core::process::ProcessInfo;
use muxtop_core::system::{
    CoreSnapshot, CpuSnapshot, LoadSnapshot, MemorySnapshot, SystemSnapshot,
};
use muxtop_proto::frame::decode_frame;
use muxtop_proto::{WireMessage, encode_frame};

fn make_snapshot_3000() -> SystemSnapshot {
    let cores: Vec<CoreSnapshot> = (0..8)
        .map(|i| CoreSnapshot {
            name: format!("cpu{i}"),
            usage: (i as f32 * 12.5) % 100.0,
            frequency: 3600,
        })
        .collect();

    let processes: Vec<ProcessInfo> = (0..3000)
        .map(|i| ProcessInfo {
            pid: i as u32 + 1,
            parent_pid: if i == 0 {
                None
            } else {
                Some((i / 3) as u32 + 1)
            },
            name: format!("proc-{i}"),
            command: format!("/usr/bin/proc-{i} --flag --option=value"),
            user: format!("user{}", i % 10),
            cpu_percent: (i as f32 * 7.3) % 100.0,
            memory_bytes: (i as u64 * 1_048_576) % 16_000_000_000,
            memory_percent: ((i as f64 * 0.03) % 100.0) as f32,
            status: if i % 50 == 0 { "Running" } else { "Sleeping" }.to_string(),
        })
        .collect();

    let interfaces: Vec<NetworkInterfaceSnapshot> = (0..4)
        .map(|i| NetworkInterfaceSnapshot {
            name: format!("eth{i}"),
            bytes_rx: (i as u64 + 1) * 1_000_000,
            bytes_tx: (i as u64 + 1) * 500_000,
            packets_rx: (i as u64 + 1) * 1000,
            packets_tx: (i as u64 + 1) * 500,
            errors_rx: 0,
            errors_tx: 0,
            mac_address: format!("00:11:22:33:44:{i:02x}"),
            is_up: true,
        })
        .collect();

    let total_rx: u64 = interfaces.iter().map(|i| i.bytes_rx).sum();
    let total_tx: u64 = interfaces.iter().map(|i| i.bytes_tx).sum();

    SystemSnapshot {
        cpu: CpuSnapshot {
            global_usage: 45.2,
            cores,
        },
        memory: MemorySnapshot {
            total: 32_000_000_000,
            used: 16_000_000_000,
            available: 16_000_000_000,
            swap_total: 8_000_000_000,
            swap_used: 2_000_000_000,
        },
        load: LoadSnapshot {
            one: 2.5,
            five: 1.8,
            fifteen: 1.2,
            uptime_secs: 86400,
        },
        processes,
        networks: NetworkSnapshot {
            interfaces,
            total_rx,
            total_tx,
        },
        containers: None,
        timestamp_ms: 1_713_200_000_000,
    }
}

fn bench_snapshot_serialize(c: &mut Criterion) {
    let snap = make_snapshot_3000();
    let msg = WireMessage::Snapshot(snap);

    c.bench_function("snapshot_serialize_3000", |b| {
        b.iter(|| msg.to_frame().unwrap());
    });
}

fn bench_snapshot_deserialize(c: &mut Criterion) {
    let snap = make_snapshot_3000();
    let msg = WireMessage::Snapshot(snap);
    let frame = msg.to_frame().unwrap();

    c.bench_function("snapshot_deserialize_3000", |b| {
        b.iter(|| WireMessage::from_frame(&frame).unwrap());
    });
}

fn bench_frame_roundtrip(c: &mut Criterion) {
    let snap = make_snapshot_3000();
    let msg = WireMessage::Snapshot(snap);
    let frame = msg.to_frame().unwrap();
    let encoded = encode_frame(&frame).unwrap();

    c.bench_function("frame_roundtrip_3000", |b| {
        b.iter(|| {
            let (decoded_frame, _) = decode_frame(&encoded).unwrap();
            WireMessage::from_frame(&decoded_frame).unwrap()
        });
    });
}

// ─── v0.3 Containers wire-protocol benches (E3) ───────────────────────────

fn make_containers_snapshot(n: usize) -> ContainersSnapshot {
    let states = [
        ContainerState::Running,
        ContainerState::Exited,
        ContainerState::Paused,
        ContainerState::Restarting,
    ];
    let containers: Vec<ContainerSnapshot> = (0..n)
        .map(|i| ContainerSnapshot {
            id: format!("container{i:012}"),
            id_full: format!("container{i:012}{:0>43}", ""),
            name: format!("svc-{i:04}"),
            image: "registry.example.com/library/app:v1.2.3".into(),
            state: states[i % states.len()],
            status_text: format!("Up {} hours", i % 24),
            cpu_pct: (i as f32 * 1.7) % 100.0,
            mem_used_bytes: (i as u64 * 4 * 1024 * 1024) % (2 * 1024 * 1024 * 1024),
            mem_limit_bytes: 2 * 1024 * 1024 * 1024,
            net_rx_bytes: i as u64 * 1_048_576,
            net_tx_bytes: i as u64 * 524_288,
            block_read_bytes: i as u64 * 4_096,
            block_write_bytes: i as u64 * 2_048,
            started_at_ms: 1_700_000_000_000 + i as u64 * 3_600_000,
        })
        .collect();

    ContainersSnapshot {
        engine: EngineKind::Docker,
        daemon_up: true,
        containers,
    }
}

fn bench_containers_serialize(c: &mut Criterion) {
    let snapshot = make_containers_snapshot(100);
    let cfg = config::standard();

    c.bench_function("containers_serialize_100", |b| {
        b.iter(|| bincode::encode_to_vec(&snapshot, cfg).unwrap());
    });
}

fn bench_containers_deserialize(c: &mut Criterion) {
    let snapshot = make_containers_snapshot(100);
    let cfg = config::standard();
    let bytes = bincode::encode_to_vec(&snapshot, cfg).unwrap();

    c.bench_function("containers_deserialize_100", |b| {
        b.iter(|| {
            let (decoded, _): (ContainersSnapshot, usize) =
                bincode::decode_from_slice(&bytes, cfg).unwrap();
            decoded
        });
    });
}

criterion_group!(
    benches,
    bench_snapshot_serialize,
    bench_snapshot_deserialize,
    bench_frame_roundtrip,
    bench_containers_serialize,
    bench_containers_deserialize
);
criterion_main!(benches);
