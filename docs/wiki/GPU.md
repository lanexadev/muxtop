# GPU

The GPU tab (`Alt+6`) shows utilisation, VRAM, temperature, power, clocks, fan
and NVENC/NVDEC across every detected card, with a per-process sub-view.

| Key | |
|---|---|
| `D` / `P` | **D**evices / **P**rocs sub-view |
| `]` / `[` | Cycle sub-views |
| `s` / `S` | Sort / reverse |
| `/` | Filter |
| `Enter` | Inspect — full clocks, driver detail |

**No extra privileges are needed.** NVML is readable by any user and the
`amdgpu` sysfs nodes are world-readable. Nothing is installed, no vendor SDK is
required at build time, and the NVIDIA library is loaded **dynamically at
runtime** — so the same binary runs on machines with and without an NVIDIA
driver. Disable the probe with `--no-gpu`.

---

## Backends

| Vendor | Backend | Platforms | Status |
|---|---|---|---|
| **NVIDIA** | NVML (`libnvidia-ml.so` / `nvml.dll`) | Linux, Windows | Full, including per-process usage |
| **AMD** | `amdgpu` sysfs (`/sys/class/drm/card*/device`) | Linux | Devices only — no per-process usage |
| **Intel** | — | — | Not implemented |
| **Apple Silicon** | IOReport | macOS | Planned for **v0.6** |

Multiple vendors are merged into one list, so an NVIDIA card and an AMD card on
the same host appear together.

---

## `—` means "cannot report", not "zero"

Unlike CPU or memory, **no GPU metric is universally available.** Every field is
optional in muxtop's data model and an unavailable one renders `—`.

This distinction matters more than it sounds: showing `0%` for a metric the
driver refuses to report would make the tab claim a busy GPU is idle. So:

- **AMD has no per-process accounting.** The `amdgpu` sysfs interface exposes no
  equivalent of NVML's process queries. The Procs sub-view says so explicitly
  rather than showing an empty list that reads as "nothing is using the GPU".
- **Encoder/decoder utilisation is NVML-only.** AMD renders `—` in that column.
- **Fanless cards report no fan.** A passively cooled card, or one whose fan is
  managed by the board rather than the GPU, has nothing to report.
- **A laptop dGPU parked in runtime-D3 may report nothing at all** until
  something wakes it. That is power management working correctly.
- **On Windows, per-process GPU memory is unavailable and every process shows
  `both`.** Under the WDDM driver model the OS owns video-memory allocation, so
  NVML reports the per-process figure as unavailable; it also returns identical
  compute and graphics process lists, so the TYPE column carries no information
  there. Both behaviours are the driver's. On Linux the figures and the
  compute/graphics distinction are real.

---

## Why Apple Silicon is not here yet

macOS exposes GPU counters only through the private `IOReport` framework — the
same source `powermetrics` reads, and `powermetrics` requires root.

Shipping that in v0.5 would have meant either taking a private-framework
dependency or asking every macOS user to run muxtop as root. Neither fits a tool
whose premise is staying out of the way. It is scheduled for **v0.6**; the data
model, the wire format and the tab already accommodate it, so it is plumbing
rather than redesign.

Until then, the GPU tab on an Apple Silicon Mac is empty. That is a missing
backend, not a broken detection.

---

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| Tab empty, NVIDIA card present | The driver is not loaded, or `libnvidia-ml.so` is not in the loader path. `nvidia-smi` is the test: if it fails, muxtop will too |
| Tab empty on Apple Silicon | Expected — not implemented until v0.6 |
| Tab empty, AMD card present | Check `ls /sys/class/drm/card*/device/gpu_busy_percent`. Absent means the `amdgpu` driver is not in use (an older `radeon` driver, or a passthrough VM) |
| Empty inside a container | `/dev/nvidia*` and `/dev/dri/*` are not passed through. For Docker, that is `--gpus all`; for Podman, `--device /dev/dri` |
| Empty under systemd, works in a shell | `PrivateDevices=true` in the unit blocks the device nodes — see [Remote monitoring](Remote-monitoring) |
| Procs sub-view says AMD is unsupported | Correct, and deliberate. `amdgpu` has no per-process query |
| Some columns `—`, others fine | Per-metric degradation working as designed. See the list above |
| Intel Arc / iGPU missing | Not implemented |

Detail lands in `~/.local/share/muxtop/muxtop.log`; `MUXTOP_LOG=debug muxtop`
logs which backends were probed and why each one was accepted or skipped.

---

## Read-only, always

The GPU tab has **no actions at all**. muxtop never sets a clock, a power limit
or a fan curve — it issues NVML query calls and reads
`/sys/class/drm/card*/device`, both read-only by construction. `--no-gpu` skips
the probe entirely.

GPU polling runs at **1 Hz**, on its own loop.
