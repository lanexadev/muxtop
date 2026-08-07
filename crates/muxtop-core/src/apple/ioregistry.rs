//! IORegistry access for the Apple Silicon backend — the *public* half.
//!
//! Everything in this module goes through documented IOKit entry points that
//! any user can call: no entitlement, no root, no private framework. It is the
//! backend's floor. If `ioreport.rs` cannot load, the tab still shows a named
//! device with utilisation and memory, because those come from here.
//!
//! Two registry nodes are read:
//!
//! * **`IOAccelerator`** — the GPU driver's own node. Its `PerformanceStatistics`
//!   dictionary is what Activity Monitor's GPU history graph is drawn from, and
//!   it carries the device name, the GPU core count and the driver version.
//! * **`pmgr`** — the SoC power manager, for the GPU's DVFS table. That one is
//!   an undocumented device-tree blob; see [`gpu_dvfs_blob`].
//!
//! # Memory management
//!
//! IOKit follows the Core Foundation ownership rules. Functions with `Create`
//! or `Copy` in the name hand over a reference the caller must release;
//! everything else is borrowed. The wrappers below encode that with
//! `wrap_under_create_rule` / `wrap_under_get_rule`, and every `io_object_t`
//! is released on every path including the early returns.

use std::ffi::{CString, c_void};

use core_foundation::base::{CFType, TCFType, ToVoid};
use core_foundation::data::CFData;
use core_foundation::dictionary::CFDictionary;
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_foundation_sys::base::kCFAllocatorDefault;
use core_foundation_sys::dictionary::{CFDictionaryRef, CFMutableDictionaryRef};

/// `io_object_t` / `io_iterator_t` — both are `mach_port_t` underneath.
type IoObject = u32;
/// `kern_return_t`; `KERN_SUCCESS` is 0.
type KernReturn = i32;

/// `kIOMainPortDefault`. The constant is 0 on every OS version muxtop
/// supports, and using the literal avoids depending on which of the two
/// spellings (`kIOMasterPortDefault` before macOS 12) the SDK exports.
const IO_MAIN_PORT_DEFAULT: u32 = 0;

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const std::ffi::c_char) -> CFMutableDictionaryRef;
    fn IOServiceNameMatching(name: *const std::ffi::c_char) -> CFMutableDictionaryRef;
    fn IOServiceGetMatchingServices(
        main_port: u32,
        matching: CFDictionaryRef,
        existing: *mut IoObject,
    ) -> KernReturn;
    fn IOIteratorNext(iterator: IoObject) -> IoObject;
    fn IORegistryEntryCreateCFProperties(
        entry: IoObject,
        properties: *mut CFMutableDictionaryRef,
        allocator: *const c_void,
        options: u32,
    ) -> KernReturn;
    fn IOObjectRelease(object: IoObject) -> KernReturn;
}

/// A registry node's property dictionary.
type Properties = CFDictionary<CFString, CFType>;

/// Everything the IORegistry knows about one GPU, decoded into plain Rust.
///
/// Every field is optional for the same reason the wire model's are: this is a
/// driver-owned dictionary whose contents vary by chip generation, and a key
/// that is absent on the next one must degrade to `—` rather than to a
/// confident zero.
#[derive(Debug, Clone, Default)]
pub struct AcceleratorInfo {
    /// SoC marketing name, e.g. `Apple M3`.
    pub model: Option<String>,
    /// Driver class, e.g. `AGXAcceleratorG15G`. Used to tell an Apple GPU
    /// apart from the AMD or Intel one in an Intel Mac.
    pub class: Option<String>,
    pub gpu_core_count: Option<u32>,
    /// `IOSourceVersion` — the AGX driver build, the closest thing Apple
    /// exposes to NVML's driver version.
    pub driver_version: Option<String>,
    /// `Device Utilization %` from `PerformanceStatistics`.
    pub utilization_pct: Option<i64>,
    /// `In use system memory` — GPU-resident bytes of the unified pool.
    pub in_use_memory_bytes: Option<u64>,
}

