// System actions: kill and renice process wrappers.
// All unsafe libc calls are isolated in this module.

use crate::error::CoreError;

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
pub fn kill_process(pid: u32, signal: i32) -> Result<(), CoreError> {
    // CRITICAL: pid as i32 must be positive. u32 values > i32::MAX wrap to
    // negative, and kill(-1, sig) sends sig to ALL user processes.
    let pid_i32 = i32::try_from(pid).map_err(|_| CoreError::ProcessNotFound { pid })?;
    if pid_i32 <= 0 {
        return Err(CoreError::ProcessNotFound { pid });
    }

    let ret = unsafe { libc::kill(pid_i32, signal) };
    if ret == 0 {
        return Ok(());
    }

    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(code) if code == libc::ESRCH => Err(CoreError::ProcessNotFound { pid }),
        Some(code) if code == libc::EPERM => Err(CoreError::Permission(format!(
            "permission denied sending signal {signal} to pid {pid}"
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
    use std::io::Write;

    /// Helper: append a log line to /tmp/muxtop-test-actions.log
    fn log(msg: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("/tmp/muxtop-test-actions.log")
            .unwrap();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        writeln!(f, "[{ts}] {msg}").unwrap();
    }

    /// Signal 0 is a no-op existence check; it must succeed for our own process.
    #[test]
    fn test_kill_zero_signal_self() {
        let pid = std::process::id();
        log(&format!("test_kill_zero_signal_self: pid={pid}, signal=0"));
        let result = kill_process(pid, 0);
        log(&format!("test_kill_zero_signal_self: result={result:?}"));
        assert!(result.is_ok(), "kill(self, 0) should succeed");
    }

    /// A very large PID should fail safely without sending signals to anyone.
    #[test]
    fn test_kill_invalid_pid() {
        // Use a PID that is large but still a valid positive i32,
        // avoiding u32::MAX which wraps to -1 (POSIX wildcard: kill ALL processes).
        let bad_pid: u32 = i32::MAX as u32 - 1; // 2147483646 — almost certainly unused
        log(&format!(
            "test_kill_invalid_pid: pid={bad_pid}, signal=SIGTERM({})",
            libc::SIGTERM
        ));
        let result = kill_process(bad_pid, libc::SIGTERM);
        log(&format!("test_kill_invalid_pid: result={result:?}"));
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
        log("test_kill_u32_max_rejected: verifying u32::MAX is rejected");
        let result = kill_process(u32::MAX, 0);
        log(&format!("test_kill_u32_max_rejected: result={result:?}"));
        assert!(
            matches!(result, Err(CoreError::ProcessNotFound { .. })),
            "u32::MAX must be rejected as ProcessNotFound, got: {result:?}"
        );
    }

    /// PID 0 must be rejected (it means "all processes in the caller's process group").
    #[test]
    fn test_kill_pid_zero_rejected() {
        log("test_kill_pid_zero_rejected: verifying pid=0 is rejected");
        let result = kill_process(0, 0);
        log(&format!("test_kill_pid_zero_rejected: result={result:?}"));
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
        log(&format!("test_renice_self: pid={pid}, nice=10"));
        let result = renice_process(pid, 10);
        log(&format!("test_renice_self: result={result:?}"));
        assert!(
            result.is_ok(),
            "renice(self, 10) should succeed: {result:?}"
        );
    }

    /// Renicing a very large PID should fail.
    #[test]
    fn test_renice_invalid_pid() {
        let bad_pid: u32 = i32::MAX as u32 - 1;
        log(&format!("test_renice_invalid_pid: pid={bad_pid}, nice=10"));
        let result = renice_process(bad_pid, 10);
        log(&format!("test_renice_invalid_pid: result={result:?}"));
        assert!(
            result.is_err(),
            "renice(large_pid, 10) must return an error"
        );
    }

    /// Verify that each error path produces the expected discriminant.
    #[test]
    fn test_kill_renice_error_types() {
        let bad_pid: u32 = i32::MAX as u32 - 1;

        log(&format!(
            "test_kill_renice_error_types: kill bad_pid={bad_pid}"
        ));
        let r = kill_process(bad_pid, libc::SIGTERM);
        log(&format!("test_kill_renice_error_types: kill result={r:?}"));
        if let Err(e) = r {
            let is_expected = matches!(
                e,
                CoreError::ProcessNotFound { .. } | CoreError::Permission(_) | CoreError::Io(_)
            );
            assert!(is_expected, "unexpected error variant: {e:?}");
        }

        log(&format!(
            "test_kill_renice_error_types: renice bad_pid={bad_pid}"
        ));
        let r2 = renice_process(bad_pid, 0);
        log(&format!(
            "test_kill_renice_error_types: renice result={r2:?}"
        ));
        if let Err(e) = r2 {
            let is_expected = matches!(
                e,
                CoreError::ProcessNotFound { .. } | CoreError::Permission(_) | CoreError::Io(_)
            );
            assert!(is_expected, "unexpected error variant: {e:?}");
        }
    }
}
