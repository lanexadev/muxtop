//! The Apple Silicon [`GpuEngine`] implementation.
//!
//! Assembles a [`GpusSnapshot`] from the two sources this module wraps: the
//! IORegistry (always available, public API) and `IOReport` (best effort,
//! private framework). See `ioregistry.rs` and `ioreport.rs` for what each one
//! is allowed to fail at.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::apple::ioregistry::{self, AcceleratorInfo};
use crate::apple::metrics::{
    DvfsTable, active_residency_pct, clamp_pct, energy_delta_to_watts, parse_dvfs_table,
    residency_weighted_clock_mhz,
};
use crate::gpu::{GpuBackend, GpuDeviceSnapshot, GpuVendor, GpusSnapshot};
use crate::gpu_engine::{GpuEngine, GpuError, MACOS_UNSUPPORTED_GPU_DETAIL};

use super::ioreport::IoReportSampler;

/// Apple Silicon telemetry source.
///
/// The state lives behind an [`Arc`] so [`GpuEngine::snapshot`] can hand it to
/// `spawn_blocking`: IOKit and `IOReport` are synchronous C APIs, and the
/// runtime worker that would otherwise run them also drives the container and
/// cluster loops.
pub struct AppleEngine {
    inner: Arc<Inner>,
}

struct Inner {
    /// Total unified memory. Read once — it cannot change while the process
    /// runs, and it is the denominator for every MEM% this engine reports.
    unified_memory_bytes: Option<u64>,
    /// Decoded GPU DVFS table, or `None` when the firmware blob could not be
    /// trusted. Read once for the same reason.
    dvfs: Option<DvfsTable>,
    /// `IOReport` subscription, absent when the private framework could not be
    /// opened. `Mutex` because sampling mutates the held previous sample and
    /// the collector shares the engine across tasks — it is also what makes
    /// the sampler's `Send` implementation sound.
    sampler: Mutex<Option<IoReportSampler>>,
}

impl AppleEngine {
    /// Probe the host for an Apple GPU.
    ///
    /// Fails on a Mac whose accelerator is not driven by Apple's own `AGX`
    /// family — an Intel Mac's AMD or Intel GPU exposes a differently shaped
    /// `PerformanceStatistics` dictionary, and reading it with these keys
    /// would produce a row labelled Apple that reports nothing.
    pub fn connect() -> Result<Self, GpuError> {
        let accelerators = ioregistry::accelerators();
        if accelerators.is_empty() {
            return Err(GpuError::DriverUnavailable {
                vendor: "Apple",
                reason: "no IOAccelerator service in the IORegistry".into(),
            });
        }
        if !accelerators.iter().any(AcceleratorInfo::is_apple_gpu) {
            return Err(GpuError::Unsupported(MACOS_UNSUPPORTED_GPU_DETAIL));
        }

        // Both of the following are allowed to fail. Losing the DVFS table
        // costs the CLK column; losing IOReport costs CLK and POWER. Neither
        // costs the tab, which is the whole point of layering the two sources.
        let dvfs = ioregistry::gpu_dvfs_blob()
            .as_deref()
            .and_then(parse_dvfs_table);
        if dvfs.is_none() {
            tracing::debug!(
                target: "muxtop::gpu",
                "GPU DVFS table unreadable; clocks will render as unavailable"
            );
        }

        let sampler = match IoReportSampler::open() {
            Ok(sampler) => Some(sampler),
            Err(err) => {
                tracing::debug!(
                    target: "muxtop::gpu",
                    error = %err,
                    "IOReport unavailable; power and clocks will render as unavailable"
                );
                None
            }
        };

        Ok(Self {
            inner: Arc::new(Inner {
                unified_memory_bytes: ioregistry::unified_memory_bytes(),
                dvfs,
                sampler: Mutex::new(sampler),
            }),
        })
    }
}