impl AcceleratorInfo {
    /// Whether this node is driven by Apple's own GPU driver family.
    ///
    /// `IOAccelerator` is the *generic* accelerator class: on an Intel Mac the
    /// same match returns the AMD or Intel driver, whose statistics keys and
    /// units differ. Claiming those as Apple Silicon devices would put the
    /// wrong vendor on the row and read the wrong counters, so detection
    /// requires the `AGX` prefix that every Apple GPU driver carries.
    pub fn is_apple_gpu(&self) -> bool {
        self.class.as_deref().is_some_and(|c| c.starts_with("AGX"))
    }
}

/// Enumerate every `IOAccelerator` node and decode what we need from each.
///
/// Order is IOKit's own registry order, which is stable across ticks on a
/// machine whose hardware does not change — and Apple Silicon GPUs are not
/// hot-pluggable.
pub fn accelerators() -> Vec<AcceleratorInfo> {
    matching_properties(MatchBy::Class("IOAccelerator"))
        .iter()
        .map(decode_accelerator)
        .collect()
}

/// The GPU's DVFS table, as the raw `voltage-states9` device-tree blob.
///
/// # Why this is the fragile part
///
/// Apple documents neither the property nor which index belongs to which
/// engine; `voltage-states9` is the GPU's on every chip from M1 to M4, and
/// that is an observation, not a contract. The blob is therefore treated as
/// untrusted input: [`super::metrics::parse_dvfs_table`] rejects anything that
/// is not a whole number of `(Hz, mV)` pairs or that decodes to an impossible
/// clock, and the caller drops the CLK column rather than guessing. Losing one
/// column on a future chip is an acceptable failure mode; printing an invented
/// clock is not.
pub fn gpu_dvfs_blob() -> Option<Vec<u8>> {
    for props in matching_properties(MatchBy::Name("pmgr")) {
        if let Some(blob) = data_value(&props, "voltage-states9") {
            return Some(blob);
        }
    }
    None
}

/// Total unified memory, in bytes.
///
/// Apple Silicon has no VRAM: the GPU addresses the same physical pool as the
/// CPU, so the honest denominator for the MEM% column is the machine's whole
/// memory. `hw.memsize` is the documented sysctl for it.
pub fn unified_memory_bytes() -> Option<u64> {
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let name = c"hw.memsize";
    // SAFETY: `name` is a NUL-terminated literal, `value` and `len` are live
    // locals of exactly the type and size the sysctl writes, and the two
    // pointers sysctlbyname does not use are passed as null with a zero
    // length, as its contract requires for a read.
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr(),
            (&raw mut value).cast::<c_void>(),
            &raw mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0 && len == std::mem::size_of::<u64>()).then_some(value)
}

// ---- registry plumbing ---------------------------------------------------

