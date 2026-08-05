//! GPU data model for the GPU tab (v0.5.0).
//!
//! Mirrors the structure of `kube.rs` and `containers.rs`: plain-data
//! `*Snapshot` structs that the collector publishes through `SystemSnapshot`.
//! The collection logic lives in `gpu_engine.rs` (trait + composite),
//! `nvml_engine.rs` (NVIDIA) and `amd_engine.rs` (AMD); this module is
//! data-only.
//!
//! ## Everything measurable is `Option`
//!
//! Unlike CPU or memory, no GPU metric is universally available. NVML reports
//! encoder/decoder utilisation that AMD's sysfs interface has no equivalent
//! for; AMD reports fan RPM on cards that expose a hwmon node and nothing on
//! the ones that don't; a laptop dGPU in runtime-D3 answers nothing at all
//! until it wakes. Every metric is therefore `Option<T>`, and the UI renders
//! `None` as `—`. This is the same graceful-degradation contract v0.4 used
//! for metrics-server, applied per field instead of per cluster.
//!
//! **Do not paper over a missing metric with `0`.** A GPU at 0 % and a GPU
//! that cannot report utilisation are different facts, and conflating them
//! makes the tab lie.
//!
//! ## Field ordering is contractual
//!
//! Field order is part of the wire protocol (`Encode`/`Decode` derives).
//! Adding fields anywhere but the end is a wire-format break — see the
//! `kube.rs` module doc for the history behind that rule.

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Which vendor's hardware a device belongs to.
///
/// `Unknown` covers a device enumerated by a backend that could not classify
/// it — it is rendered verbatim rather than hidden, because a GPU muxtop can
/// see but not name is still worth showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Apple,
    Unknown,
}

impl GpuVendor {
    /// Short lowercase label for the summary bar and the VENDOR column.
    pub fn label(self) -> &'static str {
        match self {
            GpuVendor::Nvidia => "nvidia",
            GpuVendor::Amd => "amd",
            GpuVendor::Intel => "intel",
            GpuVendor::Apple => "apple",
            GpuVendor::Unknown => "unknown",
        }
    }
}

/// Which collection mechanism produced a device.
///
/// Reported per snapshot so the UI can explain *why* a metric is missing:
/// "AMD sysfs cannot report per-process usage" is actionable, "—" alone is
/// not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum GpuBackend {
    /// NVIDIA Management Library, loaded dynamically at runtime.
    Nvml,
    /// AMD `amdgpu` driver via `/sys/class/drm/card*/device` (Linux only).
    AmdSysfs,
    /// Apple Silicon integrated GPU. **Reserved — not implemented in v0.5.**
    ///
    /// The variant exists so the wire format does not break when the macOS
    /// backend lands in v0.6; no engine produces it today.
    AppleIoReport,
}

impl GpuBackend {
    pub fn label(self) -> &'static str {
        match self {
            GpuBackend::Nvml => "nvml",
            GpuBackend::AmdSysfs => "amdgpu-sysfs",
            GpuBackend::AppleIoReport => "ioreport",
        }
    }
}

/// How a process is using the GPU.
///
/// NVML reports compute (CUDA) and graphics (OpenGL/Vulkan/display) contexts
/// through two separate calls; a process holding both is merged into
/// [`Self::Both`] rather than listed twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum GpuProcessKind {
    Compute,
    Graphics,
    Both,
    Unknown,
}

impl GpuProcessKind {
    pub fn label(self) -> &'static str {
        match self {
            GpuProcessKind::Compute => "compute",
            GpuProcessKind::Graphics => "graphics",
            GpuProcessKind::Both => "both",
            GpuProcessKind::Unknown => "unknown",
        }
    }

    /// Merge two observations of the same PID on the same device.
    ///
    /// Compute + Graphics collapses to [`Self::Both`]; anything merged with
    /// itself is unchanged. [`Self::Unknown`] never upgrades a known kind.
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (a, b) if a == b => a,
            (GpuProcessKind::Unknown, b) => b,
            (a, GpuProcessKind::Unknown) => a,
            // Compute + Graphics in either order, or anything already Both.
            _ => GpuProcessKind::Both,
        }
    }
}

