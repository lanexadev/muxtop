// System actions: kill and renice process wrappers.
// All unsafe libc calls are isolated in this module.

use crate::error::CoreError;

/// Safe subset of POSIX signals that muxtop is permitted to send.
///
/// The `i32` raw value is only accepted through this enum to prevent
/// callers from passing arbitrary signal numbers (e.g. SIGKILL to PID 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    /// Graceful termination request.
    Term,
    /// Unconditional kill.
    Kill,
}

impl Signal {
    fn as_libc(self) -> i32 {
        match self {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        }
    }
}

/// Send `signal` to the process identified by `pid`.
///
/// Maps libc errno values to typed `CoreError` variants:
/// - ESRCH  → `ProcessNotFound`
/// - EPERM  → `Permission`
/// - other  → `Io`
///
/// # Safety boundary
/// PIDs that would overflow `libc::pid_t` (i32) are rejected.
/// Negative pid values after cast are rejected (pid -1 is a POSIX wildcard
/// that sends the signal to ALL processes the caller can reach).
pub fn kill_process(pid: u32, signal: Signal) -> Result<(), CoreError> {
    // CRITICAL: pid as i32 must be positive. u32 values > i32::MAX wrap to
    // negative, and kill(-1, sig) sends sig to ALL user processes.
    let pid_i32 = i32::try_from(pid).map_err(|_| CoreError::ProcessNotFound { pid })?;
    if pid_i32 <= 0 {
        return Err(CoreError::ProcessNotFound { pid });
    }

    let ret = unsafe { libc::kill(pid_i32, signal.as_libc()) };
    if ret == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::ESRCH => Err(CoreError::ProcessNotFound { pid }),
        Some(code) if code == libc::EPERM => Err(CoreError::Permission(format!(
            "permission denied sending signal {signal:?} to pid {pid}"
        ))),
        _ => Err(CoreError::Io(err)),
    }
}

/// Change the scheduling priority (nice value) of the process identified by `pid`.
///
/// Because `setpriority` returns –1 both on error *and* as a valid success value
/// when the current priority happens to be –1, errno must be checked explicitly.
/// This function clears errno before the call and inspects it afterwards.
///
/// Maps libc errno values to typed `CoreError` variants:
/// - ESRCH  → `ProcessNotFound`
/// - EPERM  → `Permission`
/// - other  → `Io`
pub fn renice_process(pid: u32, nice_value: i32) -> Result<(), CoreError> {
    // Same overflow guard as kill_process — reject PIDs that don't fit in a
    // positive i32 to avoid silent wrapping to negative id_t values.
    let pid_i32 = i32::try_from(pid).map_err(|_| CoreError::ProcessNotFound { pid })?;
    if pid_i32 <= 0 {
        return Err(CoreError::ProcessNotFound { pid });
    }

    // Clear errno and call setpriority in a single unsafe block to prevent
    // any interleaving between the errno clear and the syscall.
    let ret = unsafe {
        set_errno_raw(0);
        libc::setpriority(libc::PRIO_PROCESS, pid_i32 as libc::id_t, nice_value)
    };

    if ret == 0 {
        return Ok(());
    }

    // ret == -1; check whether errno was actually set.
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(0) => Ok(()), // errno was not set — setpriority succeeded with value -1
        Some(code) if code == libc::ESRCH => Err(CoreError::ProcessNotFound { pid }),
        Some(code) if code == libc::EPERM => Err(CoreError::Permission(format!(
            "permission denied changing priority of pid {pid} to {nice_value}"
        ))),
        _ => Err(CoreError::Io(err)),
    }
}

/// Read the current scheduling priority (nice value) of the process identified by `pid`.
///
/// Because `getpriority` returns –1 both on error *and* as a valid success value,
/// errno must be checked explicitly (same pattern as `renice_process`).
///
/// Maps libc errno values to typed `CoreError` variants:
/// - ESRCH  → `ProcessNotFound`
/// - EPERM  → `Permission`
/// - other  → `Io`
pub fn get_process_priority(pid: u32) -> Result<i32, CoreError> {
    let pid_i32 = i32::try_from(pid).map_err(|_| CoreError::ProcessNotFound { pid })?;
    if pid_i32 <= 0 {
        return Err(CoreError::ProcessNotFound { pid });
    }

    let ret = unsafe {
        set_errno_raw(0);
        libc::getpriority(libc::PRIO_PROCESS, pid_i32 as libc::id_t)
    };

    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(0) => Ok(ret), // errno not set — return value is valid (may be -1)
        Some(code) if code == libc::ESRCH => Err(CoreError::ProcessNotFound { pid }),
        Some(code) if code == libc::EPERM => Err(CoreError::Permission(format!(
            "permission denied reading priority of pid {pid}"
        ))),
        _ => Err(CoreError::Io(err)),
    }
}