/// How to look a service up: by driver class or by registry node name.
enum MatchBy {
    /// Matches the class *and its subclasses* — `IOAccelerator` finds
    /// `AGXAcceleratorG15G`.
    Class(&'static str),
    /// Matches the node's own name, which is how `pmgr` is addressed.
    Name(&'static str),
}

/// Property dictionaries of every service matching `by`.
///
/// Returns an empty vector on any failure. A machine with no matching node is
/// the normal case for callers here (an Intel Mac has no AGX accelerator), not
/// an error worth propagating.
fn matching_properties(by: MatchBy) -> Vec<Properties> {
    let mut out = Vec::new();

    let (name, matcher): (
        &str,
        unsafe extern "C" fn(*const i8) -> CFMutableDictionaryRef,
    ) = match by {
        MatchBy::Class(c) => (c, IOServiceMatching),
        MatchBy::Name(n) => (n, IOServiceNameMatching),
    };
    let Ok(c_name) = CString::new(name) else {
        return out;
    };

    // SAFETY: `c_name` outlives the call. `IOServiceGetMatchingServices`
    // consumes one reference on the matching dictionary, so the dictionary
    // returned by the matcher is handed over and never released here — that
    // is the documented convention and releasing it too would be a
    // double-free. Every `io_object_t` obtained below is released on every
    // path, and `IORegistryEntryCreateCFProperties` follows the Create rule,
    // which `wrap_under_create_rule` takes ownership of.
    unsafe {
        let matching = matcher(c_name.as_ptr());
        if matching.is_null() {
            return out;
        }

        let mut iterator: IoObject = 0;
        if IOServiceGetMatchingServices(IO_MAIN_PORT_DEFAULT, matching, &mut iterator) != 0 {
            return out;
        }

        loop {
            let entry = IOIteratorNext(iterator);
            if entry == 0 {
                break;
            }
            let mut properties: CFMutableDictionaryRef = std::ptr::null_mut();
            let rc =
                IORegistryEntryCreateCFProperties(entry, &mut properties, kCFAllocatorDefault, 0);
            if rc == 0 && !properties.is_null() {
                out.push(CFDictionary::wrap_under_create_rule(
                    properties as CFDictionaryRef,
                ));
            }
            IOObjectRelease(entry);
        }
        IOObjectRelease(iterator);
    }

    out
}

/// Pull the fields the engine needs out of one accelerator's dictionary.
fn decode_accelerator(props: &Properties) -> AcceleratorInfo {
    let stats = props
        .find(CFString::new("PerformanceStatistics"))
        .and_then(|v| v.downcast::<CFDictionary>());

    AcceleratorInfo {
        model: string_value(props, "model"),
        class: string_value(props, "IOClass"),
        gpu_core_count: number_value(props, "gpu-core-count").and_then(|n| u32::try_from(n).ok()),
        driver_version: string_value(props, "IOSourceVersion"),
        utilization_pct: stats
            .as_ref()
            .and_then(|s| untyped_number(s, "Device Utilization %")),
        in_use_memory_bytes: stats
            .as_ref()
            .and_then(|s| untyped_number(s, "In use system memory"))
            .and_then(|n| u64::try_from(n).ok()),
    }
}

/// A `CFString` property as a Rust `String`.
///
/// Some registry entries store text as `CFData` holding a NUL-terminated C
/// string rather than as a `CFString` — `model` is one of them on several
/// chips — so both encodings are accepted.
fn string_value(props: &Properties, key: &str) -> Option<String> {
    let value = props.find(CFString::new(key))?;
    if let Some(s) = value.downcast::<CFString>() {
        return Some(s.to_string());
    }
    let bytes = value.downcast::<CFData>()?;
    let text = String::from_utf8_lossy(bytes.bytes())
        .trim_end_matches('\0')
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

/// A `CFNumber` property as an `i64`.
fn number_value(props: &Properties, key: &str) -> Option<i64> {
    props
        .find(CFString::new(key))?
        .downcast::<CFNumber>()?
        .to_i64()
}

/// A `CFData` property as raw bytes.
fn data_value(props: &Properties, key: &str) -> Option<Vec<u8>> {
    Some(
        props
            .find(CFString::new(key))?
            .downcast::<CFData>()?
            .bytes()
            .to_vec(),
    )
}

/// A number out of a `CFDictionary` whose value type is erased.
///
/// `PerformanceStatistics` is a `CFDictionary` of `CFString` to `CFType`, but
/// the concrete generic parameters are not known to the `core-foundation`
/// wrapper when it comes back as an untyped `CFDictionary`, so the lookup has
/// to go through the raw key/value pointers.
fn untyped_number(dict: &CFDictionary, key: &str) -> Option<i64> {
    let cf_key = CFString::new(key);
    // SAFETY: `find` returns a borrowed value pointer that lives as long as
    // `dict`, which outlives this call. The pointer is only wrapped under the
    // Get rule (no ownership transfer) and only after `CFNumber`'s own type
    // check via `downcast`.
    unsafe {
        let value = dict.find(cf_key.to_void())?;
        CFType::wrap_under_get_rule(*value as _)
            .downcast::<CFNumber>()?
            .to_i64()
    }
}
