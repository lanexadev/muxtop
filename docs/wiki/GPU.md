# GPU

The GPU tab (`Alt+6`) shows utilisation, memory, temperature, power, clocks, fan
and NVENC/NVDEC across every detected card, with a per-process sub-view.

| Key | |
|---|---|
| `D` / `P` | **D**evices / **P**rocs sub-view |
| `]` / `[` | Cycle sub-views |
| `s` / `S` | Sort / reverse |
| `/` | Filter |
| `Enter` | Inspect — full clocks, driver detail |

**No extra privileges are needed.** NVML is readable by any user, the `amdgpu`
sysfs nodes are world-readable, and the macOS counters muxtop reads are
available unprivileged — `powermetrics` needs root, the channels behind this tab
do not. Nothing is installed, no vendor SDK is required at build time, and both
the NVIDIA library and the macOS `IOReport` library are loaded **dynamically at
runtime** — so the same binary runs with or without them. Disable the probe with
`--no-gpu`.

---

## Backends

| Vendor | Backend | Platforms | Status |
|---|---|---|---|
| **NVIDIA** | NVML (`libnvidia-ml.so` / `nvml.dll`) | Linux, Windows | Full, including per-process usage |
| **AMD** | `amdgpu` sysfs (`/sys/class/drm/card*/device`) | Linux | Devices only — no per-process usage |
| **Apple Silicon** | IOKit `IOAccelerator` + `IOReport` | macOS | Devices only — no per-process usage, no temperature |
| **Intel** | — | — | Not implemented |

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

## Apple Silicon

Landed in **v0.7**, and it needs no root. The v0.5 notes deferred it on the
premise that macOS exposes GPU counters only through `IOReport` and that
`IOReport` needs root because `powermetrics` does. Neither half held up: the
driver publishes utilisation and memory through public IOKit calls, and the
`IOReport` channels muxtop subscribes to are readable by any user.

Two sources are read, and they fail independently:

| Source | Gives | If it fails |
|---|---|---|
| IOKit `IOAccelerator` — public API, no entitlement | device name, GPU core count, driver build, utilisation, memory | no Apple GPU is reported at all |
| `IOReport` — private framework, `dlopen`ed at runtime | power, clock | POWER and CLOCK render `—`; the tab is otherwise unaffected |

`IOReport` has no header and no stability promise, so its symbols are resolved
at runtime exactly as NVML's are. A macOS release that renames them costs two
columns, not the tab.

### Unified memory is not VRAM

The GPU addresses the same physical memory as the CPU, so the column header
reads **MEM** rather than VRAM on an Apple Silicon host. It shows the driver's
GPU-resident bytes against the machine's whole pool: on a 16 GB Mac a GPU
holding 2 GB reads 12 %, with the CPU competing for the rest. The Inspector
(`Enter`) labels it "Unified memory".

### What Apple does not report

- **No per-process usage.** There is no Apple equivalent of NVML's process
  queries, public or private. The Procs sub-view says so, as it does for AMD.
- **No GPU temperature.** The thermal channels exist in the `IOReport` legend
  and read zero for an unprivileged process. `—` is the honest answer; a
  confident 0 °C on a warm laptop is not.
- **No power limit, no fan, no encoder/decoder.** The SoC power budget is shared
  with the CPU and managed by firmware, cooling is chassis-wide rather than
  per-GPU (a MacBook Air has no fan at all), and the media engine publishes no
  utilisation counter.

### Intel Macs

Not covered. Their AMD or Intel GPU answers the same IOKit match with a
differently shaped statistics dictionary, and muxtop decodes only Apple's own
`AGX` driver family. The empty tab says so rather than showing a row labelled
Apple that reports nothing.

---

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| Tab empty, NVIDIA card present | The driver is not loaded, or `libnvidia-ml.so` is not in the loader path. `nvidia-smi` is the test: if it fails, muxtop will too |
| Tab empty on an Intel Mac | Expected — the macOS backend covers Apple Silicon only |
| POWER and CLOCK `—` on Apple Silicon, other columns fine | `IOReport` did not load. `MUXTOP_LOG=debug` names the symbol or path that failed; the rest of the tab is unaffected |
| Tab empty, AMD card present | Check `ls /sys/class/drm/card*/device/gpu_busy_percent`. Absent means the `amdgpu` driver is not in use (an older `radeon` driver, or a passthrough VM) |
| Empty inside a container | `/dev/nvidia*` and `/dev/dri/*` are not passed through. For Docker, that is `--gpus all`; for Podman, `--device /dev/dri` |
| Empty under systemd, works in a shell | `PrivateDevices=true` in the unit blocks the device nodes — see [Remote monitoring](Remote-monitoring) |
| Procs sub-view says the backend is unsupported | Correct, and deliberate. Neither `amdgpu` nor macOS has a per-process query |
| Some columns `—`, others fine | Per-metric degradation working as designed. See the list above |
| Intel Arc / iGPU missing | Not implemented |

Detail lands in `~/.local/share/muxtop/muxtop.log`; `MUXTOP_LOG=debug muxtop`
logs which backends were probed and why each one was accepted or skipped.

---

## Read-only, always

The GPU tab has **no actions at all**. muxtop never sets a clock, a power limit
or a fan curve — it issues NVML query calls, reads the `amdgpu` sysfs
attributes, and takes IOKit property and `IOReport` sample copies. All three are
read-only by construction. `--no-gpu` skips the probe entirely.

GPU polling runs at **1 Hz**, on its own loop.