/// Raw errno write — must be called inside an existing `unsafe` block.
///
/// # Safety
/// Caller must be in an `unsafe` context.
unsafe fn set_errno_raw(value: i32) {
    #[cfg(target_os = "macos")]
    {
        unsafe {
            *libc::__error() = value;
        }
    }
    #[cfg(target_os = "linux")]
    {
        unsafe {
            *libc::__errno_location() = value;
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sending SIGTERM to a very large (non-existent) PID must return an error,
    /// proving the signal dispatch path is reachable via the Signal enum.
    #[test]
    fn test_kill_sigterm_nonexistent_pid() {
        let bad_pid: u32 = i32::MAX as u32 - 1;
        let result = kill_process(bad_pid, Signal::Term);
        assert!(result.is_err(), "kill(nonexistent, SIGTERM) must fail");
    }

    /// Sending SIGKILL to a non-existent PID must also return an error.
    #[test]
    fn test_kill_sigkill_nonexistent_pid() {
        let bad_pid: u32 = i32::MAX as u32 - 1;
        let result = kill_process(bad_pid, Signal::Kill);
        assert!(result.is_err(), "kill(nonexistent, SIGKILL) must fail");
    }

    /// A very large PID should fail safely without sending signals to anyone.
    #[test]
    fn test_kill_invalid_pid() {
        // Use a PID that is large but still a valid positive i32,
        // avoiding u32::MAX which wraps to -1 (POSIX wildcard: kill ALL processes).
        let bad_pid: u32 = i32::MAX as u32 - 1; // 2147483646 — almost certainly unused
        let result = kill_process(bad_pid, Signal::Term);
        assert!(
            result.is_err(),
            "kill(large_pid, SIGTERM) must return an error"
        );
        match result.unwrap_err() {
            CoreError::ProcessNotFound { .. } | CoreError::Permission(_) | CoreError::Io(_) => {}
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    /// u32::MAX must be rejected before reaching libc (it would become pid -1).
    #[test]
    fn test_kill_u32_max_rejected() {
        let result = kill_process(u32::MAX, Signal::Term);
        assert!(
            matches!(result, Err(CoreError::ProcessNotFound { .. })),
            "u32::MAX must be rejected as ProcessNotFound, got: {result:?}"
        );
    }

    /// PID 0 must be rejected (it means "all processes in the caller's process group").
    #[test]
    fn test_kill_pid_zero_rejected() {
        let result = kill_process(0, Signal::Term);
        assert!(
            matches!(result, Err(CoreError::ProcessNotFound { .. })),
            "pid 0 must be rejected, got: {result:?}"
        );
    }

    /// Lowering priority (raising nice value) is always permitted for the
    /// process itself on POSIX systems.
    #[test]
    fn test_renice_self() {
        let pid = std::process::id();
        let result = renice_process(pid, 10);
        assert!(result.is_ok(), "renice(self, 10) should succeed: {result:?}");
    }

    /// Renicing a very large PID should fail.
    #[test]
    fn test_renice_invalid_pid() {
        let bad_pid: u32 = i32::MAX as u32 - 1;
        let result = renice_process(bad_pid, 10);
        assert!(result.is_err(), "renice(large_pid, 10) must return an error");
    }

    /// Verify that each error path produces the expected discriminant.
    #[test]
    fn test_kill_renice_error_types() {
        let bad_pid: u32 = i32::MAX as u32 - 1;

        let r = kill_process(bad_pid, Signal::Term);
        if let Err(e) = r {
            let is_expected = matches!(
                e,
                CoreError::ProcessNotFound { .. } | CoreError::Permission(_) | CoreError::Io(_)
            );
            assert!(is_expected, "unexpected error variant: {e:?}");
        }

        let r2 = renice_process(bad_pid, 0);
        if let Err(e) = r2 {
            let is_expected = matches!(
                e,
                CoreError::ProcessNotFound { .. } | CoreError::Permission(_) | CoreError::Io(_)
            );
            assert!(is_expected, "unexpected error variant: {e:?}");
        }
    }

    /// get_process_priority must succeed for our own process.
    #[test]
    fn test_get_priority_self() {
        let pid = std::process::id();
        let result = get_process_priority(pid);
        assert!(result.is_ok(), "get_process_priority(self) should succeed: {result:?}");
        let nice = result.unwrap();
        assert!((-20..=19).contains(&nice), "nice value {nice} out of POSIX range");
    }

    /// get_process_priority with pid 0 must be rejected.
    #[test]
    fn test_get_priority_pid_zero_rejected() {
        let result = get_process_priority(0);
        assert!(
            matches!(result, Err(CoreError::ProcessNotFound { .. })),
            "pid 0 must be rejected, got: {result:?}"
        );
    }
}
