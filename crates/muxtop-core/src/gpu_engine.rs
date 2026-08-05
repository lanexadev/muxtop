//! GPU engine abstraction: the [`GpuEngine`] async trait, the
//! [`CompositeGpuEngine`] that fans out to several vendor backends, and the
//! platform detection chain.
//!
//! This module defines the interface and the composition; the concrete
//! backends live in `nvml_engine.rs` (NVIDIA) and `amd_engine.rs` (AMD).
//!
//! # Why a composite rather than one engine per tab
//!
//! Unlike Docker or Kubernetes, a machine routinely has GPUs from more than
//! one vendor at once — an AMD iGPU plus an NVIDIA dGPU is the standard
//! gaming-laptop layout, and a workstation can hold both. The Containers and
//! Kube tabs each talk to exactly one daemon; the GPU tab has to merge.
//! [`CompositeGpuEngine`] owns that merge, so the collector, the wire format
//! and the TUI all see a single flat [`GpusSnapshot`].
//!
//! # Refresh rate contract
//!
//! The collector invokes [`GpuEngine::snapshot`] at **1 Hz** — faster than
//! the container (0.5 Hz) and cluster (0.2 Hz) loops. This is deliberate:
//! NVML computes `utilization_rates` over an internal sampling window of
//! roughly one second, so polling slower than 1 Hz aliases the signal and
//! makes a GPU that is pinned at 100 % render as an erratic sawtooth. The
//! cost is bounded — an NVML device query is sub-millisecond and the AMD
//! backend reads a handful of small sysfs files.
//!
//! # Detection chain
//!
//! [`detect_gpu_engines`] tries, per platform:
//! * **Linux** — NVML, then AMD sysfs. Both may succeed.
//! * **Windows** — NVML only (`amdgpu` sysfs is a Linux interface).
//! * **macOS** — nothing yet; see [`APPLE_DEFERRED_DETAIL`].
//!
//! Detection never fails: when no backend initialises it returns a
//! [`NullGpuEngine`] carrying the reason, so the UI can explain *why* the tab
//! is empty instead of showing a bare placeholder.

use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;

use crate::gpu::{GpuBackend, GpusSnapshot};

/// Message shown on macOS until the Apple Silicon backend lands.
///
/// Apple exposes no public API for GPU counters: the figures `powermetrics`
/// prints come from the private `IOReport` framework, and the tool itself
/// requires root. Shipping that in v0.5 would have meant either a private
/// framework dependency or asking every macOS user to run muxtop as root —
/// neither fits a tool whose entire premise is that it stays out of the way.
/// It is scheduled for **v0.6**, not abandoned.
pub const APPLE_DEFERRED_DETAIL: &str = "Apple Silicon GPU monitoring is not implemented yet — planned for v0.6 \
     (macOS exposes GPU counters only through the private IOReport framework, \
     which needs root)";

/// Errors raised by a [`GpuEngine`] implementation.
///
/// Granularity mirrors `ClusterError` / `EngineError`: enough structure for
/// the UI to say something useful, no more.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum GpuError {
    /// No vendor backend could be initialised on this host.
    #[error("no GPU backend available")]
    NoBackend,

    /// The vendor library or kernel driver is absent or refused to load.
    ///
    /// The overwhelmingly common case: a machine with no NVIDIA card has no
    /// `libnvidia-ml`, and that is not an error worth shouting about.
    #[error("{vendor} driver unavailable: {reason}")]
    DriverUnavailable {
        vendor: &'static str,
        reason: String,
    },

    /// The backend initialised but a specific query failed.
    #[error("GPU query failed: {0}")]
    Query(String),

    /// Reading a sysfs node failed for a reason other than absence.
    #[error("GPU sysfs read failed at {path}: {reason}")]
    Sysfs { path: String, reason: String },

    /// This platform has no implemented backend.
    #[error("{0}")]
    Unsupported(&'static str),
}

