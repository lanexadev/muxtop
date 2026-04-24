//! Container data model for the Containers tab (v0.3.0).
//!
//! Mirrors the structure of `network.rs`: plain-data `*Snapshot` structs
//! with the full derive set needed to cross the wire protocol in E3.
//! The collection logic lives in `container_engine.rs`; this module is
//! data-only.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Lifecycle state of a container, mirroring the Docker Engine API states.
///
/// Source: `/containers/json` `State` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ContainerState {
    Created,
    Running,
    Paused,
    Restarting,
    Exited,
    Dead,
    Removing,
}

/// Which runtime exposed the container snapshot.
///
/// `Unknown` covers the case where the socket answered but `GET /info` did not
/// return a recognizable `OperatingSystem` / `ServerVersion`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum EngineKind {
    Docker,
    Podman,
    Unknown,
}

/// Per-container snapshot. All byte counters are cumulative from container
/// start, identical to what Docker's stats endpoint reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct ContainerSnapshot {
    /// Short container id (first 12 hex chars).
    pub id: String,
    /// Primary container name (without leading `/`).
    pub name: String,
    /// Image reference (e.g. `nginx:1.27`).
    pub image: String,
    pub state: ContainerState,
    /// Human-readable status (e.g. `Up 2 hours`).
    pub status_text: String,
    /// Instantaneous CPU usage, in percent of one host CPU.
    pub cpu_pct: f32,
    pub mem_used_bytes: u64,
    /// `0` when no cgroup memory limit is set.
    pub mem_limit_bytes: u64,
    pub net_rx_bytes: u64,
    pub net_tx_bytes: u64,
    pub block_read_bytes: u64,
    pub block_write_bytes: u64,
    /// Milliseconds since Unix epoch.
    pub started_at_ms: u64,
}

/// Aggregated container snapshot for a single host.
///
/// `daemon_up = false` with an empty `containers` vec is the canonical "no
/// daemon detected" state; the TUI uses it to render the placeholder message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct ContainersSnapshot {
    pub engine: EngineKind,
    pub daemon_up: bool,
    pub containers: Vec<ContainerSnapshot>,
}

impl ContainersSnapshot {
    /// Canonical empty snapshot when no container engine is reachable.
    pub fn unavailable() -> Self {
        Self {
            engine: EngineKind::Unknown,
            daemon_up: false,
            containers: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::config;

    fn sample_snapshot() -> ContainerSnapshot {
        ContainerSnapshot {
            id: "abc123def456".into(),
            name: "nginx".into(),
            image: "nginx:1.27".into(),
            state: ContainerState::Running,
            status_text: "Up 2 hours".into(),
            cpu_pct: 3.5,
            mem_used_bytes: 128 * 1024 * 1024,
            mem_limit_bytes: 512 * 1024 * 1024,
            net_rx_bytes: 1_000_000,
            net_tx_bytes: 500_000,
            block_read_bytes: 4 * 1024 * 1024,
            block_write_bytes: 2 * 1024 * 1024,
            started_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn container_snapshot_derive_round_trip() {
        let original = sample_snapshot();
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (ContainerSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn engine_kind_derive_round_trip() {
        let cfg = config::standard();
        for kind in [EngineKind::Docker, EngineKind::Podman, EngineKind::Unknown] {
            let bytes = bincode::encode_to_vec(kind, cfg).expect("encode");
            let (decoded, _): (EngineKind, usize) =
                bincode::decode_from_slice(&bytes, cfg).expect("decode");
            assert_eq!(kind, decoded);
        }
    }

    #[test]
    fn container_state_is_exhaustive() {
        // Exhaustive match without wildcard — if a new variant is added,
        // the compiler flags this test and any downstream UI code.
        for state in [
            ContainerState::Created,
            ContainerState::Running,
            ContainerState::Paused,
            ContainerState::Restarting,
            ContainerState::Exited,
            ContainerState::Dead,
            ContainerState::Removing,
        ] {
            let _label: &'static str = match state {
                ContainerState::Created => "created",
                ContainerState::Running => "running",
                ContainerState::Paused => "paused",
                ContainerState::Restarting => "restarting",
                ContainerState::Exited => "exited",
                ContainerState::Dead => "dead",
                ContainerState::Removing => "removing",
            };
        }
    }

    #[test]
    fn containers_snapshot_unavailable_is_empty() {
        let s = ContainersSnapshot::unavailable();
        assert!(!s.daemon_up);
        assert!(s.containers.is_empty());
        assert_eq!(s.engine, EngineKind::Unknown);
    }
}
