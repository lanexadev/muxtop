//! AMD backend for the GPU tab, via the `amdgpu` driver's sysfs interface.
//!
//! # Why sysfs and not a vendor library
//!
//! AMD's equivalent of NVML is ROCm-SMI, which ships as part of ROCm — a
//! multi-gigabyte stack that most machines with an AMD GPU do not have
//! installed, and which does not cover consumer cards well. The `amdgpu`
//! kernel driver, by contrast, is present on every Linux machine that can
//! drive an AMD GPU at all, and exposes the same counters as plain files
//! under `/sys/class/drm/card*/device`. Reading them costs no dependency, no
//! privileges beyond world-readable sysfs, and no process spawn.
//!
//! The trade-off is coverage: sysfs has **no per-process accounting**. There
//! is no AMD equivalent of `nvmlDeviceGetComputeRunningProcesses`, so devices
//! from this backend report `supports_process_stats = false` and the Procs
//! sub-view says so explicitly rather than rendering a misleading empty list.
//!
//! # Platform
//!
//! The parsing is plain file I/O over a root directory, so this module
//! compiles and its tests run on every target. Only the *detection* entry
//! point ([`AmdEngine::connect`], which hard-codes `/sys/class/drm`) is
//! meaningful on Linux, and `gpu_engine::detect_gpu_engines` only wires it in
//! there. Keeping the module itself un-gated means the fixture tests below
//! guard the parser on Windows and macOS CI too — the v0.4.2 Windows break
//! was a reminder of how quickly un-exercised platform code rots.
//!
//! # Units in sysfs
//!
//! sysfs speaks in the kernel's units, not the user's: temperatures are
//! millidegrees, power is microwatts, hwmon frequencies are hertz, and fan
//! `pwm1` is a 0–255 duty cycle. Every conversion happens here so no other
//! layer has to know.

use std::fs;
use std::path::{Path, PathBuf};

use async_trait::async_trait;

use crate::gpu::{GpuBackend, GpuDeviceSnapshot, GpuVendor, GpusSnapshot};
use crate::gpu_engine::{GpuEngine, GpuError};

/// Canonical DRM class directory on Linux.
const DRM_ROOT: &str = "/sys/class/drm";

/// PCI vendor id for AMD/ATI. `cardN` entries exist for every GPU regardless
/// of vendor, so this is what separates an AMD card from the Intel iGPU
/// sitting next to it.
const PCI_VENDOR_AMD: &str = "0x1002";

/// AMD telemetry source backed by the `amdgpu` sysfs interface.
#[derive(Debug)]
pub struct AmdEngine {
    /// Per-card `device/` directories, discovered once at connect time and
    /// held in stable index order.
    ///
    /// Cards are not hot-plugged in practice, and re-scanning the directory
    /// on every tick would both cost I/O and let device indices shuffle under
    /// the user's selection.
    cards: Vec<PathBuf>,
    driver_version: Option<String>,
}

impl AmdEngine {
    /// Discover AMD cards under the real `/sys/class/drm`.
    pub fn connect() -> Result<Self, GpuError> {
        Self::connect_at(Path::new(DRM_ROOT))
    }

    /// [`Self::connect`] against an arbitrary DRM root — the injectable form
    /// used by the fixture tests.
    pub fn connect_at(drm_root: &Path) -> Result<Self, GpuError> {
        let cards = discover_cards(drm_root);
        if cards.is_empty() {
            return Err(GpuError::DriverUnavailable {
                vendor: "AMD",
                reason: format!("no amdgpu card found under {}", drm_root.display()),
            });
        }

        // `/sys/module/amdgpu/version` is frequently absent (it only appears
        // when the module was built with a version string), so `None` here is
        // unremarkable rather than an error.
        let driver_version = read_trimmed(&PathBuf::from("/sys/module/amdgpu/version"));

        Ok(Self {
            cards,
            driver_version,
        })
    }