/// Abstraction over a source of GPU telemetry (NVML, AMD sysfs, …).
///
/// Implementations MUST be safe to share across tokio tasks
/// (`Send + Sync + 'static`) and MUST NOT panic on missing metrics — an
/// unavailable figure is `None`, never a default.
///
/// `#[async_trait]` keeps the trait object-safe, matching the v0.3 `ADR-01`
/// decision: the collector holds `Option<Arc<dyn GpuEngine + Send + Sync>>`.
#[async_trait]
pub trait GpuEngine: Send + Sync {
    /// Build a fresh snapshot of every device this backend can see.
    ///
    /// Blocking vendor calls (NVML is a synchronous C library, sysfs is
    /// synchronous file I/O) MUST be moved off the async runtime — the
    /// backends use `tokio::task::spawn_blocking`.
    async fn snapshot(&self) -> Result<GpusSnapshot, GpuError>;

    /// Which backend this engine represents. `CompositeGpuEngine` reports
    /// the first of its children.
    fn backend(&self) -> GpuBackend;
}

/// An engine that sees nothing, and knows why.
///
/// Returned by [`detect_gpu_engines`] when no vendor backend initialises, so
/// the "no GPU" state carries an explanation ("no NVIDIA driver loaded",
/// "Apple Silicon lands in v0.6") instead of a bare placeholder. Keeping an
/// engine present rather than returning `None` also means the collector needs
/// no special case.
pub struct NullGpuEngine {
    detail: String,
}

impl NullGpuEngine {
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

#[async_trait]
impl GpuEngine for NullGpuEngine {
    async fn snapshot(&self) -> Result<GpusSnapshot, GpuError> {
        Ok(GpusSnapshot::unavailable_with(self.detail.clone()))
    }

    fn backend(&self) -> GpuBackend {
        // Arbitrary but harmless: a Null engine never contributes devices, so
        // no snapshot ever attributes anything to this backend.
        GpuBackend::Nvml
    }
}

/// Fans out to several vendor backends and merges their snapshots into one.
///
/// # Merge semantics
///
/// * Device indices are **reassigned** sequentially across the merged list.
///   Each backend numbers its own devices from 0, so an NVIDIA GPU and an AMD
///   GPU would both claim index 0; the composite renumbers them 0 and 1.
/// * `GpuProcessSnapshot::device_index` is remapped through the same shift,
///   so a process keeps pointing at its own device after renumbering. Getting
///   this wrong would silently attribute one card's processes to another.
/// * A backend that errors is **skipped, not fatal** — a broken NVIDIA driver
///   must not hide a working AMD card. The failure is logged and, when every
///   backend fails, surfaced through `detail`.
pub struct CompositeGpuEngine {
    backends: Vec<Arc<dyn GpuEngine + Send + Sync>>,
}

impl CompositeGpuEngine {
    /// Build a composite over one or more backends.
    pub fn new(backends: Vec<Arc<dyn GpuEngine + Send + Sync>>) -> Self {
        Self { backends }
    }

    /// Number of backends composed.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }
}

#[async_trait]
impl GpuEngine for CompositeGpuEngine {
    async fn snapshot(&self) -> Result<GpusSnapshot, GpuError> {
        let mut merged = GpusSnapshot::unavailable();
        let mut failures: Vec<String> = Vec::new();

        for backend in &self.backends {
            let snapshot = match backend.snapshot().await {
                Ok(s) => s,
                Err(err) => {
                    // One vendor failing must not blind the others.
                    tracing::debug!(
                        target: "muxtop::gpu",
                        backend = backend.backend().label(),
                        error = %err,
                        "GPU backend failed; continuing with the remaining backends"
                    );
                    failures.push(err.to_string());
                    continue;
                }
            };

            if !snapshot.available {
                if !snapshot.detail.is_empty() {
                    failures.push(snapshot.detail.clone());
                }
                continue;
            }

            // Devices from this backend land at `offset..offset + n`.
            let offset = merged.devices.len() as u32;
            for mut device in snapshot.devices {
                let local_index = device.index;
                device.index = offset + local_index;
                merged.devices.push(device);
            }
            for mut process in snapshot.processes {
                process.device_index += offset;
                merged.processes.push(process);
            }
            for backend_kind in snapshot.backends {
                if !merged.backends.contains(&backend_kind) {
                    merged.backends.push(backend_kind);
                }
            }
        }

        merged.available = !merged.devices.is_empty();
        if !merged.available {
            merged.detail = if failures.is_empty() {
                "no GPU detected".to_string()
            } else {
                failures.join("; ")
            };
        }

        Ok(merged)
    }