/// Per-device snapshot.
///
/// Every metric is `Option` — see the module doc. `None` means "this backend
/// cannot report this on this device", never "zero".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct GpuDeviceSnapshot {
    /// muxtop-assigned index, stable within one snapshot and ordered by
    /// backend then by the backend's own enumeration order. **Not** a stable
    /// hardware identifier across reboots — use `bus_id` for that.
    pub index: u32,
    pub vendor: GpuVendor,
    pub backend: GpuBackend,
    /// Marketing name reported by the driver (e.g. `NVIDIA GeForce RTX 4090`).
    ///
    /// Driver-provided and therefore **not trusted for rendering** — the TUI
    /// scrubs it like any other foreign string.
    pub name: String,
    /// PCI bus id (`0000:01:00.0`), empty when the backend cannot report one.
    pub bus_id: String,
    /// Kernel/driver version string, when the backend exposes it.
    pub driver_version: Option<String>,
    /// Core utilisation, 0–100 %.
    pub utilization_pct: Option<f32>,
    /// Memory-controller utilisation, 0–100 %. Distinct from
    /// `mem_used_bytes`: this is bandwidth pressure, not occupancy.
    pub mem_utilization_pct: Option<f32>,
    pub mem_used_bytes: Option<u64>,
    pub mem_total_bytes: Option<u64>,
    /// Die temperature in °C.
    pub temperature_c: Option<f32>,
    pub power_watts: Option<f32>,
    /// Enforced power cap in W, when the driver exposes one.
    pub power_limit_watts: Option<f32>,
    pub graphics_clock_mhz: Option<u32>,
    pub memory_clock_mhz: Option<u32>,
    /// Fan duty cycle, 0–100 %. `None` on passively-cooled and
    /// datacentre cards, which genuinely have no fan to report.
    pub fan_pct: Option<f32>,
    /// Hardware video **encoder** utilisation, 0–100 % (NVML only).
    pub encoder_pct: Option<f32>,
    /// Hardware video **decoder** utilisation, 0–100 % (NVML only).
    pub decoder_pct: Option<f32>,
    /// Whether this device's backend can enumerate per-process GPU usage.
    ///
    /// Drives the Procs sub-view's explanatory fallback: AMD's sysfs
    /// interface exposes no per-process accounting, so an empty process list
    /// there means "not supported", not "nothing is using the GPU".
    pub supports_process_stats: bool,
}

impl GpuDeviceSnapshot {
    /// Memory occupancy as a percentage, or `None` when either side is
    /// unknown or the device reports a zero-byte total.
    pub fn mem_pct(&self) -> Option<f32> {
        let (used, total) = (self.mem_used_bytes?, self.mem_total_bytes?);
        if total == 0 {
            return None;
        }
        Some((used as f64 / total as f64 * 100.0) as f32)
    }

    /// Power draw as a percentage of the enforced limit, when both are known.
    pub fn power_pct(&self) -> Option<f32> {
        let (used, limit) = (self.power_watts?, self.power_limit_watts?);
        if limit <= 0.0 {
            return None;
        }
        Some(used / limit * 100.0)
    }
}

/// Per-process GPU usage.
///
/// One row per (pid, device) pair — a process spanning two GPUs appears
/// twice, which is what `nvidia-smi` does and what users expect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct GpuProcessSnapshot {
    pub pid: u32,
    /// Index of the [`GpuDeviceSnapshot`] this row belongs to.
    pub device_index: u32,
    /// Process name resolved by the engine from the host process table.
    /// Empty when the PID vanished between the GPU query and the lookup.
    ///
    /// Attacker-controlled: any user can name a process whatever they like.
    /// The TUI scrubs it before rendering.
    pub name: String,
    pub kind: GpuProcessKind,
    /// GPU memory held by this process, in bytes.
    ///
    /// `None` is common and expected on Windows: under the WDDM driver model
    /// the OS owns video memory allocation, so NVML reports the value as
    /// unavailable rather than guessing.
    pub mem_bytes: Option<u64>,
}

