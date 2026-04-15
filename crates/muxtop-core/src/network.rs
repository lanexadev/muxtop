use std::collections::VecDeque;
use std::time::Instant;

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Per-interface network snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct NetworkInterfaceSnapshot {
    pub name: String,
    pub bytes_rx: u64,
    pub bytes_tx: u64,
    pub packets_rx: u64,
    pub packets_tx: u64,
    pub errors_rx: u64,
    pub errors_tx: u64,
    pub mac_address: String,
    /// Whether this interface has seen any traffic (cumulative rx or tx > 0).
    /// Note: sysinfo 0.34 does not expose OS-level link state, so this is a
    /// traffic-based heuristic — not a true up/down indicator.
    pub is_up: bool,
}

/// Aggregated network snapshot across all interfaces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct NetworkSnapshot {
    pub interfaces: Vec<NetworkInterfaceSnapshot>,
    pub total_rx: u64,
    pub total_tx: u64,
}

impl NetworkSnapshot {
    /// Collect network snapshot from sysinfo Networks.
    pub fn collect(networks: &sysinfo::Networks) -> Self {
        let mut total_rx: u64 = 0;
        let mut total_tx: u64 = 0;

        let interfaces: Vec<NetworkInterfaceSnapshot> = networks
            .iter()
            .map(|(name, data)| {
                let bytes_rx = data.total_received();
                let bytes_tx = data.total_transmitted();
                total_rx = total_rx.saturating_add(bytes_rx);
                total_tx = total_tx.saturating_add(bytes_tx);

                NetworkInterfaceSnapshot {
                    name: name.clone(),
                    bytes_rx,
                    bytes_tx,
                    packets_rx: data.total_packets_received(),
                    packets_tx: data.total_packets_transmitted(),
                    errors_rx: data.total_errors_on_received(),
                    errors_tx: data.total_errors_on_transmitted(),
                    mac_address: data.mac_address().to_string(),
                    is_up: bytes_rx > 0 || bytes_tx > 0,
                }
            })
            .collect();

        Self {
            interfaces,
            total_rx,
            total_tx,
        }
    }
}

/// Timestamped network snapshot for history tracking.
#[derive(Debug, Clone)]
struct TimestampedSnapshot {
    snapshot: NetworkSnapshot,
    timestamp: Instant,
}

/// Circular buffer storing network snapshots for bandwidth and sparkline calculations.
///
/// Bandwidth is computed as bytes/s using timestamps from consecutive snapshots.
/// Sparkline values are byte deltas between consecutive samples (not normalized
/// to time — suitable for fixed-interval display).
#[derive(Debug, Clone)]
pub struct NetworkHistory {
    samples: VecDeque<TimestampedSnapshot>,
    capacity: usize,
}