    fn backend(&self) -> GpuBackend {
        self.backends
            .first()
            .map(|b| b.backend())
            .unwrap_or(GpuBackend::Nvml)
    }
}

/// Probe every backend this platform supports and return a single engine.
///
/// Never returns `None`: when nothing initialises, the result is a
/// [`NullGpuEngine`] carrying the reason. Callers that want the tab disabled
/// entirely (`--no-gpu`) pass `None` to the collector instead of calling this.
pub fn detect_gpu_engines() -> Arc<dyn GpuEngine + Send + Sync> {
    // `mut` is genuinely unused on platforms where no backend is compiled in
    // (macOS today — see `APPLE_DEFERRED_DETAIL`). Darwin is in the release
    // matrix, so it has to compile warning-free.
    let mut backends: Vec<Arc<dyn GpuEngine + Send + Sync>> = Vec::new();
    let mut reasons: Vec<String> = Vec::new();

    // Both bindings are passed by `&mut` unconditionally, which is what keeps
    // this warning-free on a target where every backend is `cfg`-ed out (macOS
    // today). Doing the probing inline would leave `backends` never mutated
    // there and trip `unused_mut` — a warning that only appears on a platform
    // this machine cannot compile for, which is exactly how the v0.4.2 Windows
    // break survived two releases.
    probe_platform_backends(&mut backends, &mut reasons);

    if backends.is_empty() {
        let detail = if reasons.is_empty() {
            "no supported GPU backend on this platform".to_string()
        } else {
            reasons.join("; ")
        };
        return Arc::new(NullGpuEngine::new(detail));
    }

    Arc::new(CompositeGpuEngine::new(backends))
}

/// Try every backend compiled in for this target, appending the ones that
/// initialise to `backends` and the reasons the others did not to `reasons`.
fn probe_platform_backends(
    backends: &mut Vec<Arc<dyn GpuEngine + Send + Sync>>,
    reasons: &mut Vec<String>,
) {
    // Unconditional, and deliberately not `cfg`-ed: on macOS every block below
    // that touches `backends` is compiled out, which would make it an unused
    // parameter. Guarding this with a `cfg` would mean getting the platform
    // list exactly right for a warning no machine here can reproduce.
    let _ = (&backends, &reasons);

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        match crate::nvml_engine::NvmlEngine::connect() {
            Ok(engine) => {
                tracing::info!(target: "muxtop::gpu", "NVML backend initialised");
                backends.push(Arc::new(engine));
            }
            Err(err) => {
                tracing::debug!(target: "muxtop::gpu", error = %err, "NVML backend unavailable");
                reasons.push(err.to_string());
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        match crate::amd_engine::AmdEngine::connect() {
            Ok(engine) => {
                tracing::info!(target: "muxtop::gpu", "AMD sysfs backend initialised");
                backends.push(Arc::new(engine));
            }
            Err(err) => {
                tracing::debug!(target: "muxtop::gpu", error = %err, "AMD backend unavailable");
                reasons.push(err.to_string());
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        reasons.push(APPLE_DEFERRED_DETAIL.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::{GpuDeviceSnapshot, GpuProcessKind, GpuProcessSnapshot, GpuVendor};

    // ---- test doubles -----------------------------------------------------

    /// Backend returning a fixed snapshot, or an error in `fail_mode`.
    struct FakeBackend {
        kind: GpuBackend,
        result: Result<GpusSnapshot, GpuError>,
    }

    #[async_trait]
    impl GpuEngine for FakeBackend {
        async fn snapshot(&self) -> Result<GpusSnapshot, GpuError> {
            self.result.clone()
        }
        fn backend(&self) -> GpuBackend {
            self.kind
        }
    }

    fn device(index: u32, vendor: GpuVendor, backend: GpuBackend, name: &str) -> GpuDeviceSnapshot {
        GpuDeviceSnapshot {
            index,
            vendor,
            backend,
            name: name.into(),
            bus_id: format!("0000:0{index}:00.0"),
            driver_version: None,
            utilization_pct: Some(10.0),
            mem_utilization_pct: None,
            mem_used_bytes: Some(1024),
            mem_total_bytes: Some(4096),
            temperature_c: Some(50.0),
            power_watts: None,
            power_limit_watts: None,
            graphics_clock_mhz: None,
            memory_clock_mhz: None,
            fan_pct: None,
            encoder_pct: None,
            decoder_pct: None,
            supports_process_stats: matches!(backend, GpuBackend::Nvml),
        }
    }

    fn process(pid: u32, device_index: u32) -> GpuProcessSnapshot {
        GpuProcessSnapshot {
            pid,
            device_index,
            name: format!("proc{pid}"),
            kind: GpuProcessKind::Compute,
            mem_bytes: Some(512),
        }
    }

    fn snapshot_with(
        backend: GpuBackend,
        devices: Vec<GpuDeviceSnapshot>,
        processes: Vec<GpuProcessSnapshot>,
    ) -> GpusSnapshot {
        GpusSnapshot {
            backends: vec![backend],
            available: !devices.is_empty(),
            devices,
            processes,
            detail: String::new(),
        }
    }

    fn ok_backend(
        kind: GpuBackend,
        devices: Vec<GpuDeviceSnapshot>,
        processes: Vec<GpuProcessSnapshot>,
    ) -> Arc<dyn GpuEngine + Send + Sync> {
        Arc::new(FakeBackend {
            kind,
            result: Ok(snapshot_with(kind, devices, processes)),
        })
    }

    fn err_backend(kind: GpuBackend, err: GpuError) -> Arc<dyn GpuEngine + Send + Sync> {
        Arc::new(FakeBackend {
            kind,
            result: Err(err),
        })
    }

    // ---- trait shape ------------------------------------------------------

    #[test]
    fn gpu_engine_is_object_safe() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn GpuEngine>();
        let _boxed: Box<dyn GpuEngine + Send + Sync> = Box::new(NullGpuEngine::new("x"));
    }

    #[test]
    fn gpu_error_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<GpuError>();
    }

    #[test]
    fn gpu_error_display_is_informative() {
        let variants = vec![
            GpuError::NoBackend,
            GpuError::DriverUnavailable {
                vendor: "NVIDIA",
                reason: "libnvidia-ml.so not found".into(),
            },
            GpuError::Query("nvmlDeviceGetCount failed".into()),
            GpuError::Sysfs {
                path: "/sys/class/drm/card0/device/gpu_busy_percent".into(),
                reason: "permission denied".into(),
            },
            GpuError::Unsupported(APPLE_DEFERRED_DETAIL),
        ];
        for err in &variants {
            assert!(!format!("{err}").is_empty(), "empty Display for {err:?}");
        }
        assert!(format!("{}", variants[1]).contains("libnvidia-ml.so"));
        assert!(format!("{}", variants[1]).contains("NVIDIA"));
        assert!(format!("{}", variants[3]).contains("gpu_busy_percent"));
        assert!(format!("{}", variants[4]).contains("v0.6"));
    }

    // ---- NullGpuEngine ----------------------------------------------------

    #[tokio::test]
    async fn null_engine_reports_its_reason() {
        let engine = NullGpuEngine::new("no NVIDIA driver loaded");
        let snap = engine.snapshot().await.expect("null engine never errors");
        assert!(!snap.available);
        assert!(snap.devices.is_empty());
        assert_eq!(snap.detail, "no NVIDIA driver loaded");
    }

    // ---- CompositeGpuEngine: the merge contract ---------------------------

    #[tokio::test]
    async fn composite_reindexes_devices_across_backends() {
        // The core invariant: two backends each numbering from 0 must not
        // collide after the merge.
        let nvidia = ok_backend(
            GpuBackend::Nvml,
            vec![
                device(0, GpuVendor::Nvidia, GpuBackend::Nvml, "RTX 4090"),
                device(1, GpuVendor::Nvidia, GpuBackend::Nvml, "RTX 3060"),
            ],
            vec![],
        );
        let amd = ok_backend(
            GpuBackend::AmdSysfs,
            vec![device(
                0,
                GpuVendor::Amd,
                GpuBackend::AmdSysfs,
                "Radeon 780M",
            )],
            vec![],
        );

        let composite = CompositeGpuEngine::new(vec![nvidia, amd]);
        let snap = composite.snapshot().await.unwrap();

        assert!(snap.available);
        assert_eq!(snap.devices.len(), 3);
        let indices: Vec<u32> = snap.devices.iter().map(|d| d.index).collect();
        assert_eq!(indices, vec![0, 1, 2], "indices must be dense and unique");
        assert_eq!(snap.devices[2].name, "Radeon 780M");
        assert_eq!(snap.devices[2].vendor, GpuVendor::Amd);
    }

    #[tokio::test]
    async fn composite_remaps_process_device_index() {
        // A process on the second backend's device 0 must end up pointing at
        // the merged index, not at the first backend's device 0. Getting this
        // wrong attributes one card's processes to another card.
        let nvidia = ok_backend(
            GpuBackend::Nvml,
            vec![device(0, GpuVendor::Nvidia, GpuBackend::Nvml, "RTX 4090")],
            vec![process(100, 0)],
        );
        let second = ok_backend(
            GpuBackend::Nvml,
            vec![device(0, GpuVendor::Nvidia, GpuBackend::Nvml, "RTX 3060")],
            vec![process(200, 0)],
        );

        let composite = CompositeGpuEngine::new(vec![nvidia, second]);
        let snap = composite.snapshot().await.unwrap();

        assert_eq!(snap.processes.len(), 2);
        let p100 = snap.processes.iter().find(|p| p.pid == 100).unwrap();
        let p200 = snap.processes.iter().find(|p| p.pid == 200).unwrap();
        assert_eq!(p100.device_index, 0);
        assert_eq!(p200.device_index, 1, "second backend's device 0 becomes 1");

        // And every process points at a device that actually exists.
        for p in &snap.processes {
            assert!(
                snap.devices.iter().any(|d| d.index == p.device_index),
                "process {} points at missing device {}",
                p.pid,
                p.device_index
            );
        }
    }

    #[tokio::test]
    async fn composite_survives_one_backend_failing() {
        // A broken NVIDIA driver must not hide a working AMD card.
        let nvidia = err_backend(
            GpuBackend::Nvml,
            GpuError::DriverUnavailable {
                vendor: "NVIDIA",
                reason: "driver not loaded".into(),
            },
        );
        let amd = ok_backend(
            GpuBackend::AmdSysfs,
            vec![device(
                0,
                GpuVendor::Amd,
                GpuBackend::AmdSysfs,
                "Radeon 780M",
            )],
            vec![],
        );

        let composite = CompositeGpuEngine::new(vec![nvidia, amd]);
        let snap = composite.snapshot().await.unwrap();

        assert!(snap.available, "the working backend must still be reported");
        assert_eq!(snap.devices.len(), 1);
        assert_eq!(snap.devices[0].vendor, GpuVendor::Amd);
        assert_eq!(snap.devices[0].index, 0, "indices stay dense from zero");
    }

    #[tokio::test]
    async fn composite_reports_every_reason_when_all_fail() {
        let nvidia = err_backend(
            GpuBackend::Nvml,
            GpuError::DriverUnavailable {
                vendor: "NVIDIA",
                reason: "driver not loaded".into(),
            },
        );
        let amd = err_backend(
            GpuBackend::AmdSysfs,
            GpuError::DriverUnavailable {
                vendor: "AMD",
                reason: "no amdgpu card in /sys/class/drm".into(),
            },
        );

        let composite = CompositeGpuEngine::new(vec![nvidia, amd]);
        let snap = composite.snapshot().await.unwrap();

        assert!(!snap.available);
        assert!(snap.devices.is_empty());
        assert!(snap.detail.contains("NVIDIA"), "detail: {}", snap.detail);
        assert!(snap.detail.contains("AMD"), "detail: {}", snap.detail);
    }

    #[tokio::test]
    async fn composite_collects_distinct_backend_kinds() {
        let nvidia = ok_backend(
            GpuBackend::Nvml,
            vec![device(0, GpuVendor::Nvidia, GpuBackend::Nvml, "RTX 4090")],
            vec![],
        );
        let amd = ok_backend(
            GpuBackend::AmdSysfs,
            vec![device(
                0,
                GpuVendor::Amd,
                GpuBackend::AmdSysfs,
                "Radeon 780M",
            )],
            vec![],
        );

        let composite = CompositeGpuEngine::new(vec![nvidia, amd]);
        let snap = composite.snapshot().await.unwrap();

        assert_eq!(snap.backends.len(), 2);
        assert!(snap.backends.contains(&GpuBackend::Nvml));
        assert!(snap.backends.contains(&GpuBackend::AmdSysfs));
    }

    #[tokio::test]
    async fn composite_dedupes_backend_kinds() {
        // Two NVML engines (unusual but legal) must not report `nvml` twice
        // in the summary bar.
        let a = ok_backend(
            GpuBackend::Nvml,
            vec![device(0, GpuVendor::Nvidia, GpuBackend::Nvml, "RTX 4090")],
            vec![],
        );
        let b = ok_backend(
            GpuBackend::Nvml,
            vec![device(0, GpuVendor::Nvidia, GpuBackend::Nvml, "RTX 3060")],
            vec![],
        );

        let composite = CompositeGpuEngine::new(vec![a, b]);
        let snap = composite.snapshot().await.unwrap();
        assert_eq!(snap.backends, vec![GpuBackend::Nvml]);
    }

    #[tokio::test]
    async fn composite_with_no_backends_is_unavailable() {
        let composite = CompositeGpuEngine::new(vec![]);
        assert!(composite.is_empty());
        assert_eq!(composite.len(), 0);

        let snap = composite.snapshot().await.unwrap();
        assert!(!snap.available);
        assert_eq!(snap.detail, "no GPU detected");
    }

    #[tokio::test]
    async fn composite_ignores_unavailable_child_snapshots() {
        // A backend that returns Ok(unavailable) — the Null case — must not
        // count as a device source, but its detail should still surface.
        let null: Arc<dyn GpuEngine + Send + Sync> =
            Arc::new(NullGpuEngine::new("no NVIDIA driver loaded"));
        let composite = CompositeGpuEngine::new(vec![null]);
        let snap = composite.snapshot().await.unwrap();

        assert!(!snap.available);
        assert_eq!(snap.detail, "no NVIDIA driver loaded");
    }

    // ---- detection --------------------------------------------------------

    #[tokio::test]
    async fn detect_gpu_engines_never_panics_and_always_answers() {
        // Result depends entirely on the host, so we pin only the contract:
        // detection returns a usable engine and snapshotting it succeeds.
        let engine = detect_gpu_engines();
        let snap = engine
            .snapshot()
            .await
            .expect("detection engine must not error");
        // Either it found devices, or it explained why it did not.
        if !snap.available {
            assert!(
                !snap.detail.is_empty(),
                "an unavailable GPU snapshot must carry a reason"
            );
        }
    }

    #[test]
    fn apple_detail_names_the_target_version() {
        // The roadmap promises v0.6; the user-visible string must agree with
        // it, otherwise the deferral reads as an abandonment.
        assert!(APPLE_DEFERRED_DETAIL.contains("v0.6"));
        assert!(APPLE_DEFERRED_DETAIL.contains("IOReport"));
    }
}
