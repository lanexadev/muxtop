//! Apple Silicon backend for the GPU tab (v0.7.0).
//!
//! # What v0.5 got wrong
//!
//! The v0.5 release deferred this backend on the grounds that macOS exposes
//! GPU counters only through `IOReport`, and that `IOReport` needs root
//! because `powermetrics` does. The first half is half true and the second
//! half is false, and both are worth correcting because they are the reason
//! the tab was empty on every Mac for two releases:
//!
//! * **Utilisation and memory are public.** The GPU driver publishes them in
//!   the IORegistry under `IOAccelerator`, through documented IOKit calls any
//!   user can make. That is where Activity Monitor's GPU graph comes from.
//! * **`IOReport` does not need root.** The GPU energy and performance-state
//!   channels are readable unprivileged. `powermetrics` needs root because it
//!   also reads channels muxtop never subscribes to.
//!
//! So the backend is layered rather than all-or-nothing:
//!
//! | Source | Gives | If it fails |
//! |---|---|---|
//! | IORegistry (`ioregistry.rs`) | name, cores, driver, utilisation, memory | no Apple GPU is reported at all |
//! | `IOReport` (`ioreport.rs`) | power, clock | those two columns render `—` |
//!
//! `IOReport` is a private framework, so its symbols are resolved with
//! `dlopen` at runtime exactly as the NVIDIA backend resolves NVML. A macOS
//! release that renames them costs two columns, not the tab.
//!
//! # What Apple does not report
//!
//! Held to the same rule as every other backend: **`—` means "cannot report",
//! never "zero".**
//!
//! * **No per-process accounting.** There is no Apple equivalent of NVML's
//!   process queries — no public one and no private one either. The Procs
//!   sub-view says so, as it does for AMD.
//! * **No GPU die temperature.** The `GPU Stats / Temperature` channels exist
//!   in the `IOReport` legend and read zero for an unprivileged process on
//!   every machine tested. Rather than publish a confident 0 °C on a warm
//!   laptop, the field stays `None`.
//! * **No power limit, no fan, no encoder/decoder counters.** The SoC power
//!   budget is shared with the CPU and managed by firmware, cooling is
//!   chassis-wide rather than per-GPU, and the media engine publishes no
//!   utilisation.
//!
//! # Unified memory is not VRAM
//!
//! The GPU addresses the same physical pool as the CPU. `MEM` is the driver's
//! `In use system memory` — bytes of the shared pool currently resident for
//! the GPU — and `MEM%` is that against the machine's whole memory, not
//! against a dedicated allocation. On a 16 GB Mac a GPU using 2 GB reads 12 %,
//! and the CPU is competing for the other 88 %.
//!
//! # Apple Silicon only
//!
//! The module compiles on any macOS target but reports only GPUs driven by
//! Apple's own `AGX` driver family. An Intel Mac's AMD or Intel GPU answers
//! the same `IOAccelerator` match with a differently shaped statistics
//! dictionary; claiming it here would put the wrong vendor on the row and read
//! the wrong keys.

/// Pure derivations, deliberately un-gated so their tests run on every CI
/// target — see the module's own doc for why.
pub mod metrics;

#[cfg(target_os = "macos")]
mod engine;
#[cfg(target_os = "macos")]
mod ioregistry;
#[cfg(target_os = "macos")]
mod ioreport;

#[cfg(target_os = "macos")]
pub use engine::AppleEngine;