    /// Number of AMD cards this engine will report.
    pub fn device_count(&self) -> usize {
        self.cards.len()
    }
}

#[async_trait]
impl GpuEngine for AmdEngine {
    async fn snapshot(&self) -> Result<GpusSnapshot, GpuError> {
        // sysfs reads are blocking file I/O. They are fast (a few dozen small
        // reads from a virtual filesystem), but "fast" is not "non-blocking",
        // and the runtime worker also drives the container and cluster loops.
        let cards = self.cards.clone();
        let driver_version = self.driver_version.clone();

        tokio::task::spawn_blocking(move || collect(&cards, driver_version))
            .await
            .map_err(|e| {
                GpuError::Query(format!(
                    "AMD sysfs collection task panicked or was cancelled: {e}"
                ))
            })
    }

    fn backend(&self) -> GpuBackend {
        GpuBackend::AmdSysfs
    }
}

/// Find every `cardN/device` directory whose PCI vendor is AMD.
///
/// Filters out two things that also live in `/sys/class/drm`:
/// * connector entries (`card0-DP-1`, `card0-HDMI-A-1`) — same prefix, not a
///   device;
/// * cards belonging to other vendors — an Intel iGPU is `card0` just as
///   readily as an AMD one, and claiming it here would report an Intel
///   device with AMD's (absent) counters.
///
/// Results are sorted by card number so device indices are stable across
/// ticks; `read_dir` order is filesystem-dependent and must not leak into the
/// UI as rows that jump around.
fn discover_cards(drm_root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(drm_root) else {
        return Vec::new();
    };

    let mut found: Vec<(u32, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(card_number) = parse_card_number(name) else {
            continue;
        };

        let device_dir = entry.path().join("device");
        if !is_amd_card(&device_dir) {
            continue;
        }
        found.push((card_number, device_dir));
    }

    found.sort_by_key(|(n, _)| *n);
    found.into_iter().map(|(_, path)| path).collect()
}