/// Aggregated GPU snapshot for a single muxtop tick.
///
/// `available = false` with empty vecs is the canonical "no GPU backend"
/// state, mirroring `KubeSnapshot::reachable` and
/// `ContainersSnapshot::daemon_up`. It covers three distinct situations the
/// UI distinguishes via [`Self::detail`]: no supported hardware, a driver
/// that failed to load, and `--no-gpu`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct GpusSnapshot {
    /// Backends that initialised successfully. Empty when `available` is
    /// false.
    pub backends: Vec<GpuBackend>,
    pub available: bool,
    pub devices: Vec<GpuDeviceSnapshot>,
    pub processes: Vec<GpuProcessSnapshot>,
    /// Human-readable reason the snapshot is empty or degraded, rendered
    /// under the "No GPU" placeholder. Empty when there is nothing to say.
    ///
    /// Engine-authored, never user input — but it crosses the wire in
    /// `--remote` mode, so the TUI scrubs it like every other foreign string.
    pub detail: String,
}

impl GpusSnapshot {
    /// Canonical empty snapshot when no GPU backend is usable.
    ///
    /// Mirrors `ContainersSnapshot::unavailable()` / `KubeSnapshot::unavailable()`.
    pub fn unavailable() -> Self {
        Self {
            backends: Vec::new(),
            available: false,
            devices: Vec::new(),
            processes: Vec::new(),
            detail: String::new(),
        }
    }

