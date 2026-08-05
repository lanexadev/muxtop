//! NVIDIA backend for the GPU tab, via the NVIDIA Management Library.
//!
//! Compiled on Linux and Windows only — NVML ships no macOS build, and
//! Apple dropped NVIDIA support after Kepler. The module is gated at the
//! `lib.rs` level so the dependency itself is absent on other targets (see
//! the v0.4.2 Windows lesson: a platform-specific call with no `cfg` gate
//! broke the whole workspace build).
//!
//! # Why `nvml-wrapper` and not `nvidia-smi`
//!
//! Shelling out to `nvidia-smi` costs a process spawn per tick, returns
//! locale-dependent text, and would make muxtop's output depend on a binary
//! it does not control. NVML is the same library `nvidia-smi` itself calls.
//!
//! # The driver is a runtime dependency, not a build dependency
//!
//! [`nvml_wrapper::Nvml::init`] resolves `libnvidia-ml.so` / `nvml.dll`
//! through `libloading` at **runtime**. muxtop therefore builds and runs
//! identically on machines with no NVIDIA hardware — [`NvmlEngine::connect`]
//! simply returns [`GpuError::DriverUnavailable`] and the composite engine
//! moves on to the next backend. No feature flag, no separate build.
//!
//! # Everything is best-effort
//!
//! Which metrics NVML answers depends on the card, the driver branch and the
//! OS. A consumer GeForce refuses `enforced_power_limit` on some branches; a
//! datacentre A100 has no fan; encoder utilisation is absent on cards without
//! NVENC. Every query is therefore `.ok()`-ed into an `Option` — a failed
//! query yields `None`, never a zero, and never aborts the snapshot.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use nvml_wrapper::Nvml;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::enums::device::UsedGpuMemory;

use crate::gpu::{
    GpuBackend, GpuDeviceSnapshot, GpuProcessKind, GpuProcessSnapshot, GpuVendor, GpusSnapshot,
};
use crate::gpu_engine::{GpuEngine, GpuError};

/// NVIDIA telemetry source backed by a dynamically-loaded NVML.
pub struct NvmlEngine {
    /// `Arc` because every snapshot moves a handle into `spawn_blocking`.
    nvml: Arc<Nvml>,
    /// Driver version, read once at connect time — it cannot change while
    /// the process lives, so re-querying it every tick would be waste.
    driver_version: Option<String>,
}

impl NvmlEngine {
    /// Initialise NVML and verify at least one device is present.
    ///
    /// Returns [`GpuError::DriverUnavailable`] when the library is missing,
    /// the driver is not loaded, or the machine simply has no NVIDIA GPU.
    /// All three are ordinary conditions on a machine without NVIDIA
    /// hardware, so callers log them at debug level rather than warning.
    pub fn connect() -> Result<Self, GpuError> {
        let nvml = Nvml::init().map_err(|e| GpuError::DriverUnavailable {
            vendor: "NVIDIA",
            reason: e.to_string(),
        })?;

        // An NVML that initialises but sees no device is the normal state on
        // a host with the driver installed and the card removed (or a laptop
        // whose dGPU is disabled in firmware). Treat it as "no backend"
        // rather than exposing an empty GPU tab.
        let count = nvml
            .device_count()
            .map_err(|e| GpuError::DriverUnavailable {
                vendor: "NVIDIA",
                reason: format!("device enumeration failed: {e}"),
            })?;
        if count == 0 {
            return Err(GpuError::DriverUnavailable {
                vendor: "NVIDIA",
                reason: "NVML loaded but reports no devices".to_string(),
            });
        }

        let driver_version = nvml.sys_driver_version().ok();

        Ok(Self {
            nvml: Arc::new(nvml),
            driver_version,
        })
    }
}

#[async_trait]
impl GpuEngine for NvmlEngine {
    async fn snapshot(&self) -> Result<GpusSnapshot, GpuError> {
        // NVML is a synchronous C library: every call blocks the calling
        // thread. Running it inline would stall the runtime worker that also
        // drives the container and cluster loops.
        let nvml = Arc::clone(&self.nvml);
        let driver_version = self.driver_version.clone();

        tokio::task::spawn_blocking(move || collect(&nvml, driver_version))
            .await
            .map_err(|e| {
                GpuError::Query(format!(
                    "NVML collection task panicked or was cancelled: {e}"
                ))
            })?
    }

    fn backend(&self) -> GpuBackend {
        GpuBackend::Nvml
    }
}