impl Inner {
    /// Build one snapshot. Blocking: every call underneath is synchronous.
    fn collect(&self) -> GpusSnapshot {
        // The IOReport interval covers the whole SoC. Apple Silicon has
        // exactly one GPU, so there is nothing to attribute between devices.
        let interval = self
            .sampler
            // A panic inside `sample` would poison the lock, but the sampler's
            // own invariants survive it (the worst case is a stale previous
            // sample, which the next delta corrects). Losing the GPU tab for
            // the rest of the session would be the larger harm.
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_mut()
            .and_then(IoReportSampler::sample);

        let power_watts = interval.as_ref().and_then(|report| {
            let (delta, unit) = report.energy.as_ref()?;
            energy_delta_to_watts(*delta, unit, report.elapsed)
        });
        let residency = interval
            .as_ref()
            .zip(self.dvfs.as_ref())
            .map(|(report, table)| (report.residencies.as_slice(), table));
        let graphics_clock_mhz =
            residency.and_then(|(res, table)| residency_weighted_clock_mhz(res, table));
        let residency_pct = residency.and_then(|(res, table)| active_residency_pct(res, table));

        let devices: Vec<GpuDeviceSnapshot> = ioregistry::accelerators()
            .into_iter()
            .filter(AcceleratorInfo::is_apple_gpu)
            .enumerate()
            .map(|(index, info)| DeviceInputs {
                index: index as u32,
                info,
                power_watts,
                graphics_clock_mhz,
                residency_pct,
                unified_memory_bytes: self.unified_memory_bytes,
            })
            .map(build_device)
            .collect();

        if devices.is_empty() {
            // The accelerator was there at connect time and is gone now. Not
            // expected on hardware that cannot be unplugged, but saying so
            // beats inventing a device.
            return GpusSnapshot::unavailable_with("the Apple GPU disappeared from the IORegistry");
        }

        GpusSnapshot {
            backends: vec![GpuBackend::AppleIoReport],
            available: true,
            devices,
            // Apple exposes no per-process GPU accounting — see the module doc.
            processes: Vec::new(),
            detail: String::new(),
        }
    }
}

/// Everything needed to render one device row, gathered so the mapping below
/// stays a single expression rather than a six-argument call.
struct DeviceInputs {
    index: u32,
    info: AcceleratorInfo,
    power_watts: Option<f32>,
    graphics_clock_mhz: Option<u32>,
    residency_pct: Option<f32>,
    unified_memory_bytes: Option<u64>,
}

fn build_device(input: DeviceInputs) -> GpuDeviceSnapshot {
    let DeviceInputs {
        index,
        info,
        power_watts,
        graphics_clock_mhz,
        residency_pct,
        unified_memory_bytes,
    } = input;

    GpuDeviceSnapshot {
        index,
        vendor: GpuVendor::Apple,
        backend: GpuBackend::AppleIoReport,
        name: device_name(&info),
        // The GPU is on the SoC die: there is no PCI bus and therefore no bus
        // id. An empty string, as the field documents, rather than a
        // fabricated one.
        bus_id: String::new(),
        driver_version: info.driver_version.clone(),
        // The driver's own figure first — it is what Activity Monitor draws.
        // Unparked residency stands in only when the driver key is absent, and
        // the two are never averaged: they measure different things (work
        // submitted versus time out of the parked state) and blending them
        // would produce a number neither source stands behind.
        utilization_pct: info.utilization_pct.map(clamp_pct).or(residency_pct),
        // No memory-controller utilisation counter is exposed.
        mem_utilization_pct: None,
        // GPU-resident bytes of the unified pool, against the whole pool.
        // Apple Silicon has no VRAM, so the MEM% column is occupancy of system
        // memory by the GPU — see the module doc.
        mem_used_bytes: info.in_use_memory_bytes,
        mem_total_bytes: unified_memory_bytes,
        // Apple exposes no GPU die temperature to an unprivileged process.
        temperature_c: None,
        power_watts,
        // No enforced GPU power cap is published; the SoC budget is shared
        // with the CPU and managed by firmware.
        power_limit_watts: None,
        graphics_clock_mhz,
        // Unified memory runs at the SoC's memory clock, which is not a GPU
        // counter and is not published per-engine.
        memory_clock_mhz: None,
        // Cooling is chassis-wide, not per-GPU; a MacBook Air has no fan.
        fan_pct: None,
        // The media engine is a separate block with no published utilisation
        // counter.
        encoder_pct: None,
        decoder_pct: None,
        supports_process_stats: false,
    }
}

/// Name the row.
///
/// `model` is the SoC ("Apple M3"), not the GPU, so the core count is appended
/// to make the row describe the part it stands for — the same way Apple's own
/// spec sheets name it. Falls back through the driver class before giving up,
/// because a row named after its driver is still identifiable.
fn device_name(info: &AcceleratorInfo) -> String {
    let base = info
        .model
        .clone()
        .or_else(|| info.class.clone())
        .unwrap_or_else(|| "Apple GPU".to_string());
    match info.gpu_core_count {
        Some(cores) if cores > 0 => format!("{base} ({cores}-core GPU)"),
        _ => base,
    }
}