/// `card0` → `Some(0)`; `card0-DP-1`, `renderD128`, `version` → `None`.
///
/// The strict "digits to end of string" rule is what rejects connector
/// directories, which share the `card` prefix and would otherwise be probed
/// as devices.
fn parse_card_number(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("card")?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// True when `device/vendor` reads as AMD's PCI vendor id.
fn is_amd_card(device_dir: &Path) -> bool {
    read_trimmed(&device_dir.join("vendor"))
        .map(|v| v.eq_ignore_ascii_case(PCI_VENDOR_AMD))
        .unwrap_or(false)
}

/// Blocking collection body, free-standing so it can run in `spawn_blocking`.
fn collect(cards: &[PathBuf], driver_version: Option<String>) -> GpusSnapshot {
    let mut devices = Vec::with_capacity(cards.len());

    for (index, device_dir) in cards.iter().enumerate() {
        devices.push(read_device(
            device_dir,
            index as u32,
            driver_version.clone(),
        ));
    }

    if devices.is_empty() {
        return GpusSnapshot::unavailable_with("no amdgpu card readable");
    }

    GpusSnapshot {
        backends: vec![GpuBackend::AmdSysfs],
        available: true,
        devices,
        // sysfs exposes no per-process GPU accounting — see the module doc.
        processes: Vec::new(),
        detail: String::new(),
    }
}

/// Read one card's counters. Every field is best-effort: a card that is
/// powered down, or a kernel too old to expose an attribute, yields `None`
/// for that field and a complete row for the rest.
fn read_device(device_dir: &Path, index: u32, driver_version: Option<String>) -> GpuDeviceSnapshot {
    let hwmon = find_hwmon(device_dir);

    // hwmon frequencies are in Hz; the `pp_dpm_*` tables are the fallback and
    // are already in MHz.
    let graphics_clock_mhz = hwmon
        .as_ref()
        .and_then(|h| read_u64(&h.join("freq1_input")))
        .map(hz_to_mhz)
        .or_else(|| read_active_dpm_mhz(&device_dir.join("pp_dpm_sclk")));
    let memory_clock_mhz = hwmon
        .as_ref()
        .and_then(|h| read_u64(&h.join("freq2_input")))
        .map(hz_to_mhz)
        .or_else(|| read_active_dpm_mhz(&device_dir.join("pp_dpm_mclk")));

    GpuDeviceSnapshot {
        index,
        vendor: GpuVendor::Amd,
        backend: GpuBackend::AmdSysfs,
        name: read_device_name(device_dir),
        bus_id: read_bus_id(device_dir),
        driver_version,
        utilization_pct: read_u64(&device_dir.join("gpu_busy_percent")).map(|v| v as f32),
        mem_utilization_pct: read_u64(&device_dir.join("mem_busy_percent")).map(|v| v as f32),
        mem_used_bytes: read_u64(&device_dir.join("mem_info_vram_used")),
        mem_total_bytes: read_u64(&device_dir.join("mem_info_vram_total")),
        // millidegrees Celsius
        temperature_c: hwmon
            .as_ref()
            .and_then(|h| read_u64(&h.join("temp1_input")))
            .map(|v| v as f32 / 1000.0),
        // microwatts. `power1_average` is the smoothed figure the driver
        // prefers; `power1_input` is the instantaneous one some cards expose
        // instead.
        power_watts: hwmon
            .as_ref()
            .and_then(|h| {
                read_u64(&h.join("power1_average")).or_else(|| read_u64(&h.join("power1_input")))
            })
            .map(uw_to_watts),
        power_limit_watts: hwmon
            .as_ref()
            .and_then(|h| read_u64(&h.join("power1_cap")))
            .map(uw_to_watts),
        graphics_clock_mhz,
        memory_clock_mhz,
        fan_pct: hwmon.as_ref().and_then(|h| read_fan_pct(h)),
        // No hardware encoder/decoder counters in the amdgpu sysfs interface.
        encoder_pct: None,
        decoder_pct: None,
        supports_process_stats: false,
    }
}

/// The card's marketing name, or an honest fallback.
///
/// `product_name` is exposed by recent `amdgpu` builds and is the only place
/// sysfs carries a human name. When it is missing there is no name to be had
/// — the PCI device id is not a lookup table we ship — so we render the id
/// itself rather than inventing a label.
fn read_device_name(device_dir: &Path) -> String {
    if let Some(name) = read_trimmed(&device_dir.join("product_name"))
        && !name.is_empty()
    {
        return name;
    }
    match read_trimmed(&device_dir.join("device")) {
        Some(id) if !id.is_empty() => format!("AMD GPU {id}"),
        _ => "AMD GPU".to_string(),
    }
}

/// PCI slot name (`0000:03:00.0`), pulled from `uevent`.
///
/// The directory name itself is the bus id on a real sysfs, but `uevent` is
/// explicit and survives the symlinked layout that fixtures use.
fn read_bus_id(device_dir: &Path) -> String {
    let Some(uevent) = read_trimmed(&device_dir.join("uevent")) else {
        return String::new();
    };
    uevent
        .lines()
        .find_map(|line| line.strip_prefix("PCI_SLOT_NAME="))
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// First `device/hwmon/hwmon*` directory, which is where temperature, power
/// and fan counters live. A card exposes exactly one in practice.
fn find_hwmon(device_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(device_dir.join("hwmon")).ok()?;
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("hwmon"))
        })
        .collect();
    // Deterministic pick when a card somehow exposes several.
    found.sort();
    found.into_iter().next()
}