/// Blocking collection body. Kept free-standing so it can run inside
/// `spawn_blocking` without borrowing the engine.
fn collect(nvml: &Nvml, driver_version: Option<String>) -> Result<GpusSnapshot, GpuError> {
    let count = nvml
        .device_count()
        .map_err(|e| GpuError::Query(format!("nvmlDeviceGetCount failed: {e}")))?;

    let mut devices = Vec::with_capacity(count as usize);
    let mut processes = Vec::new();

    for index in 0..count {
        let device = match nvml.device_by_index(index) {
            Ok(d) => d,
            Err(err) => {
                // A single unreadable device (ECC fault, GPU reset in
                // progress, MIG reconfiguration) must not blank the others.
                tracing::debug!(
                    target: "muxtop::gpu",
                    index,
                    error = %err,
                    "skipping unreadable NVML device"
                );
                continue;
            }
        };

        let utilization = device.utilization_rates().ok();
        let memory = device.memory_info().ok();

        // `enforced_power_limit` is what the card will actually respect;
        // `power_management_limit` is the configured target. Prefer the
        // former and fall back, because a user reading "210 W / 450 W" wants
        // the cap that is really in force.
        let power_limit_mw = device
            .enforced_power_limit()
            .or_else(|_| device.power_management_limit())
            .ok();

        devices.push(GpuDeviceSnapshot {
            // Local to this backend — `CompositeGpuEngine` renumbers.
            index: devices.len() as u32,
            vendor: GpuVendor::Nvidia,
            backend: GpuBackend::Nvml,
            name: device.name().unwrap_or_default(),
            bus_id: device.pci_info().map(|p| p.bus_id).unwrap_or_default(),
            driver_version: driver_version.clone(),
            utilization_pct: utilization.as_ref().map(|u| u.gpu as f32),
            mem_utilization_pct: utilization.as_ref().map(|u| u.memory as f32),
            mem_used_bytes: memory.as_ref().map(|m| m.used),
            mem_total_bytes: memory.as_ref().map(|m| m.total),
            temperature_c: device
                .temperature(TemperatureSensor::Gpu)
                .ok()
                .map(|t| t as f32),
            // NVML reports power in milliwatts.
            power_watts: device.power_usage().ok().map(|mw| mw as f32 / 1000.0),
            power_limit_watts: power_limit_mw.map(|mw| mw as f32 / 1000.0),
            graphics_clock_mhz: device.clock_info(Clock::Graphics).ok(),
            memory_clock_mhz: device.clock_info(Clock::Memory).ok(),
            // Fan 0 stands in for the whole card. Multi-fan boards report
            // near-identical duty cycles, and a single percentage is what the
            // column has room for; cards with no fan return an error here and
            // land on `None`, which is the honest answer.
            fan_pct: device.fan_speed(0).ok().map(|f| f as f32),
            encoder_pct: device
                .encoder_utilization()
                .ok()
                .map(|u| u.utilization as f32),
            decoder_pct: device
                .decoder_utilization()
                .ok()
                .map(|u| u.utilization as f32),
            supports_process_stats: true,
        });

        let device_index = (devices.len() - 1) as u32;
        collect_processes(&device, device_index, &mut processes);
    }

    if devices.is_empty() {
        return Ok(GpusSnapshot::unavailable_with(
            "NVML reported devices but none could be read",
        ));
    }

    Ok(GpusSnapshot {
        backends: vec![GpuBackend::Nvml],
        available: true,
        devices,
        processes,
        detail: String::new(),
    })
}