#[async_trait]
impl GpuEngine for AppleEngine {
    async fn snapshot(&self) -> Result<GpusSnapshot, GpuError> {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || inner.collect())
            .await
            .map_err(|e| {
                GpuError::Query(format!(
                    "Apple GPU collection task panicked or was cancelled: {e}"
                ))
            })
    }

    fn backend(&self) -> GpuBackend {
        GpuBackend::AppleIoReport
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(model: Option<&str>, class: Option<&str>, cores: Option<u32>) -> AcceleratorInfo {
        AcceleratorInfo {
            model: model.map(Into::into),
            class: class.map(Into::into),
            gpu_core_count: cores,
            ..AcceleratorInfo::default()
        }
    }

    fn inputs(info: AcceleratorInfo) -> DeviceInputs {
        DeviceInputs {
            index: 0,
            info,
            power_watts: None,
            graphics_clock_mhz: None,
            residency_pct: None,
            unified_memory_bytes: Some(8 * 1024 * 1024 * 1024),
        }
    }

    // ---- naming -----------------------------------------------------------

    #[test]
    fn device_name_carries_the_core_count() {
        let named = device_name(&info(Some("Apple M3"), Some("AGXAcceleratorG15G"), Some(8)));
        assert_eq!(named, "Apple M3 (8-core GPU)");
    }

    #[test]
    fn device_name_falls_back_to_the_driver_class() {
        assert_eq!(
            device_name(&info(None, Some("AGXAcceleratorG16P"), None)),
            "AGXAcceleratorG16P"
        );
    }

    #[test]
    fn device_name_never_ends_up_empty() {
        assert_eq!(device_name(&info(None, None, None)), "Apple GPU");
    }

    #[test]
    fn device_name_omits_a_zero_core_count() {
        // A driver reporting zero cores is reporting nothing useful; "(0-core
        // GPU)" would read as a fact rather than as a missing value.
        assert_eq!(
            device_name(&info(Some("Apple M3"), None, Some(0))),
            "Apple M3"
        );
    }

    // ---- Apple-only detection --------------------------------------------

    #[test]
    fn only_agx_drivers_count_as_apple_gpus() {
        assert!(info(None, Some("AGXAcceleratorG15G"), None).is_apple_gpu());
        // An Intel Mac's discrete card answers the same IOAccelerator match.
        assert!(!info(None, Some("AMDRadeonX6000"), None).is_apple_gpu());
        assert!(!info(None, Some("IntelAccelerator"), None).is_apple_gpu());
        assert!(!info(None, None, None).is_apple_gpu());
    }

    // ---- the honesty contract, per field ---------------------------------

    #[test]
    fn unreportable_metrics_stay_none() {
        // Every one of these is a metric Apple genuinely does not publish to
        // an unprivileged process. The tab renders `None` as `—`; a zero here
        // would claim a cold, unclocked, fanless GPU.
        let device = build_device(inputs(info(
            Some("Apple M3"),
            Some("AGXAcceleratorG15G"),
            Some(8),
        )));
        assert_eq!(device.temperature_c, None);
        assert_eq!(device.power_limit_watts, None);
        assert_eq!(device.memory_clock_mhz, None);
        assert_eq!(device.mem_utilization_pct, None);
        assert_eq!(device.fan_pct, None);
        assert_eq!(device.encoder_pct, None);
        assert_eq!(device.decoder_pct, None);
        assert!(!device.supports_process_stats);
        assert!(device.bus_id.is_empty(), "there is no PCI bus to report");
    }

    #[test]
    fn utilisation_prefers_the_driver_over_residency() {
        let mut acc = info(Some("Apple M3"), Some("AGXAcceleratorG15G"), Some(8));
        acc.utilization_pct = Some(62);
        let mut input = inputs(acc);
        input.residency_pct = Some(24.8);

        let device = build_device(input);
        assert_eq!(device.utilization_pct, Some(62.0));
    }

    #[test]
    fn utilisation_falls_back_to_unparked_residency() {
        let mut input = inputs(info(Some("Apple M3"), Some("AGXAcceleratorG15G"), Some(8)));
        input.residency_pct = Some(24.5);

        let device = build_device(input);
        assert_eq!(device.utilization_pct, Some(24.5));
    }

    #[test]
    fn utilisation_is_clamped_to_the_gauge_range() {
        let mut acc = info(Some("Apple M3"), Some("AGXAcceleratorG15G"), Some(8));
        acc.utilization_pct = Some(120);
        let device = build_device(inputs(acc));
        assert_eq!(device.utilization_pct, Some(100.0));
    }

    #[test]
    fn memory_is_reported_against_the_unified_pool() {
        let mut acc = info(Some("Apple M3"), Some("AGXAcceleratorG15G"), Some(8));
        acc.in_use_memory_bytes = Some(2 * 1024 * 1024 * 1024);
        let device = build_device(inputs(acc));

        assert_eq!(device.mem_total_bytes, Some(8 * 1024 * 1024 * 1024));
        let pct = device.mem_pct().expect("both sides known");
        assert!((pct - 25.0).abs() < 0.01, "expected 25%, got {pct}");
    }

    #[test]
    fn a_device_is_tagged_apple_on_both_axes() {
        let device = build_device(inputs(info(
            Some("Apple M3"),
            Some("AGXAcceleratorG15G"),
            Some(8),
        )));
        assert_eq!(device.vendor, GpuVendor::Apple);
        assert_eq!(device.backend, GpuBackend::AppleIoReport);
    }
}