    /// [`Self::unavailable`] carrying an explanation for the UI.
    pub fn unavailable_with(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
            ..Self::unavailable()
        }
    }

    /// Whether any device in this snapshot can report per-process usage.
    ///
    /// The Procs sub-view uses this to tell "no process is using the GPU"
    /// apart from "this backend cannot answer that question".
    pub fn any_process_stats(&self) -> bool {
        self.devices.iter().any(|d| d.supports_process_stats)
    }

    /// Fill in [`GpuProcessSnapshot::name`] from the host process table.
    ///
    /// GPU backends report PIDs, not names: NVML has no notion of a command
    /// line, and resolving one inside the backend would mean a second full
    /// process enumeration every tick. The collector already refreshes the
    /// process table for the Processes tab, so the lookup is free there —
    /// [`crate::system::SystemSnapshot::collect`] calls this with a closure
    /// over that table.
    ///
    /// A PID that has exited between the GPU query and this call keeps its
    /// empty name; the row still carries a real PID and real memory figure,
    /// which is more useful than dropping it.
    pub fn resolve_process_names<F>(&mut self, lookup: F)
    where
        F: Fn(u32) -> Option<String>,
    {
        for process in &mut self.processes {
            if process.name.is_empty()
                && let Some(name) = lookup(process.pid)
            {
                process.name = name;
            }
        }
    }

    /// Total VRAM used / total across every device that reports both.
    ///
    /// Returns `None` when no device reports memory at all, so the summary
    /// bar can omit the figure rather than print a misleading `0 / 0`.
    pub fn total_memory(&self) -> Option<(u64, u64)> {
        let mut used = 0u64;
        let mut total = 0u64;
        let mut seen = false;
        for d in &self.devices {
            if let (Some(u), Some(t)) = (d.mem_used_bytes, d.mem_total_bytes) {
                used = used.saturating_add(u);
                total = total.saturating_add(t);
                seen = true;
            }
        }
        seen.then_some((used, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bincode::config;

    fn sample_device() -> GpuDeviceSnapshot {
        GpuDeviceSnapshot {
            index: 0,
            vendor: GpuVendor::Nvidia,
            backend: GpuBackend::Nvml,
            name: "NVIDIA GeForce RTX 4090".into(),
            bus_id: "0000:01:00.0".into(),
            driver_version: Some("560.35.03".into()),
            utilization_pct: Some(42.0),
            mem_utilization_pct: Some(18.0),
            mem_used_bytes: Some(6 * 1024 * 1024 * 1024),
            mem_total_bytes: Some(24 * 1024 * 1024 * 1024),
            temperature_c: Some(64.0),
            power_watts: Some(210.5),
            power_limit_watts: Some(450.0),
            graphics_clock_mhz: Some(2520),
            memory_clock_mhz: Some(10501),
            fan_pct: Some(38.0),
            encoder_pct: Some(0.0),
            decoder_pct: Some(12.0),
            supports_process_stats: true,
        }
    }

    fn sample_process() -> GpuProcessSnapshot {
        GpuProcessSnapshot {
            pid: 4242,
            device_index: 0,
            name: "ollama".into(),
            kind: GpuProcessKind::Compute,
            mem_bytes: Some(3 * 1024 * 1024 * 1024),
        }
    }

    // ---- wire round-trips -------------------------------------------------

    #[test]
    fn gpu_device_snapshot_derive_round_trip() {
        let original = sample_device();
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (GpuDeviceSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn gpu_process_snapshot_derive_round_trip() {
        let original = sample_process();
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (GpuProcessSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn gpus_snapshot_derive_round_trip() {
        let original = GpusSnapshot {
            backends: vec![GpuBackend::Nvml, GpuBackend::AmdSysfs],
            available: true,
            devices: vec![sample_device()],
            processes: vec![sample_process()],
            detail: String::new(),
        };
        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&original, cfg).expect("encode");
        let (decoded, _len): (GpusSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(original, decoded);
    }

    #[test]
    fn none_metrics_survive_the_wire() {
        // The whole graceful-degradation contract rests on `None` crossing
        // the wire as `None`. If bincode ever coerced it to a default the
        // remote UI would render a confident `0` for an unknown metric.
        let mut device = sample_device();
        device.utilization_pct = None;
        device.temperature_c = None;
        device.power_watts = None;
        device.mem_used_bytes = None;

        let cfg = config::standard();
        let bytes = bincode::encode_to_vec(&device, cfg).expect("encode");
        let (decoded, _): (GpuDeviceSnapshot, usize) =
            bincode::decode_from_slice(&bytes, cfg).expect("decode");
        assert_eq!(decoded.utilization_pct, None);
        assert_eq!(decoded.temperature_c, None);
        assert_eq!(decoded.power_watts, None);
        assert_eq!(decoded.mem_used_bytes, None);
    }

    #[test]
    fn vendor_and_backend_round_trip() {
        let cfg = config::standard();
        for vendor in [
            GpuVendor::Nvidia,
            GpuVendor::Amd,
            GpuVendor::Intel,
            GpuVendor::Apple,
            GpuVendor::Unknown,
        ] {
            let bytes = bincode::encode_to_vec(vendor, cfg).expect("encode");
            let (decoded, _): (GpuVendor, usize) =
                bincode::decode_from_slice(&bytes, cfg).expect("decode");
            assert_eq!(vendor, decoded);
        }
        for backend in [
            GpuBackend::Nvml,
            GpuBackend::AmdSysfs,
            GpuBackend::AppleIoReport,
        ] {
            let bytes = bincode::encode_to_vec(backend, cfg).expect("encode");
            let (decoded, _): (GpuBackend, usize) =
                bincode::decode_from_slice(&bytes, cfg).expect("decode");
            assert_eq!(backend, decoded);
        }
    }

    // ---- exhaustiveness guards -------------------------------------------

    #[test]
    fn vendor_labels_are_exhaustive() {
        // Exhaustive match without wildcard — a new variant breaks this test
        // and every downstream UI match along with it.
        for vendor in [
            GpuVendor::Nvidia,
            GpuVendor::Amd,
            GpuVendor::Intel,
            GpuVendor::Apple,
            GpuVendor::Unknown,
        ] {
            let label: &'static str = match vendor {
                GpuVendor::Nvidia => "nvidia",
                GpuVendor::Amd => "amd",
                GpuVendor::Intel => "intel",
                GpuVendor::Apple => "apple",
                GpuVendor::Unknown => "unknown",
            };
            assert_eq!(vendor.label(), label);
        }
    }

    #[test]
    fn backend_labels_are_exhaustive() {
        for backend in [
            GpuBackend::Nvml,
            GpuBackend::AmdSysfs,
            GpuBackend::AppleIoReport,
        ] {
            assert!(!backend.label().is_empty());
        }
    }

    // ---- GpuProcessKind::merge -------------------------------------------

    #[test]
    fn process_kind_merge_promotes_to_both() {
        assert_eq!(
            GpuProcessKind::Compute.merge(GpuProcessKind::Graphics),
            GpuProcessKind::Both
        );
        assert_eq!(
            GpuProcessKind::Graphics.merge(GpuProcessKind::Compute),
            GpuProcessKind::Both
        );
    }

    #[test]
    fn process_kind_merge_is_idempotent() {
        for kind in [
            GpuProcessKind::Compute,
            GpuProcessKind::Graphics,
            GpuProcessKind::Both,
            GpuProcessKind::Unknown,
        ] {
            assert_eq!(kind.merge(kind), kind);
        }
    }

    #[test]
    fn process_kind_merge_never_downgrades_to_unknown() {
        for kind in [
            GpuProcessKind::Compute,
            GpuProcessKind::Graphics,
            GpuProcessKind::Both,
        ] {
            assert_eq!(kind.merge(GpuProcessKind::Unknown), kind);
            assert_eq!(GpuProcessKind::Unknown.merge(kind), kind);
        }
    }

    #[test]
    fn process_kind_merge_keeps_both_absorbing() {
        assert_eq!(
            GpuProcessKind::Both.merge(GpuProcessKind::Compute),
            GpuProcessKind::Both
        );
        assert_eq!(
            GpuProcessKind::Compute.merge(GpuProcessKind::Both),
            GpuProcessKind::Both
        );
    }

    // ---- derived helpers --------------------------------------------------

    #[test]
    fn mem_pct_computes_occupancy() {
        let device = sample_device();
        let pct = device.mem_pct().expect("both sides known");
        assert!((pct - 25.0).abs() < 0.01, "expected 25%, got {pct}");
    }

    #[test]
    fn mem_pct_is_none_when_either_side_unknown() {
        let mut device = sample_device();
        device.mem_used_bytes = None;
        assert_eq!(device.mem_pct(), None);

        let mut device = sample_device();
        device.mem_total_bytes = None;
        assert_eq!(device.mem_pct(), None);
    }

    #[test]
    fn mem_pct_is_none_on_zero_total() {
        // Guard against a divide-by-zero rendering `inf%` or `NaN%`.
        let mut device = sample_device();
        device.mem_total_bytes = Some(0);
        assert_eq!(device.mem_pct(), None);
    }

    #[test]
    fn power_pct_computes_headroom() {
        let device = sample_device();
        let pct = device.power_pct().expect("both sides known");
        assert!((pct - 46.777).abs() < 0.01, "expected ~46.8%, got {pct}");
    }

    #[test]
    fn power_pct_is_none_on_zero_limit() {
        let mut device = sample_device();
        device.power_limit_watts = Some(0.0);
        assert_eq!(device.power_pct(), None);
    }

    // ---- GpusSnapshot -----------------------------------------------------

    #[test]
    fn gpus_snapshot_unavailable_is_empty() {
        let s = GpusSnapshot::unavailable();
        assert!(!s.available);
        assert!(s.devices.is_empty());
        assert!(s.processes.is_empty());
        assert!(s.backends.is_empty());
        assert!(s.detail.is_empty());
    }

    #[test]
    fn gpus_snapshot_unavailable_with_carries_detail() {
        let s = GpusSnapshot::unavailable_with("no NVIDIA driver loaded");
        assert!(!s.available);
        assert_eq!(s.detail, "no NVIDIA driver loaded");
    }

    #[test]
    fn any_process_stats_reflects_devices() {
        let mut snap = GpusSnapshot::unavailable();
        assert!(!snap.any_process_stats());

        let mut amd = sample_device();
        amd.supports_process_stats = false;
        snap.devices.push(amd);
        assert!(!snap.any_process_stats());

        snap.devices.push(sample_device());
        assert!(snap.any_process_stats());
    }

    #[test]
    fn total_memory_sums_reporting_devices() {
        let mut snap = GpusSnapshot::unavailable();
        snap.devices.push(sample_device());
        snap.devices.push(sample_device());
        let (used, total) = snap.total_memory().expect("two reporting devices");
        assert_eq!(used, 12 * 1024 * 1024 * 1024);
        assert_eq!(total, 48 * 1024 * 1024 * 1024);
    }

    #[test]
    fn total_memory_is_none_when_nobody_reports() {
        let mut snap = GpusSnapshot::unavailable();
        let mut device = sample_device();
        device.mem_used_bytes = None;
        device.mem_total_bytes = None;
        snap.devices.push(device);
        assert_eq!(snap.total_memory(), None);
    }

    #[test]
    fn total_memory_skips_partial_reporters() {
        // A device reporting a total but no usage must not contribute its
        // total — that would understate the fleet-wide occupancy percentage.
        let mut snap = GpusSnapshot::unavailable();
        let mut partial = sample_device();
        partial.mem_used_bytes = None;
        snap.devices.push(partial);
        snap.devices.push(sample_device());

        let (used, total) = snap.total_memory().expect("one full reporter");
        assert_eq!(used, 6 * 1024 * 1024 * 1024);
        assert_eq!(total, 24 * 1024 * 1024 * 1024);
    }

    // ---- process-name resolution -----------------------------------------

    #[test]
    fn resolve_process_names_fills_from_the_lookup() {
        let mut snap = GpusSnapshot::unavailable();
        snap.processes.push(GpuProcessSnapshot {
            pid: 7,
            device_index: 0,
            name: String::new(),
            kind: GpuProcessKind::Compute,
            mem_bytes: None,
        });

        snap.resolve_process_names(|pid| (pid == 7).then(|| "python3".to_string()));
        assert_eq!(snap.processes[0].name, "python3");
    }

    #[test]
    fn resolve_process_names_leaves_exited_pids_empty() {
        // A process that died between the GPU query and the lookup keeps an
        // empty name — the row is still worth showing for its PID and memory.
        let mut snap = GpusSnapshot::unavailable();
        snap.processes.push(GpuProcessSnapshot {
            pid: 999_999,
            device_index: 0,
            name: String::new(),
            kind: GpuProcessKind::Graphics,
            mem_bytes: Some(1024),
        });

        snap.resolve_process_names(|_| None);
        assert!(snap.processes[0].name.is_empty());
        assert_eq!(snap.processes[0].mem_bytes, Some(1024));
    }

    #[test]
    fn resolve_process_names_does_not_overwrite_a_known_name() {
        // A backend that *can* name its processes must win over the host
        // table, which may have recycled the PID.
        let mut snap = GpusSnapshot::unavailable();
        snap.processes.push(GpuProcessSnapshot {
            pid: 7,
            device_index: 0,
            name: "from-backend".into(),
            kind: GpuProcessKind::Compute,
            mem_bytes: None,
        });

        snap.resolve_process_names(|_| Some("from-host-table".to_string()));
        assert_eq!(snap.processes[0].name, "from-backend");
    }

    #[test]
    fn snapshots_are_send_clone() {
        fn assert_send_clone<T: Send + Clone>() {}
        assert_send_clone::<GpuDeviceSnapshot>();
        assert_send_clone::<GpuProcessSnapshot>();
        assert_send_clone::<GpusSnapshot>();
    }
}