impl NetworkHistory {
    /// Create a new history buffer with the given capacity.
    /// Capacity is clamped to a minimum of 2 (needed for delta computation).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(2);
        Self {
            samples: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Push a new snapshot, evicting the oldest if at capacity.
    pub fn push(&mut self, snapshot: NetworkSnapshot) {
        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(TimestampedSnapshot {
            snapshot,
            timestamp: Instant::now(),
        });
    }

    /// Number of samples currently stored.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Compute RX bandwidth in bytes/s for a given interface over the last interval.
    /// Returns 0.0 if fewer than 2 samples or interface not found.
    pub fn bandwidth_rx(&self, iface: &str) -> f64 {
        self.bandwidth(iface, |i| i.bytes_rx)
    }

    /// Compute TX bandwidth in bytes/s for a given interface over the last interval.
    /// Returns 0.0 if fewer than 2 samples or interface not found.
    pub fn bandwidth_tx(&self, iface: &str) -> f64 {
        self.bandwidth(iface, |i| i.bytes_tx)
    }

    /// Return the last N RX bandwidth values for sparkline rendering.
    /// Each value is the byte delta between consecutive samples.
    pub fn sparkline_rx(&self, iface: &str, points: usize) -> Vec<u64> {
        self.sparkline(iface, points, |i| i.bytes_rx)
    }

    /// Return the last N TX bandwidth values for sparkline rendering.
    pub fn sparkline_tx(&self, iface: &str, points: usize) -> Vec<u64> {
        self.sparkline(iface, points, |i| i.bytes_tx)
    }

    fn find_iface_value(
        snapshot: &NetworkSnapshot,
        iface: &str,
        extract: &impl Fn(&NetworkInterfaceSnapshot) -> u64,
    ) -> Option<u64> {
        snapshot
            .interfaces
            .iter()
            .find(|i| i.name == iface)
            .map(extract)
    }

    fn bandwidth(&self, iface: &str, extract: impl Fn(&NetworkInterfaceSnapshot) -> u64) -> f64 {
        if self.samples.len() < 2 {
            return 0.0;
        }
        let prev = &self.samples[self.samples.len() - 2];
        let curr = &self.samples[self.samples.len() - 1];

        let prev_val = Self::find_iface_value(&prev.snapshot, iface, &extract).unwrap_or(0);
        let curr_val = Self::find_iface_value(&curr.snapshot, iface, &extract).unwrap_or(0);

        // Handle counter reset (interface bounce): treat negative delta as 0.
        let delta = curr_val.saturating_sub(prev_val) as f64;
        let elapsed = curr.timestamp.duration_since(prev.timestamp).as_secs_f64();

        if elapsed > 0.0 { delta / elapsed } else { 0.0 }
    }

    fn sparkline(
        &self,
        iface: &str,
        points: usize,
        extract: impl Fn(&NetworkInterfaceSnapshot) -> u64,
    ) -> Vec<u64> {
        if self.samples.len() < 2 || points == 0 {
            return Vec::new();
        }

        let n = points.min(self.samples.len() - 1);
        let start = self.samples.len() - n - 1;
        let mut result = Vec::with_capacity(n);

        for i in start..self.samples.len() - 1 {
            let prev_val =
                Self::find_iface_value(&self.samples[i].snapshot, iface, &extract).unwrap_or(0);
            let curr_val =
                Self::find_iface_value(&self.samples[i + 1].snapshot, iface, &extract).unwrap_or(0);
            result.push(curr_val.saturating_sub(prev_val));
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_types_send_clone() {
        fn assert_send_clone<T: Send + Clone>() {}
        assert_send_clone::<NetworkInterfaceSnapshot>();
        assert_send_clone::<NetworkSnapshot>();
        assert_send_clone::<NetworkHistory>();
    }

    #[test]
    fn test_interface_snapshot_from_sysinfo() {
        let networks = sysinfo::Networks::new_with_refreshed_list();
        let snapshot = NetworkSnapshot::collect(&networks);
        // On any real system there should be at least one interface (lo/lo0).
        assert!(
            !snapshot.interfaces.is_empty(),
            "should have at least one network interface"
        );
        for iface in &snapshot.interfaces {
            assert!(!iface.name.is_empty(), "interface name should not be empty");
        }
    }

    #[test]
    fn test_network_snapshot_totals_consistent() {
        let networks = sysinfo::Networks::new_with_refreshed_list();
        let snapshot = NetworkSnapshot::collect(&networks);

        let sum_rx: u64 = snapshot.interfaces.iter().map(|i| i.bytes_rx).sum();
        let sum_tx: u64 = snapshot.interfaces.iter().map(|i| i.bytes_tx).sum();
        assert_eq!(
            snapshot.total_rx, sum_rx,
            "total_rx should equal sum of interface bytes_rx"
        );
        assert_eq!(
            snapshot.total_tx, sum_tx,
            "total_tx should equal sum of interface bytes_tx"
        );
    }

    /// Helper to create a synthetic NetworkSnapshot with one interface.
    fn make_snapshot(iface: &str, rx: u64, tx: u64) -> NetworkSnapshot {
        NetworkSnapshot {
            interfaces: vec![NetworkInterfaceSnapshot {
                name: iface.into(),
                bytes_rx: rx,
                bytes_tx: tx,
                packets_rx: 0,
                packets_tx: 0,
                errors_rx: 0,
                errors_tx: 0,
                mac_address: "00:00:00:00:00:00".into(),
                is_up: rx > 0 || tx > 0,
            }],
            total_rx: rx,
            total_tx: tx,
        }
    }

    #[test]
    fn test_history_empty() {
        let history = NetworkHistory::new(60);
        assert!(history.is_empty());
        assert_eq!(history.len(), 0);
        assert_eq!(history.bandwidth_rx("eth0"), 0.0);
        assert_eq!(history.bandwidth_tx("eth0"), 0.0);
        assert!(history.sparkline_rx("eth0", 30).is_empty());
        assert!(history.sparkline_tx("eth0", 30).is_empty());
    }

    #[test]
    fn test_history_single_snapshot() {
        let mut history = NetworkHistory::new(60);
        history.push(make_snapshot("eth0", 1000, 500));
        assert_eq!(history.len(), 1);
        assert_eq!(history.bandwidth_rx("eth0"), 0.0);
        assert!(history.sparkline_rx("eth0", 30).is_empty());
    }

    #[test]
    fn test_bandwidth_calculation() {
        let mut history = NetworkHistory::new(60);
        history.push(make_snapshot("eth0", 1000, 500));
        // Sleep briefly so elapsed > 0 for bandwidth division.
        std::thread::sleep(std::time::Duration::from_millis(10));
        history.push(make_snapshot("eth0", 2000, 800));

        let bw_rx = history.bandwidth_rx("eth0");
        let bw_tx = history.bandwidth_tx("eth0");
        // With ~10ms elapsed: 1000 bytes / 0.01s ≈ 100_000 bytes/s
        // We just verify it's positive and in a plausible range.
        assert!(bw_rx > 0.0, "bandwidth_rx should be positive, got {bw_rx}");
        assert!(bw_tx > 0.0, "bandwidth_tx should be positive, got {bw_tx}");
    }

    #[test]
    fn test_bandwidth_counter_reset() {
        let mut history = NetworkHistory::new(60);
        history.push(make_snapshot("eth0", 5000, 3000));
        std::thread::sleep(std::time::Duration::from_millis(10));
        // Counter reset: new value < old value
        history.push(make_snapshot("eth0", 100, 50));

        // saturating_sub handles this: 100 - 5000 = 0
        assert_eq!(history.bandwidth_rx("eth0"), 0.0);
        assert_eq!(history.bandwidth_tx("eth0"), 0.0);
    }

    #[test]
    fn test_bandwidth_unknown_interface() {
        let mut history = NetworkHistory::new(60);
        history.push(make_snapshot("eth0", 1000, 500));
        std::thread::sleep(std::time::Duration::from_millis(10));
        history.push(make_snapshot("eth0", 2000, 800));

        assert_eq!(history.bandwidth_rx("nonexistent"), 0.0);
    }

    #[test]
    fn test_history_capacity_eviction() {
        let mut history = NetworkHistory::new(60);
        for i in 0..70 {
            history.push(make_snapshot("eth0", i * 100, i * 50));
        }
        assert_eq!(history.len(), 60);
    }

    #[test]
    fn test_history_capacity_minimum() {
        // Capacity 0 should be clamped to 2.
        let history = NetworkHistory::new(0);
        assert_eq!(history.capacity, 2);

        let history = NetworkHistory::new(1);
        assert_eq!(history.capacity, 2);
    }

    #[test]
    fn test_sparkline_data() {
        let mut history = NetworkHistory::new(60);
        // Push 5 snapshots: 0, 100, 300, 600, 1000
        for &rx in &[0u64, 100, 300, 600, 1000] {
            history.push(make_snapshot("eth0", rx, 0));
        }

        let spark = history.sparkline_rx("eth0", 10);
        // 4 deltas from 5 samples: 100, 200, 300, 400
        assert_eq!(spark, vec![100, 200, 300, 400]);
    }

    #[test]
    fn test_sparkline_limited_points() {
        let mut history = NetworkHistory::new(60);
        for &rx in &[0u64, 100, 300, 600, 1000] {
            history.push(make_snapshot("eth0", rx, 0));
        }

        let spark = history.sparkline_rx("eth0", 2);
        // Last 2 deltas: 300, 400
        assert_eq!(spark, vec![300, 400]);
    }

    #[test]
    fn test_sparkline_zero_points() {
        let mut history = NetworkHistory::new(60);
        history.push(make_snapshot("eth0", 0, 0));
        history.push(make_snapshot("eth0", 100, 50));

        let spark = history.sparkline_rx("eth0", 0);
        assert!(spark.is_empty());
    }

    #[test]
    fn test_sparkline_unknown_interface() {
        let mut history = NetworkHistory::new(60);
        history.push(make_snapshot("eth0", 0, 0));
        history.push(make_snapshot("eth0", 100, 50));

        let spark = history.sparkline_rx("nonexistent", 10);
        // Unknown interface values are 0, so deltas are 0
        assert_eq!(spark, vec![0]);
    }

    #[test]
    fn test_all_structs_are_debug() {
        let iface = NetworkInterfaceSnapshot {
            name: "eth0".into(),
            bytes_rx: 1000,
            bytes_tx: 500,
            packets_rx: 10,
            packets_tx: 5,
            errors_rx: 0,
            errors_tx: 0,
            mac_address: "00:00:00:00:00:00".into(),
            is_up: true,
        };
        assert!(!format!("{iface:?}").is_empty());

        let snap = NetworkSnapshot {
            interfaces: vec![iface],
            total_rx: 1000,
            total_tx: 500,
        };
        assert!(!format!("{snap:?}").is_empty());
    }
}
