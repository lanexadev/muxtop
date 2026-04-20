use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Information about a single process.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Encode, Decode)]
pub struct ProcessInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub command: String,
    pub user: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub memory_percent: f32,
    pub status: String,
}

/// Fields by which a process list can be sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortField {
    Cpu,
    Mem,
    Pid,
    Name,
    User,
}

impl std::fmt::Display for SortField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortField::Cpu => write!(f, "cpu"),
            SortField::Mem => write!(f, "mem"),
            SortField::Pid => write!(f, "pid"),
            SortField::Name => write!(f, "name"),
            SortField::User => write!(f, "user"),
        }
    }
}

impl std::str::FromStr for SortField {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "cpu" => Ok(SortField::Cpu),
            "mem" | "memory" => Ok(SortField::Mem),
            "pid" => Ok(SortField::Pid),
            "name" => Ok(SortField::Name),
            "user" => Ok(SortField::User),
            _ => Err(format!(
                "invalid sort field '{s}': expected one of cpu, mem, pid, name, user"
            )),
        }
    }
}

/// Direction of a sort operation.
#[derive(Debug, Clone, Copy)]
pub enum SortOrder {
    Asc,
    Desc,
}

/// Sorts `procs` in-place by `field` in the given `order`.
/// Name and User comparisons are case-insensitive.
pub fn sort_processes(procs: &mut [ProcessInfo], field: SortField, order: SortOrder) {
    // Name/User use `sort_by_cached_key` so `to_lowercase()` runs O(n) instead
    // of O(n log n) times — a comparator-based sort reallocates the lowercased
    // key on every comparison.
    match field {
        SortField::Name => procs.sort_by_cached_key(|p| p.name.to_lowercase()),
        SortField::User => procs.sort_by_cached_key(|p| p.user.to_lowercase()),
        SortField::Cpu => procs.sort_unstable_by(|a, b| {
            a.cpu_percent
                .partial_cmp(&b.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        SortField::Mem => procs.sort_unstable_by_key(|p| p.memory_bytes),
        SortField::Pid => procs.sort_unstable_by_key(|p| p.pid),
    }

    if matches!(order, SortOrder::Desc) {
        procs.reverse();
    }
}

/// Returns a new `Vec` containing every process whose `name` or `command`
/// contains `pattern` (case-insensitive). An empty pattern matches everything.
pub fn filter_processes(procs: &[ProcessInfo], pattern: &str) -> Vec<ProcessInfo> {
    if pattern.is_empty() {
        return procs.to_vec();
    }
    let lower = pattern.to_lowercase();
    procs
        .iter()
        .filter(|p| {
            p.name.to_lowercase().contains(&lower) || p.command.to_lowercase().contains(&lower)
        })
        .cloned()
        .collect()
}

/// A node in the process tree — wraps a `ProcessInfo` with children and depth.
#[derive(Debug, Clone)]
pub struct TreeNode {
    pub process: ProcessInfo,
    pub children: Vec<TreeNode>,
    pub depth: usize,
}

/// Build a process tree from a flat list.
///
/// Processes whose `parent_pid` matches another process's `pid` become children
/// of that parent. Processes with no parent in the list (orphans), `parent_pid`
/// of `None`, or `parent_pid` of `Some(0)` become root nodes at depth 0.
pub fn build_process_tree(procs: &[ProcessInfo]) -> Vec<TreeNode> {
    use std::collections::HashMap;

    if procs.is_empty() {
        return Vec::new();
    }

    // Index: pid → list of child ProcessInfo indices.
    let mut children_map: HashMap<u32, Vec<usize>> = HashMap::with_capacity(procs.len());
    let pid_set: std::collections::HashSet<u32> = procs.iter().map(|p| p.pid).collect();

    for (i, p) in procs.iter().enumerate() {
        match p.parent_pid {
            Some(ppid) if ppid > 0 && pid_set.contains(&ppid) => {
                children_map.entry(ppid).or_default().push(i);
            }
            _ => {} // root or orphan — handled below
        }
    }

    // Identify roots: no parent, parent_pid == 0/None, or parent not in list.
    let roots: Vec<usize> = procs
        .iter()
        .enumerate()
        .filter(|(_, p)| match p.parent_pid {
            None | Some(0) => true,
            Some(ppid) => !pid_set.contains(&ppid),
        })
        .map(|(i, _)| i)
        .collect();

    const MAX_DEPTH: usize = 256;

    fn build_subtree(
        idx: usize,
        depth: usize,
        procs: &[ProcessInfo],
        children_map: &HashMap<u32, Vec<usize>>,
    ) -> TreeNode {
        let p = &procs[idx];
        let children = if depth < MAX_DEPTH {
            children_map
                .get(&p.pid)
                .map(|indices| indices.as_slice())
                .unwrap_or_default()
                .iter()
                .map(|&ci| build_subtree(ci, depth + 1, procs, children_map))
                .collect()
        } else {
            Vec::new() // Truncate at max depth to prevent stack overflow.
        };
        TreeNode {
            process: p.clone(),
            children,
            depth,
        }
    }

    roots
        .iter()
        .map(|&ri| build_subtree(ri, 0, procs, &children_map))
        .collect()
}

/// Flatten a tree into a depth-first ordered list of `(ProcessInfo, depth)` pairs.
/// Useful for rendering the tree view in the TUI.
pub fn flatten_tree(roots: &[TreeNode]) -> Vec<(ProcessInfo, usize)> {
    let mut result = Vec::new();
    fn walk(node: &TreeNode, out: &mut Vec<(ProcessInfo, usize)>) {
        out.push((node.process.clone(), node.depth));
        for child in &node.children {
            walk(child, out);
        }
    }
    for root in roots {
        walk(root, &mut result);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_proc(pid: u32, name: &str, cmd: &str, user: &str, cpu: f32, mem: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid: None,
            name: name.to_string(),
            command: cmd.to_string(),
            user: user.to_string(),
            cpu_percent: cpu,
            memory_bytes: mem,
            memory_percent: 0.0,
            status: "Running".to_string(),
        }
    }

    fn cpu_procs() -> Vec<ProcessInfo> {
        vec![
            make_proc(1, "a", "", "alice", 10.0, 100),
            make_proc(2, "b", "", "bob", 50.0, 200),
            make_proc(3, "c", "", "carol", 30.0, 300),
            make_proc(4, "d", "", "dave", 90.0, 400),
            make_proc(5, "e", "", "eve", 20.0, 500),
        ]
    }

    #[test]
    fn test_sort_by_cpu_desc() {
        let mut procs = cpu_procs();
        sort_processes(&mut procs, SortField::Cpu, SortOrder::Desc);
        let cpus: Vec<f32> = procs.iter().map(|p| p.cpu_percent).collect();
        assert_eq!(cpus, vec![90.0, 50.0, 30.0, 20.0, 10.0]);
    }

    #[test]
    fn test_sort_by_cpu_asc() {
        let mut procs = cpu_procs();
        sort_processes(&mut procs, SortField::Cpu, SortOrder::Asc);
        let cpus: Vec<f32> = procs.iter().map(|p| p.cpu_percent).collect();
        assert_eq!(cpus, vec![10.0, 20.0, 30.0, 50.0, 90.0]);
    }

    #[test]
    fn test_sort_by_name_asc() {
        let mut procs = vec![
            make_proc(1, "Zsh", "", "u", 0.0, 0),
            make_proc(2, "apache", "", "u", 0.0, 0),
            make_proc(3, "Bash", "", "u", 0.0, 0),
        ];
        sort_processes(&mut procs, SortField::Name, SortOrder::Asc);
        let names: Vec<&str> = procs.iter().map(|p| p.name.as_str()).collect();
        // case-insensitive: apache < Bash < Zsh
        assert_eq!(names, vec!["apache", "Bash", "Zsh"]);
    }

    #[test]
    fn test_sort_by_mem_desc() {
        let mut procs = cpu_procs();
        sort_processes(&mut procs, SortField::Mem, SortOrder::Desc);
        let mems: Vec<u64> = procs.iter().map(|p| p.memory_bytes).collect();
        assert_eq!(mems, vec![500, 400, 300, 200, 100]);
    }

    #[test]
    fn test_sort_by_pid_asc() {
        let mut procs = vec![
            make_proc(30, "c", "", "u", 0.0, 0),
            make_proc(10, "a", "", "u", 0.0, 0),
            make_proc(20, "b", "", "u", 0.0, 0),
        ];
        sort_processes(&mut procs, SortField::Pid, SortOrder::Asc);
        let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![10, 20, 30]);
    }

    #[test]
    fn test_sort_by_user_asc() {
        let mut procs = vec![
            make_proc(1, "a", "", "Zara", 0.0, 0),
            make_proc(2, "b", "", "alice", 0.0, 0),
            make_proc(3, "c", "", "Bob", 0.0, 0),
        ];
        sort_processes(&mut procs, SortField::User, SortOrder::Asc);
        let users: Vec<&str> = procs.iter().map(|p| p.user.as_str()).collect();
        assert_eq!(users, vec!["alice", "Bob", "Zara"]);
    }

    #[test]
    fn test_sort_empty_list() {
        let mut procs: Vec<ProcessInfo> = vec![];
        // Must not panic.
        sort_processes(&mut procs, SortField::Cpu, SortOrder::Desc);
        assert!(procs.is_empty());
    }

    #[test]
    fn test_sort_single_element() {
        let mut procs = vec![make_proc(42, "solo", "/bin/solo", "root", 5.0, 1024)];
        sort_processes(&mut procs, SortField::Cpu, SortOrder::Desc);
        assert_eq!(procs.len(), 1);
        assert_eq!(procs[0].pid, 42);
    }

    #[test]
    fn test_filter_case_insensitive() {
        let procs = vec![
            make_proc(1, "Firefox", "/usr/bin/firefox", "u", 0.0, 0),
            make_proc(2, "firefox-esr", "/usr/bin/firefox-esr", "u", 0.0, 0),
            make_proc(3, "bash", "/bin/bash", "u", 0.0, 0),
        ];
        let result = filter_processes(&procs, "fire");
        assert_eq!(result.len(), 2);
        let names: Vec<&str> = result.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"Firefox"));
        assert!(names.contains(&"firefox-esr"));
    }

    #[test]
    fn test_filter_empty_pattern() {
        let procs = cpu_procs();
        let result = filter_processes(&procs, "");
        assert_eq!(result.len(), procs.len());
    }

    #[test]
    fn test_filter_no_match() {
        let procs = cpu_procs();
        let result = filter_processes(&procs, "zzznomatch");
        assert!(result.is_empty());
    }

    #[test]
    fn test_filter_matches_command_field() {
        let procs = vec![
            make_proc(1, "proc1", "/usr/bin/rustc --edition 2021", "u", 0.0, 0),
            make_proc(2, "proc2", "/bin/bash", "u", 0.0, 0),
        ];
        // "rustc" is only in the command field, not the name.
        let result = filter_processes(&procs, "rustc");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].pid, 1);
    }

    // ---- Tree tests (STORY-04) ----

    fn make_proc_with_parent(pid: u32, ppid: Option<u32>, name: &str) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid: ppid,
            name: name.to_string(),
            command: String::new(),
            user: "u".to_string(),
            cpu_percent: 0.0,
            memory_bytes: 0,
            memory_percent: 0.0,
            status: "Running".to_string(),
        }
    }

    #[test]
    fn test_tree_parent_child() {
        let procs = vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(100, Some(1), "sshd"),
            make_proc_with_parent(200, Some(100), "bash"),
        ];
        let tree = build_process_tree(&procs);
        assert_eq!(tree.len(), 1, "should have one root (init)");
        assert_eq!(tree[0].process.pid, 1);
        assert_eq!(tree[0].children.len(), 1, "init should have 1 child");
        assert_eq!(tree[0].children[0].process.pid, 100);
        assert_eq!(
            tree[0].children[0].children.len(),
            1,
            "sshd should have 1 child"
        );
        assert_eq!(tree[0].children[0].children[0].process.pid, 200);
    }

    #[test]
    fn test_tree_depth_increments() {
        let procs = vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(10, Some(1), "child"),
            make_proc_with_parent(100, Some(10), "grandchild"),
        ];
        let tree = build_process_tree(&procs);
        assert_eq!(tree[0].depth, 0);
        assert_eq!(tree[0].children[0].depth, 1);
        assert_eq!(tree[0].children[0].children[0].depth, 2);
    }

    #[test]
    fn test_tree_orphan_as_root() {
        let procs = vec![make_proc_with_parent(500, Some(999), "orphan")];
        let tree = build_process_tree(&procs);
        assert_eq!(tree.len(), 1, "orphan with missing parent should be a root");
        assert_eq!(tree[0].process.pid, 500);
        assert_eq!(tree[0].depth, 0);
    }

    #[test]
    fn test_tree_multiple_roots() {
        let procs = vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(2, Some(0), "kthreadd"),
        ];
        let tree = build_process_tree(&procs);
        assert_eq!(
            tree.len(),
            2,
            "two processes with PPID=0 should both be roots"
        );
    }

    #[test]
    fn test_tree_empty_list() {
        let tree = build_process_tree(&[]);
        assert!(tree.is_empty());
    }

    #[test]
    fn test_tree_single_process() {
        let procs = vec![make_proc_with_parent(42, None, "solo")];
        let tree = build_process_tree(&procs);
        assert_eq!(tree.len(), 1);
        assert!(tree[0].children.is_empty());
    }

    #[test]
    fn test_flatten_tree_order() {
        let procs = vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(10, Some(1), "child_a"),
            make_proc_with_parent(20, Some(1), "child_b"),
            make_proc_with_parent(100, Some(10), "grandchild"),
        ];
        let tree = build_process_tree(&procs);
        let flat = flatten_tree(&tree);
        let pids: Vec<u32> = flat.iter().map(|(p, _)| p.pid).collect();
        // DFS: init → child_a → grandchild → child_b
        assert_eq!(pids, vec![1, 10, 100, 20]);
    }

    #[test]
    fn test_flatten_tree_depth_values() {
        let procs = vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(10, Some(1), "child"),
            make_proc_with_parent(100, Some(10), "grandchild"),
        ];
        let tree = build_process_tree(&procs);
        let flat = flatten_tree(&tree);
        let depths: Vec<usize> = flat.iter().map(|(_, d)| *d).collect();
        assert_eq!(depths, vec![0, 1, 2]);
    }

    // ---- SortField FromStr tests (STORY-01) ----

    #[test]
    fn test_sort_field_from_str_valid() {
        assert_eq!("cpu".parse::<SortField>().unwrap(), SortField::Cpu);
        assert_eq!("mem".parse::<SortField>().unwrap(), SortField::Mem);
        assert_eq!("memory".parse::<SortField>().unwrap(), SortField::Mem);
        assert_eq!("pid".parse::<SortField>().unwrap(), SortField::Pid);
        assert_eq!("name".parse::<SortField>().unwrap(), SortField::Name);
        assert_eq!("user".parse::<SortField>().unwrap(), SortField::User);
    }

    #[test]
    fn test_sort_field_from_str_case_insensitive() {
        assert_eq!("CPU".parse::<SortField>().unwrap(), SortField::Cpu);
        assert_eq!("Mem".parse::<SortField>().unwrap(), SortField::Mem);
        assert_eq!("PID".parse::<SortField>().unwrap(), SortField::Pid);
    }

    #[test]
    fn test_sort_field_from_str_invalid() {
        let err = "invalid".parse::<SortField>().unwrap_err();
        assert!(err.contains("invalid sort field"));
        assert!(err.contains("cpu, mem, pid, name, user"));
    }

    #[test]
    fn test_sort_field_display_roundtrip() {
        for &field in &[
            SortField::Cpu,
            SortField::Mem,
            SortField::Pid,
            SortField::Name,
            SortField::User,
        ] {
            let s = field.to_string();
            assert_eq!(s.parse::<SortField>().unwrap(), field);
        }
    }

    #[test]
    fn test_tree_preserves_all_processes() {
        let procs = vec![
            make_proc_with_parent(1, Some(0), "init"),
            make_proc_with_parent(2, Some(0), "kthreadd"),
            make_proc_with_parent(10, Some(1), "a"),
            make_proc_with_parent(11, Some(1), "b"),
            make_proc_with_parent(20, Some(2), "c"),
            make_proc_with_parent(100, Some(10), "d"),
            make_proc_with_parent(101, Some(10), "e"),
            make_proc_with_parent(200, Some(20), "f"),
            make_proc_with_parent(500, Some(999), "orphan"),
            make_proc_with_parent(42, None, "none_parent"),
        ];
        let tree = build_process_tree(&procs);
        let flat = flatten_tree(&tree);
        assert_eq!(flat.len(), procs.len(), "all processes must be in the tree");
    }
}