/// Fan duty cycle as a percentage, from the PWM value.
///
/// `pwm1` is a raw 0–`pwm1_max` duty cycle (255 on every device seen in the
/// wild, but the attribute exists so we honour it). Cards with no fan expose
/// neither file and yield `None`, which the UI renders as `—`.
fn read_fan_pct(hwmon: &Path) -> Option<f32> {
    let pwm = read_u64(&hwmon.join("pwm1"))?;
    let max = read_u64(&hwmon.join("pwm1_max"))
        .filter(|m| *m > 0)
        .unwrap_or(255);
    Some((pwm as f32 / max as f32 * 100.0).clamp(0.0, 100.0))
}

/// Parse the active entry out of a `pp_dpm_sclk` / `pp_dpm_mclk` table.
///
/// The format is one state per line, with the active one flagged by a
/// trailing `*`:
///
/// ```text
/// 0: 500Mhz
/// 1: 1200Mhz *
/// 2: 2200Mhz
/// ```
///
/// Returns `None` when no line is starred — a card mid-transition briefly
/// reports no active state, and guessing the first line would show a idle
/// clock as if it were current.
fn read_active_dpm_mhz(path: &Path) -> Option<u32> {
    let content = read_trimmed(path)?;
    for line in content.lines() {
        let line = line.trim();
        if !line.ends_with('*') {
            continue;
        }
        // "1: 1200Mhz *" → take the token carrying the unit.
        let value = line
            .split(':')
            .nth(1)?
            .trim()
            .trim_end_matches('*')
            .trim()
            .to_ascii_lowercase();
        let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            continue;
        }
        return digits.parse().ok();
    }
    None
}

fn hz_to_mhz(hz: u64) -> u32 {
    (hz / 1_000_000) as u32
}

fn uw_to_watts(uw: u64) -> f32 {
    uw as f32 / 1_000_000.0
}

