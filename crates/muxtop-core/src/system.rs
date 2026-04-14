use std::time::Instant;

use crate::process::ProcessInfo;

/// Per-core CPU snapshot.
#[derive(Debug, Clone)]
pub struct CoreSnapshot {
    pub name: String,
    pub usage: f32,
    pub frequency: u64,
}

/// Aggregated CPU snapshot with global usage and per-core data.
#[derive(Debug, Clone)]
pub struct CpuSnapshot {
    pub global_usage: f32,
    pub cores: Vec<CoreSnapshot>,
}

/// Memory and swap snapshot (all values in bytes).
#[derive(Debug, Clone)]
pub struct MemorySnapshot {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

/// System load averages and uptime.
#[derive(Debug, Clone)]
pub struct LoadSnapshot {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
    pub uptime_secs: u64,
}

/// Full system snapshot aggregating all subsystems.
#[derive(Debug, Clone)]
pub struct SystemSnapshot {
    pub cpu: CpuSnapshot,
    pub memory: MemorySnapshot,
    pub load: LoadSnapshot,
    pub processes: Vec<ProcessInfo>,
    pub timestamp: Instant,
}

impl SystemSnapshot {
    /// Collect a full system snapshot from sysinfo.
    pub fn collect(sys: &sysinfo::System) -> Self {
        use sysinfo::System as SysSystem;

        let global_usage = sys.global_cpu_usage();
        let cores = sys
            .cpus()
            .iter()
            .enumerate()
            .map(|(i, cpu)| CoreSnapshot {
                name: format!("cpu{i}"),
                usage: cpu.cpu_usage(),
                frequency: cpu.frequency(),
            })
            .collect();

        let cpu = CpuSnapshot {
            global_usage,
            cores,
        };

        let memory = MemorySnapshot {
            total: sys.total_memory(),
            used: sys.used_memory(),
            available: sys.available_memory(),
            swap_total: sys.total_swap(),
            swap_used: sys.used_swap(),
        };

        let load = {
            let avg = SysSystem::load_average();
            LoadSnapshot {
                one: avg.one,
                five: avg.five,
                fifteen: avg.fifteen,
                uptime_secs: SysSystem::uptime(),
            }
        };

        let total_mem = sys.total_memory();

        let processes = sys
            .processes()
            .iter()
            .map(|(pid, proc_info)| {
                let mem_pct = if total_mem > 0 {
                    ((proc_info.memory() as f64 / total_mem as f64) * 100.0)
                        .clamp(0.0, 100.0) as f32
                } else {
                    0.0
                };

                let status = match proc_info.status() {
                    sysinfo::ProcessStatus::Run => "Running",
                    sysinfo::ProcessStatus::Sleep => "Sleeping",
                    sysinfo::ProcessStatus::Idle => "Idle",
                    sysinfo::ProcessStatus::Zombie => "Zombie",
                    sysinfo::ProcessStatus::Stop => "Stopped",
                    _ => "Unknown",
                };

                ProcessInfo {
                    pid: pid.as_u32(),
                    parent_pid: proc_info.parent().map(|p| p.as_u32()),
                    name: proc_info.name().to_string_lossy().into_owned(),
                    command: proc_info
                        .cmd()
                        .iter()
                        .map(|s| s.to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join(" "),
                    user: proc_info
                        .user_id()
                        .map(|u| u.to_string())
                        .unwrap_or_default(),
                    cpu_percent: proc_info.cpu_usage(),
                    memory_bytes: proc_info.memory(),
                    memory_percent: mem_pct,
                    status: status.to_string(),
                }
            })
            .collect();

        Self {
            cpu,
            memory,
            load,
            processes,
            timestamp: Instant::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_is_send_clone() {
        fn assert_send_clone<T: Send + Clone>() {}
        assert_send_clone::<CoreSnapshot>();
        assert_send_clone::<CpuSnapshot>();
        assert_send_clone::<MemorySnapshot>();
        assert_send_clone::<LoadSnapshot>();
        assert_send_clone::<SystemSnapshot>();
        assert_send_clone::<ProcessInfo>();
    }

    #[test]
    fn test_system_snapshot_from_sysinfo() {
        use sysinfo::System;
        let mut sys = System::new_all();
        // sysinfo needs a refresh to populate CPU data
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys.refresh_all();

        let snap = SystemSnapshot::collect(&sys);
        assert!(!snap.cpu.cores.is_empty(), "should have CPU cores");
        assert!(snap.memory.total > 0, "should have total memory");
        assert!(!snap.processes.is_empty(), "should have processes");
    }

    #[test]
    fn test_cpu_snapshot_has_global_and_cores() {
        use sysinfo::System;
        let mut sys = System::new_all();
        std::thread::sleep(std::time::Duration::from_millis(200));
        sys.refresh_all();

        let snap = SystemSnapshot::collect(&sys);
        assert!(
            snap.cpu.global_usage >= 0.0 && snap.cpu.global_usage <= 100.0,
            "global CPU usage should be 0..=100, got {}",
            snap.cpu.global_usage
        );
        for core in &snap.cpu.cores {
            assert!(
                core.usage >= 0.0 && core.usage <= 100.0,
                "core usage should be 0..=100, got {}",
                core.usage
            );
        }
    }

    #[test]
    fn test_memory_snapshot_invariant() {
        use sysinfo::System;
        let mut sys = System::new_all();
        sys.refresh_all();

        let snap = SystemSnapshot::collect(&sys);
        assert!(
            snap.memory.total > 0,
            "total memory should be positive"
        );
        // used + available can slightly exceed total due to kernel accounting
        // but total should be >= used
        assert!(
            snap.memory.total >= snap.memory.used,
            "total ({}) should be >= used ({})",
            snap.memory.total,
            snap.memory.used
        );
    }

    #[test]
    fn test_all_structs_are_debug() {
        let core = CoreSnapshot {
            name: "cpu0".into(),
            usage: 50.0,
            frequency: 3600,
        };
        assert!(!format!("{core:?}").is_empty());

        let cpu = CpuSnapshot {
            global_usage: 25.0,
            cores: vec![core],
        };
        assert!(!format!("{cpu:?}").is_empty());

        let mem = MemorySnapshot {
            total: 16_000_000_000,
            used: 8_000_000_000,
            available: 8_000_000_000,
            swap_total: 4_000_000_000,
            swap_used: 1_000_000_000,
        };
        assert!(!format!("{mem:?}").is_empty());

        let load = LoadSnapshot {
            one: 1.5,
            five: 1.2,
            fifteen: 0.8,
            uptime_secs: 3600,
        };
        assert!(!format!("{load:?}").is_empty());
    }
}