/// Append this device's compute and graphics processes to `out`, merging a
/// PID that appears in both lists into a single [`GpuProcessKind::Both`] row.
///
/// # Windows reports everything as `Both`
///
/// Measured against a real RTX 3080 on Windows 11: `running_compute_processes`
/// and `running_graphics_processes` return the *identical* set (27 PIDs each,
/// full overlap). Under WDDM the driver does not distinguish the two context
/// types, so every row legitimately merges to [`GpuProcessKind::Both`] and the
/// TYPE column carries no information there. On Linux the two lists differ
/// and the distinction is meaningful. This is the driver's answer, not a merge
/// bug — the merge is what makes one row out of the two identical lists
/// instead of showing every process twice.
///
/// `name` is deliberately left empty: resolving a PID to a process name is
/// the collector's job, which already holds a refreshed process table (see
/// `GpusSnapshot::resolve_process_names`). Doing it here would mean a second
/// full process enumeration per tick.
fn collect_processes(
    device: &nvml_wrapper::Device<'_>,
    device_index: u32,
    out: &mut Vec<GpuProcessSnapshot>,
) {
    // Insertion-ordered accumulation keyed by PID so the compute and graphics
    // lists merge instead of producing two rows for the same process.
    let mut merged: HashMap<u32, (GpuProcessKind, Option<u64>)> = HashMap::new();
    let mut order: Vec<u32> = Vec::new();

    fn absorb(
        procs: Vec<nvml_wrapper::struct_wrappers::device::ProcessInfo>,
        kind: GpuProcessKind,
        merged: &mut HashMap<u32, (GpuProcessKind, Option<u64>)>,
        order: &mut Vec<u32>,
    ) {
        for proc_info in procs {
            let mem = match proc_info.used_gpu_memory {
                // Windows/WDDM always reports Unavailable: the OS, not the
                // driver, owns video memory allocation there.
                UsedGpuMemory::Unavailable => None,
                UsedGpuMemory::Used(bytes) => Some(bytes),
            };
            merged
                .entry(proc_info.pid)
                .and_modify(|(k, m)| {
                    *k = k.merge(kind);
                    // Keep whichever list actually reported a figure.
                    if m.is_none() {
                        *m = mem;
                    }
                })
                .or_insert_with(|| {
                    order.push(proc_info.pid);
                    (kind, mem)
                });
        }
    }

    if let Ok(procs) = device.running_compute_processes() {
        absorb(procs, GpuProcessKind::Compute, &mut merged, &mut order);
    }
    if let Ok(procs) = device.running_graphics_processes() {
        absorb(procs, GpuProcessKind::Graphics, &mut merged, &mut order);
    }

    for pid in order {
        let (kind, mem_bytes) = merged[&pid];
        out.push(GpuProcessSnapshot {
            pid,
            device_index,
            name: String::new(),
            kind,
            mem_bytes,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Arc<Nvml>` is moved into `spawn_blocking`, which demands `Send`, and
    /// shared across snapshots, which demands `Sync`. If a future
    /// `nvml-wrapper` release loses either, this fails at compile time here
    /// rather than at the call site with an inscrutable future-is-not-Send
    /// error.
    #[test]
    fn nvml_handle_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Nvml>();
        assert_send_sync::<NvmlEngine>();
    }

    /// Connecting on a machine with no NVIDIA driver must fail cleanly with a
    /// `DriverUnavailable` naming the vendor — never panic, never hang. This
    /// runs in CI on GPU-less runners, which is exactly the path that matters.
    #[test]
    fn connect_without_driver_reports_driver_unavailable() {
        match NvmlEngine::connect() {
            Ok(engine) => {
                // A CI runner with a real GPU: assert the happy path shape
                // instead of skipping the test entirely.
                assert_eq!(engine.backend(), GpuBackend::Nvml);
            }
            Err(err) => {
                assert!(
                    matches!(
                        err,
                        GpuError::DriverUnavailable {
                            vendor: "NVIDIA",
                            ..
                        }
                    ),
                    "expected DriverUnavailable, got {err:?}"
                );
                // The reason must be non-empty — it is what the UI shows the
                // user under "No GPU".
                assert!(!err.to_string().is_empty());
            }
        }
    }

    /// End-to-end on whatever the host has. On a GPU-less machine `connect`
    /// fails and there is nothing to assert; on a real GPU we check the
    /// invariants the UI relies on.
    #[tokio::test]
    async fn snapshot_invariants_hold_on_real_hardware() {
        let Ok(engine) = NvmlEngine::connect() else {
            return;
        };
        let snap = engine.snapshot().await.expect("snapshot must not error");

        assert!(snap.available);
        assert!(!snap.devices.is_empty());
        assert_eq!(snap.backends, vec![GpuBackend::Nvml]);

        // Indices are dense from zero — the composite's remapping assumes it.
        for (i, device) in snap.devices.iter().enumerate() {
            assert_eq!(device.index, i as u32);
            assert_eq!(device.vendor, GpuVendor::Nvidia);
            assert!(device.supports_process_stats);
            if let Some(pct) = device.utilization_pct {
                assert!(
                    (0.0..=100.0).contains(&pct),
                    "utilisation out of range: {pct}"
                );
            }
            if let (Some(used), Some(total)) = (device.mem_used_bytes, device.mem_total_bytes) {
                assert!(used <= total, "used {used} exceeds total {total}");
            }
        }

        // Every process points at a device that exists.
        for p in &snap.processes {
            assert!(
                snap.devices.iter().any(|d| d.index == p.device_index),
                "process {} points at missing device {}",
                p.pid,
                p.device_index
            );
        }

        // The engine leaves names for the collector to resolve.
        assert!(
            snap.processes.iter().all(|p| p.name.is_empty()),
            "the NVML backend must not resolve process names itself"
        );
    }

    /// A PID that shows up in both the compute and the graphics list must
    /// produce one row, not two. Exercised through the merge helper because
    /// `nvml_wrapper::Device` cannot be constructed without hardware.
    #[test]
    fn process_kind_merge_collapses_dual_use() {
        let merged = GpuProcessKind::Compute.merge(GpuProcessKind::Graphics);
        assert_eq!(merged, GpuProcessKind::Both);
    }
}