/// Read a sysfs file, trimming the trailing newline the kernel always adds.
///
/// A missing file is `None`, not an error: the whole point of this backend is
/// that attribute availability varies by card and kernel version.
fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_u64(path: &Path) -> Option<u64> {
    read_trimmed(path)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::io::Write;
    use tempfile::{TempDir, tempdir};

    /// Build a fake `/sys/class/drm` tree. Returns the root; the card's
    /// `device/` directory is at `<root>/card<n>/device`.
    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = File::create(path).unwrap();
        writeln!(f, "{contents}").unwrap();
    }

    /// A fully-populated AMD card, as a recent kernel would expose it.
    fn fixture_full_card() -> (TempDir, PathBuf) {
        let root = tempdir().unwrap();
        let device = root.path().join("card0").join("device");

        write_file(&device.join("vendor"), "0x1002");
        write_file(&device.join("device"), "0x744c");
        write_file(&device.join("product_name"), "AMD Radeon RX 7900 XTX");
        write_file(
            &device.join("uevent"),
            "DRIVER=amdgpu\nPCI_CLASS=30000\nPCI_ID=1002:744C\nPCI_SLOT_NAME=0000:03:00.0",
        );
        write_file(&device.join("gpu_busy_percent"), "73");
        write_file(&device.join("mem_busy_percent"), "41");
        write_file(&device.join("mem_info_vram_used"), "8589934592");
        write_file(&device.join("mem_info_vram_total"), "25757220864");

        let hwmon = device.join("hwmon").join("hwmon3");
        write_file(&hwmon.join("temp1_input"), "67000"); // 67.0 °C
        write_file(&hwmon.join("power1_average"), "289000000"); // 289 W
        write_file(&hwmon.join("power1_cap"), "355000000"); // 355 W
        write_file(&hwmon.join("pwm1"), "128");
        write_file(&hwmon.join("pwm1_max"), "255");
        write_file(&hwmon.join("freq1_input"), "2394000000"); // 2394 MHz
        write_file(&hwmon.join("freq2_input"), "1249000000"); // 1249 MHz

        (root, device)
    }

    // ---- discovery --------------------------------------------------------

    #[test]
    fn parse_card_number_accepts_only_real_cards() {
        assert_eq!(parse_card_number("card0"), Some(0));
        assert_eq!(parse_card_number("card12"), Some(12));
        // Connector directories share the prefix and must be rejected —
        // probing them would read a non-existent device dir on every tick.
        assert_eq!(parse_card_number("card0-DP-1"), None);
        assert_eq!(parse_card_number("card1-HDMI-A-1"), None);
        assert_eq!(parse_card_number("card"), None);
        assert_eq!(parse_card_number("renderD128"), None);
        assert_eq!(parse_card_number("version"), None);
    }

    #[test]
    fn discover_skips_connectors_and_foreign_vendors() {
        let root = tempdir().unwrap();

        // An AMD card.
        write_file(&root.path().join("card0/device/vendor"), "0x1002");
        // An Intel iGPU — same shape, different vendor.
        write_file(&root.path().join("card1/device/vendor"), "0x8086");
        // An NVIDIA card.
        write_file(&root.path().join("card2/device/vendor"), "0x10de");
        // A connector belonging to card0.
        write_file(&root.path().join("card0-DP-1/device/vendor"), "0x1002");

        let cards = discover_cards(root.path());
        assert_eq!(cards.len(), 1, "only the AMD card should be discovered");
        assert!(cards[0].ends_with("card0/device"));
    }

    #[test]
    fn discover_orders_cards_numerically() {
        // `read_dir` returns entries in filesystem order, which puts card10
        // before card2 lexicographically. Device indices must not depend on
        // that — the user's row selection would jump between ticks.
        let root = tempdir().unwrap();
        for n in [0u32, 2, 10] {
            write_file(
                &root.path().join(format!("card{n}/device/vendor")),
                "0x1002",
            );
        }

        let cards = discover_cards(root.path());
        assert_eq!(cards.len(), 3);
        assert!(cards[0].ends_with("card0/device"));
        assert!(cards[1].ends_with("card2/device"));
        assert!(cards[2].ends_with("card10/device"));
    }

    #[test]
    fn discover_on_missing_root_is_empty_not_a_panic() {
        let cards = discover_cards(Path::new("/definitely/not/a/real/drm/root"));
        assert!(cards.is_empty());
    }

    #[test]
    fn connect_at_without_amd_cards_reports_driver_unavailable() {
        let root = tempdir().unwrap();
        write_file(&root.path().join("card0/device/vendor"), "0x10de"); // NVIDIA

        let err = AmdEngine::connect_at(root.path()).unwrap_err();
        assert!(
            matches!(err, GpuError::DriverUnavailable { vendor: "AMD", .. }),
            "got {err:?}"
        );
        assert!(err.to_string().contains("no amdgpu card"));
    }

    // ---- unit conversions -------------------------------------------------

    #[test]
    fn sysfs_units_are_converted() {
        assert_eq!(hz_to_mhz(2_394_000_000), 2394);
        assert_eq!(hz_to_mhz(0), 0);
        assert!((uw_to_watts(289_000_000) - 289.0).abs() < 0.001);
        assert!((uw_to_watts(0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fan_pct_scales_pwm_to_percent() {
        let hwmon = tempdir().unwrap();
        write_file(&hwmon.path().join("pwm1"), "128");
        write_file(&hwmon.path().join("pwm1_max"), "255");
        let pct = read_fan_pct(hwmon.path()).unwrap();
        assert!((pct - 50.196).abs() < 0.01, "got {pct}");
    }

    #[test]
    fn fan_pct_defaults_max_to_255() {
        let hwmon = tempdir().unwrap();
        write_file(&hwmon.path().join("pwm1"), "255");
        let pct = read_fan_pct(hwmon.path()).unwrap();
        assert!((pct - 100.0).abs() < 0.01, "got {pct}");
    }

    #[test]
    fn fan_pct_survives_zero_max() {
        // A `pwm1_max` of 0 would divide by zero. Fall back to 255.
        let hwmon = tempdir().unwrap();
        write_file(&hwmon.path().join("pwm1"), "255");
        write_file(&hwmon.path().join("pwm1_max"), "0");
        let pct = read_fan_pct(hwmon.path()).unwrap();
        assert!(pct.is_finite(), "pwm1_max=0 produced {pct}");
        assert!((pct - 100.0).abs() < 0.01);
    }

    #[test]
    fn fan_pct_is_none_on_fanless_card() {
        let hwmon = tempdir().unwrap();
        write_file(&hwmon.path().join("temp1_input"), "50000");
        assert_eq!(read_fan_pct(hwmon.path()), None);
    }

    // ---- pp_dpm parsing ---------------------------------------------------

    #[test]
    fn dpm_table_picks_the_starred_state() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pp_dpm_sclk");
        write_file(&path, "0: 500Mhz\n1: 1200Mhz *\n2: 2200Mhz");
        assert_eq!(read_active_dpm_mhz(&path), Some(1200));
    }

    #[test]
    fn dpm_table_without_active_state_is_none() {
        // Mid-transition the driver stars nothing. Guessing the first row
        // would report an idle clock as the current one.
        let dir = tempdir().unwrap();
        let path = dir.path().join("pp_dpm_sclk");
        write_file(&path, "0: 500Mhz\n1: 1200Mhz\n2: 2200Mhz");
        assert_eq!(read_active_dpm_mhz(&path), None);
    }

    #[test]
    fn dpm_table_missing_file_is_none() {
        assert_eq!(read_active_dpm_mhz(Path::new("/no/such/pp_dpm_sclk")), None);
    }

    #[test]
    fn dpm_table_tolerates_garbage() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pp_dpm_sclk");
        write_file(&path, "not a table at all *");
        assert_eq!(read_active_dpm_mhz(&path), None);
    }

    // ---- full device read -------------------------------------------------

    #[test]
    fn reads_a_fully_populated_card() {
        let (_root, device) = fixture_full_card();
        let snap = read_device(&device, 0, Some("6.11.0".into()));

        assert_eq!(snap.index, 0);
        assert_eq!(snap.vendor, GpuVendor::Amd);
        assert_eq!(snap.backend, GpuBackend::AmdSysfs);
        assert_eq!(snap.name, "AMD Radeon RX 7900 XTX");
        assert_eq!(snap.bus_id, "0000:03:00.0");
        assert_eq!(snap.driver_version.as_deref(), Some("6.11.0"));
        assert_eq!(snap.utilization_pct, Some(73.0));
        assert_eq!(snap.mem_utilization_pct, Some(41.0));
        assert_eq!(snap.mem_used_bytes, Some(8_589_934_592));
        assert_eq!(snap.mem_total_bytes, Some(25_757_220_864));
        assert_eq!(snap.temperature_c, Some(67.0));
        assert_eq!(snap.power_watts, Some(289.0));
        assert_eq!(snap.power_limit_watts, Some(355.0));
        assert_eq!(snap.graphics_clock_mhz, Some(2394));
        assert_eq!(snap.memory_clock_mhz, Some(1249));
        assert!(snap.fan_pct.is_some());

        // The two NVML-only metrics, and the defining limitation of sysfs.
        assert_eq!(snap.encoder_pct, None);
        assert_eq!(snap.decoder_pct, None);
        assert!(!snap.supports_process_stats);
    }

    #[test]
    fn missing_attributes_become_none_not_zero() {
        // The core degradation contract: a kernel too old to expose
        // `gpu_busy_percent` must render `—`, not a confident `0 %`.
        let root = tempdir().unwrap();
        let device = root.path().join("card0").join("device");
        write_file(&device.join("vendor"), "0x1002");

        let snap = read_device(&device, 0, None);
        assert_eq!(snap.utilization_pct, None);
        assert_eq!(snap.mem_used_bytes, None);
        assert_eq!(snap.mem_total_bytes, None);
        assert_eq!(snap.temperature_c, None);
        assert_eq!(snap.power_watts, None);
        assert_eq!(snap.fan_pct, None);
        assert_eq!(snap.graphics_clock_mhz, None);
        assert_eq!(snap.driver_version, None);
    }

    #[test]
    fn falls_back_to_pci_id_when_product_name_is_absent() {
        let root = tempdir().unwrap();
        let device = root.path().join("card0").join("device");
        write_file(&device.join("vendor"), "0x1002");
        write_file(&device.join("device"), "0x73df");

        let snap = read_device(&device, 0, None);
        assert_eq!(snap.name, "AMD GPU 0x73df");
    }

    #[test]
    fn falls_back_again_when_even_the_id_is_absent() {
        let root = tempdir().unwrap();
        let device = root.path().join("card0").join("device");
        write_file(&device.join("vendor"), "0x1002");

        let snap = read_device(&device, 0, None);
        assert_eq!(snap.name, "AMD GPU");
    }

    #[test]
    fn dpm_tables_back_up_missing_hwmon_frequencies() {
        let root = tempdir().unwrap();
        let device = root.path().join("card0").join("device");
        write_file(&device.join("vendor"), "0x1002");
        write_file(&device.join("pp_dpm_sclk"), "0: 500Mhz\n1: 2200Mhz *");
        write_file(&device.join("pp_dpm_mclk"), "0: 96Mhz\n1: 1124Mhz *");

        let snap = read_device(&device, 0, None);
        assert_eq!(snap.graphics_clock_mhz, Some(2200));
        assert_eq!(snap.memory_clock_mhz, Some(1124));
    }

    #[test]
    fn hwmon_frequencies_win_over_dpm_tables() {
        // hwmon is the live reading; the DPM table is a state list. When both
        // exist the live one must be preferred.
        let (_root, device) = fixture_full_card();
        write_file(&device.join("pp_dpm_sclk"), "0: 500Mhz *");

        let snap = read_device(&device, 0, None);
        assert_eq!(snap.graphics_clock_mhz, Some(2394));
    }

    #[test]
    fn instantaneous_power_backs_up_average() {
        let root = tempdir().unwrap();
        let device = root.path().join("card0").join("device");
        write_file(&device.join("vendor"), "0x1002");
        write_file(&device.join("hwmon/hwmon0/power1_input"), "150000000");

        let snap = read_device(&device, 0, None);
        assert_eq!(snap.power_watts, Some(150.0));
    }

    // ---- engine-level -----------------------------------------------------

    #[tokio::test]
    async fn snapshot_reports_devices_and_no_processes() {
        let (root, _device) = fixture_full_card();
        let engine = AmdEngine::connect_at(root.path()).expect("fixture has an AMD card");
        assert_eq!(engine.device_count(), 1);
        assert_eq!(engine.backend(), GpuBackend::AmdSysfs);

        let snap = engine.snapshot().await.expect("snapshot must not error");
        assert!(snap.available);
        assert_eq!(snap.backends, vec![GpuBackend::AmdSysfs]);
        assert_eq!(snap.devices.len(), 1);
        assert_eq!(snap.devices[0].name, "AMD Radeon RX 7900 XTX");

        // sysfs has no per-process accounting — the empty list must be
        // explained by the flag, not mistaken for an idle GPU.
        assert!(snap.processes.is_empty());
        assert!(!snap.any_process_stats());
    }

    #[tokio::test]
    async fn snapshot_indices_are_dense_across_multiple_cards() {
        let root = tempdir().unwrap();
        for n in [0u32, 1] {
            let device = root.path().join(format!("card{n}")).join("device");
            write_file(&device.join("vendor"), "0x1002");
            write_file(&device.join("gpu_busy_percent"), "10");
        }

        let engine = AmdEngine::connect_at(root.path()).unwrap();
        let snap = engine.snapshot().await.unwrap();

        let indices: Vec<u32> = snap.devices.iter().map(|d| d.index).collect();
        assert_eq!(indices, vec![0, 1]);
    }
}
