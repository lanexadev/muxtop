// Application state machine.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};

use muxtop_core::process::{
    ProcessInfo, SortField, SortOrder, build_process_tree, filter_processes, flatten_tree,
    sort_processes,
};
use muxtop_core::system::SystemSnapshot;

/// Tab identifiers for TUI views.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    #[default]
    General,
    Processes,
}

impl std::fmt::Display for Tab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

impl Tab {
    pub const ALL: &[Tab] = &[Tab::General, Tab::Processes];

    pub fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Processes => "Processes",
        }
    }

    pub fn next(self) -> Tab {
        let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Tab {
        let idx = Self::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Self::ALL[(idx + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// Full application state for the TUI.
pub struct AppState {
    pub tab: Tab,
    pub sort_field: SortField,
    pub sort_order: SortOrder,
    pub filter_input: String,
    pub filter_active: bool,
    pub tree_mode: bool,
    pub selected: usize,
    pub scroll_offset: usize,
    pub show_palette: bool,
    running: bool,
    pub last_snapshot: Option<SystemSnapshot>,
    /// Derived: sorted + filtered process list.
    pub visible_processes: Vec<ProcessInfo>,
    /// Derived: flattened tree (process, depth) pairs.
    pub visible_tree: Vec<(ProcessInfo, usize)>,
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        Self {
            tab: Tab::default(),
            sort_field: SortField::Cpu,
            sort_order: SortOrder::Desc,
            filter_input: String::new(),
            filter_active: false,
            tree_mode: false,
            selected: 0,
            scroll_offset: 0,
            show_palette: false,
            running: true,
            last_snapshot: None,
            visible_processes: Vec::new(),
            visible_tree: Vec::new(),
        }
    }

    pub fn running(&self) -> bool {
        self.running
    }

    pub fn quit(&mut self) {
        self.running = false;
    }

    /// Number of visible processes (respects tree_mode).
    pub fn process_count(&self) -> usize {
        if self.tree_mode {
            self.visible_tree.len()
        } else {
            self.visible_processes.len()
        }
    }

    /// Returns the currently selected process, if any.
    pub fn selected_process(&self) -> Option<&ProcessInfo> {
        if self.tree_mode {
            self.visible_tree.get(self.selected).map(|(p, _)| p)
        } else {
            self.visible_processes.get(self.selected)
        }
    }

    /// Update the snapshot and recompute derived views.
    pub fn apply_snapshot(&mut self, snapshot: SystemSnapshot) {
        self.last_snapshot = Some(snapshot);
        self.recompute_visible();
    }

    /// Recompute visible_processes and visible_tree from last_snapshot.
    pub fn recompute_visible(&mut self) {
        let Some(ref snapshot) = self.last_snapshot else {
            self.visible_processes.clear();
            self.visible_tree.clear();
            return;
        };

        // Filter
        let filtered = filter_processes(&snapshot.processes, &self.filter_input);

        // Sort
        let mut sorted = filtered;
        sort_processes(&mut sorted, self.sort_field, self.sort_order);
        self.visible_processes = sorted;

        // Tree — only build when tree_mode is active (G-09: skip when off).
        // G-07: tree is built from filtered list, not raw snapshot.
        if self.tree_mode {
            let tree =
                build_process_tree(&filter_processes(&snapshot.processes, &self.filter_input));
            self.visible_tree = flatten_tree(&tree);
        } else {
            self.visible_tree.clear();
        }

        // Clamp selection and scroll_offset (G-06).
        let count = self.process_count();
        if count > 0 {
            self.selected = self.selected.min(count - 1);
            self.scroll_offset = self.scroll_offset.min(count - 1);
        } else {
            self.selected = 0;
            self.scroll_offset = 0;
        }
    }

    /// Handle a keyboard event.
    pub fn handle_key_event(&mut self, key: KeyEvent) {
        // Filter mode captures most keys as text input.
        if self.filter_active {
            self.handle_filter_key(key);
            return;
        }

        match key.code {
            // Quit
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit(),

            // Navigation
            KeyCode::Down | KeyCode::Char('j') => {
                if self.process_count() > 0 {
                    self.selected = (self.selected + 1).min(self.process_count() - 1);
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::PageDown => {
                if self.process_count() > 0 {
                    self.selected = (self.selected + 20).min(self.process_count() - 1);
                }
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(20);
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.selected = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                if self.process_count() > 0 {
                    self.selected = self.process_count() - 1;
                }
            }

            // Direct tab selection (Alt+1/Alt+2)
            KeyCode::Char('1') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.tab = Tab::General;
            }
            KeyCode::Char('2') if key.modifiers.contains(KeyModifiers::ALT) => {
                self.tab = Tab::Processes;
            }

            // Arrow tab navigation
            KeyCode::Right => {
                self.tab = self.tab.next();
            }
            KeyCode::Left => {
                self.tab = self.tab.prev();
            }

            // Tab switching
            KeyCode::Tab => {
                self.tab = self.tab.next();
            }
            KeyCode::BackTab => {
                self.tab = self.tab.prev();
            }

            // Tree mode toggle (G-08: reset selection to avoid jumping to wrong process)
            KeyCode::Char('t') => {
                self.tree_mode = !self.tree_mode;
                self.selected = 0;
                self.scroll_offset = 0;
                self.recompute_visible();
            }

            // Sort cycling
            KeyCode::Char('s') => {
                self.sort_field = next_sort_field(self.sort_field);
                self.recompute_visible();
            }
            KeyCode::Char('S') => {
                self.sort_order = match self.sort_order {
                    SortOrder::Asc => SortOrder::Desc,
                    SortOrder::Desc => SortOrder::Asc,
                };
                self.recompute_visible();
            }

            // F-key sort shortcuts
            KeyCode::F(1) => {
                self.sort_field = SortField::Pid;
                self.recompute_visible();
            }
            KeyCode::F(2) => {
                self.sort_field = SortField::Name;
                self.recompute_visible();
            }
            KeyCode::F(3) => {
                self.sort_field = SortField::Cpu;
                self.recompute_visible();
            }
            KeyCode::F(4) => {
                self.sort_field = SortField::Mem;
                self.recompute_visible();
            }
            KeyCode::F(5) => {
                self.sort_field = SortField::User;
                self.recompute_visible();
            }

            // Filter mode
            KeyCode::Char('/') => {
                self.filter_active = true;
            }

            // Command palette toggle
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.show_palette = !self.show_palette;
            }

            _ => {}
        }
    }

    /// Handle keys while in filter input mode.
    fn handle_filter_key(&mut self, key: KeyEvent) {
        // G-03: Ctrl+C must always quit, even in filter mode.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit();
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.filter_active = false;
            }
            KeyCode::Enter => {
                self.filter_active = false;
                // Keep filter_input — it stays applied.
            }
            KeyCode::Backspace => {
                self.filter_input.pop();
                self.recompute_visible();
            }
            KeyCode::Char(c) => {
                self.filter_input.push(c);
                self.recompute_visible();
            }
            _ => {}
        }
    }

    /// Handle a mouse event.
    pub fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::ScrollDown => {
                let max = self.process_count().saturating_sub(1);
                self.scroll_offset = (self.scroll_offset + 1).min(max);
            }
            MouseEventKind::ScrollUp => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            _ => {}
        }
    }
}

/// Cycle to the next sort field.
fn next_sort_field(field: SortField) -> SortField {
    match field {
        SortField::Cpu => SortField::Mem,
        SortField::Mem => SortField::Pid,
        SortField::Pid => SortField::Name,
        SortField::Name => SortField::User,
        SortField::User => SortField::Cpu,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn make_process(pid: u32, name: &str, cpu: f32, mem: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            parent_pid: None,
            name: name.to_string(),
            command: format!("/usr/bin/{name}"),
            user: "user".to_string(),
            cpu_percent: cpu,
            memory_bytes: mem,
            memory_percent: 0.0,
            status: "Running".to_string(),
        }
    }

    fn make_snapshot(processes: Vec<ProcessInfo>) -> SystemSnapshot {
        use muxtop_core::system::{CpuSnapshot, LoadSnapshot, MemorySnapshot};
        SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 25.0,
                cores: vec![],
            },
            memory: MemorySnapshot {
                total: 16_000_000_000,
                used: 8_000_000_000,
                available: 8_000_000_000,
                swap_total: 0,
                swap_used: 0,
            },
            load: LoadSnapshot {
                one: 1.0,
                five: 0.8,
                fifteen: 0.5,
                uptime_secs: 3600,
            },
            processes,
            timestamp: Instant::now(),
        }
    }

    // -- Tab tests (STORY-01) --

    #[test]
    fn test_tab_default_is_general() {
        assert_eq!(Tab::default(), Tab::General);
    }

    #[test]
    fn test_tab_next_cycles() {
        assert_eq!(Tab::General.next(), Tab::Processes);
        assert_eq!(Tab::Processes.next(), Tab::General);
    }

    #[test]
    fn test_tab_prev_cycles() {
        assert_eq!(Tab::Processes.prev(), Tab::General);
        assert_eq!(Tab::General.prev(), Tab::Processes);
    }

    #[test]
    fn test_tab_label_values() {
        assert_eq!(Tab::General.label(), "General");
        assert_eq!(Tab::Processes.label(), "Processes");
    }

    #[test]
    fn test_tab_display() {
        assert_eq!(format!("{}", Tab::General), "General");
        assert_eq!(format!("{}", Tab::Processes), "Processes");
    }

    #[test]
    fn test_tab_all_contains_both() {
        assert!(Tab::ALL.contains(&Tab::General));
        assert!(Tab::ALL.contains(&Tab::Processes));
        assert_eq!(Tab::ALL.len(), 2);
    }

    // -- AppState defaults (STORY-02) --

    #[test]
    fn test_app_state_defaults() {
        let app = AppState::new();
        assert_eq!(app.tab, Tab::General);
        assert!(matches!(app.sort_field, SortField::Cpu));
        assert!(matches!(app.sort_order, SortOrder::Desc));
        assert!(app.filter_input.is_empty());
        assert!(!app.filter_active);
        assert!(!app.tree_mode);
        assert_eq!(app.selected, 0);
        assert_eq!(app.scroll_offset, 0);
        assert!(!app.show_palette);
        assert!(app.running());
        assert!(app.last_snapshot.is_none());
    }

    #[test]
    fn test_app_state_running_and_quit() {
        let mut app = AppState::new();
        assert!(app.running());
        app.quit();
        assert!(!app.running());
    }

    #[test]
    fn test_selected_process_none_initially() {
        let app = AppState::new();
        assert!(app.selected_process().is_none());
    }

    #[test]
    fn test_apply_snapshot_populates_visible() {
        let mut app = AppState::new();
        let snap = make_snapshot(vec![
            make_process(1, "firefox", 50.0, 1000),
            make_process(2, "chrome", 30.0, 2000),
        ]);
        app.apply_snapshot(snap);
        assert!(!app.visible_processes.is_empty());
        assert!(app.last_snapshot.is_some());
    }

    #[test]
    fn test_apply_snapshot_sorts_cpu_desc() {
        let mut app = AppState::new();
        app.sort_field = SortField::Cpu;
        app.sort_order = SortOrder::Desc;
        let snap = make_snapshot(vec![
            make_process(1, "low", 10.0, 100),
            make_process(2, "high", 90.0, 200),
            make_process(3, "mid", 50.0, 300),
        ]);
        app.apply_snapshot(snap);
        let cpus: Vec<f32> = app
            .visible_processes
            .iter()
            .map(|p| p.cpu_percent)
            .collect();
        assert_eq!(cpus, vec![90.0, 50.0, 10.0]);
    }

    #[test]
    fn test_apply_snapshot_filters() {
        let mut app = AppState::new();
        app.filter_input = "fire".to_string();
        let snap = make_snapshot(vec![
            make_process(1, "firefox", 50.0, 1000),
            make_process(2, "chrome", 30.0, 2000),
            make_process(3, "firefox-esr", 20.0, 500),
        ]);
        app.apply_snapshot(snap);
        assert_eq!(app.visible_processes.len(), 2);
        assert!(
            app.visible_processes
                .iter()
                .all(|p| p.name.contains("fire"))
        );
    }

    #[test]
    fn test_apply_snapshot_tree_mode() {
        let mut app = AppState::new();
        app.tree_mode = true;
        let snap = make_snapshot(vec![
            make_process(1, "init", 1.0, 100),
            make_process(2, "child", 2.0, 200),
        ]);
        app.apply_snapshot(snap);
        assert!(!app.visible_tree.is_empty());
    }

    #[test]
    fn test_selected_process_after_snapshot() {
        let mut app = AppState::new();
        let snap = make_snapshot(vec![make_process(1, "proc", 10.0, 100)]);
        app.apply_snapshot(snap);
        assert!(app.selected_process().is_some());
    }

    #[test]
    fn test_process_count_flat_vs_tree() {
        let mut app = AppState::new();
        let snap = make_snapshot(vec![
            make_process(1, "a", 10.0, 100),
            make_process(2, "b", 20.0, 200),
        ]);
        app.apply_snapshot(snap);
        let flat_count = app.process_count();
        app.tree_mode = true;
        app.recompute_visible();
        let tree_count = app.process_count();
        // Both should contain the same number of processes
        assert_eq!(flat_count, 2);
        assert_eq!(tree_count, 2);
    }

    #[test]
    fn test_apply_snapshot_clamps_selection() {
        let mut app = AppState::new();
        app.selected = 10; // beyond bounds
        let snap = make_snapshot(vec![
            make_process(1, "a", 10.0, 100),
            make_process(2, "b", 20.0, 200),
        ]);
        app.apply_snapshot(snap);
        assert!(app.selected < app.process_count());
    }

    #[test]
    fn test_app_state_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<AppState>();
    }

    // -- Key handling (STORY-03) --

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    fn app_with_processes() -> AppState {
        let mut app = AppState::new();
        let snap = make_snapshot(vec![
            make_process(1, "alpha", 90.0, 500),
            make_process(2, "bravo", 50.0, 300),
            make_process(3, "charlie", 30.0, 200),
            make_process(4, "delta", 10.0, 100),
            make_process(5, "echo", 70.0, 400),
        ]);
        app.apply_snapshot(snap);
        app
    }

    #[test]
    fn test_quit_q() {
        let mut app = AppState::new();
        app.handle_key_event(key(KeyCode::Char('q')));
        assert!(!app.running());
    }

    #[test]
    fn test_quit_ctrl_c() {
        let mut app = AppState::new();
        app.handle_key_event(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.running());
    }

    #[test]
    fn test_navigate_down() {
        let mut app = app_with_processes();
        assert_eq!(app.selected, 0);
        app.handle_key_event(key(KeyCode::Char('j')));
        assert_eq!(app.selected, 1);
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.selected, 2);
    }

    #[test]
    fn test_navigate_up() {
        let mut app = app_with_processes();
        app.selected = 3;
        app.handle_key_event(key(KeyCode::Char('k')));
        assert_eq!(app.selected, 2);
        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.selected, 1);
    }

    #[test]
    fn test_navigate_home() {
        let mut app = app_with_processes();
        app.selected = 3;
        app.handle_key_event(key(KeyCode::Char('g')));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_navigate_end() {
        let mut app = app_with_processes();
        app.handle_key_event(key(KeyCode::Char('G')));
        assert_eq!(app.selected, app.process_count() - 1);
    }

    #[test]
    fn test_navigate_clamp_bottom() {
        let mut app = app_with_processes();
        app.selected = app.process_count() - 1;
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.selected, app.process_count() - 1);
    }

    #[test]
    fn test_navigate_clamp_top() {
        let mut app = app_with_processes();
        app.selected = 0;
        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_navigate_no_snapshot() {
        let mut app = AppState::new();
        // Must not panic with no processes.
        app.handle_key_event(key(KeyCode::Down));
        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_tab_switch() {
        let mut app = AppState::new();
        assert_eq!(app.tab, Tab::General);
        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.tab, Tab::Processes);
        app.handle_key_event(key(KeyCode::Tab));
        assert_eq!(app.tab, Tab::General);
    }

    #[test]
    fn test_backtab_switch() {
        let mut app = AppState::new();
        app.handle_key_event(key(KeyCode::BackTab));
        assert_eq!(app.tab, Tab::Processes);
    }

    #[test]
    fn test_tree_toggle() {
        let mut app = app_with_processes();
        assert!(!app.tree_mode);
        app.handle_key_event(key(KeyCode::Char('t')));
        assert!(app.tree_mode);
        app.handle_key_event(key(KeyCode::Char('t')));
        assert!(!app.tree_mode);
    }

    #[test]
    fn test_sort_cycle() {
        let mut app = app_with_processes();
        assert!(matches!(app.sort_field, SortField::Cpu));
        app.handle_key_event(key(KeyCode::Char('s')));
        assert!(matches!(app.sort_field, SortField::Mem));
        app.handle_key_event(key(KeyCode::Char('s')));
        assert!(matches!(app.sort_field, SortField::Pid));
    }

    #[test]
    fn test_sort_order_toggle() {
        let mut app = app_with_processes();
        assert!(matches!(app.sort_order, SortOrder::Desc));
        app.handle_key_event(key(KeyCode::Char('S')));
        assert!(matches!(app.sort_order, SortOrder::Asc));
        app.handle_key_event(key(KeyCode::Char('S')));
        assert!(matches!(app.sort_order, SortOrder::Desc));
    }

    #[test]
    fn test_sort_f_keys() {
        let mut app = app_with_processes();
        app.handle_key_event(key(KeyCode::F(1)));
        assert!(matches!(app.sort_field, SortField::Pid));
        app.handle_key_event(key(KeyCode::F(2)));
        assert!(matches!(app.sort_field, SortField::Name));
        app.handle_key_event(key(KeyCode::F(3)));
        assert!(matches!(app.sort_field, SortField::Cpu));
        app.handle_key_event(key(KeyCode::F(4)));
        assert!(matches!(app.sort_field, SortField::Mem));
        app.handle_key_event(key(KeyCode::F(5)));
        assert!(matches!(app.sort_field, SortField::User));
    }

    #[test]
    fn test_filter_enter_exit() {
        let mut app = AppState::new();
        assert!(!app.filter_active);
        app.handle_key_event(key(KeyCode::Char('/')));
        assert!(app.filter_active);
        app.handle_key_event(key(KeyCode::Esc));
        assert!(!app.filter_active);
    }

    #[test]
    fn test_filter_typing() {
        let mut app = AppState::new();
        app.handle_key_event(key(KeyCode::Char('/')));
        assert!(app.filter_active);
        app.handle_key_event(key(KeyCode::Char('f')));
        app.handle_key_event(key(KeyCode::Char('o')));
        app.handle_key_event(key(KeyCode::Char('o')));
        assert_eq!(app.filter_input, "foo");
    }

    #[test]
    fn test_filter_backspace() {
        let mut app = AppState::new();
        app.filter_input = "bar".to_string();
        app.filter_active = true;
        app.handle_key_event(key(KeyCode::Backspace));
        assert_eq!(app.filter_input, "ba");
        app.handle_key_event(key(KeyCode::Backspace));
        assert_eq!(app.filter_input, "b");
    }

    #[test]
    fn test_filter_enter_keeps() {
        let mut app = AppState::new();
        app.filter_input = "test".to_string();
        app.filter_active = true;
        app.handle_key_event(key(KeyCode::Enter));
        assert!(!app.filter_active);
        assert_eq!(app.filter_input, "test"); // kept
    }

    // -- Guard fixes: missing test coverage --

    #[test]
    fn test_ctrl_c_quits_in_filter_mode() {
        let mut app = AppState::new();
        app.filter_active = true;
        app.handle_key_event(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.running());
    }

    // -- Alt+1/Alt+2 tab switching (STORY-08) --

    #[test]
    fn test_tab_alt1_switches_to_general() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        app.handle_key_event(key_mod(KeyCode::Char('1'), KeyModifiers::ALT));
        assert_eq!(app.tab, Tab::General);
    }

    #[test]
    fn test_tab_alt2_switches_to_processes() {
        let mut app = AppState::new();
        app.handle_key_event(key_mod(KeyCode::Char('2'), KeyModifiers::ALT));
        assert_eq!(app.tab, Tab::Processes);
    }

    #[test]
    fn test_tab_alt1_idempotent_on_general() {
        let mut app = AppState::new();
        assert_eq!(app.tab, Tab::General);
        app.handle_key_event(key_mod(KeyCode::Char('1'), KeyModifiers::ALT));
        assert_eq!(app.tab, Tab::General);
    }

    #[test]
    fn test_palette_toggle() {
        let mut app = AppState::new();
        assert!(!app.show_palette);
        app.handle_key_event(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.show_palette);
        app.handle_key_event(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(!app.show_palette);
    }

    // -- Left/Right arrow tab cycling (STORY-09) --

    #[test]
    fn test_tab_right_arrow_next() {
        let mut app = AppState::new();
        assert_eq!(app.tab, Tab::General);
        app.handle_key_event(key(KeyCode::Right));
        assert_eq!(app.tab, Tab::Processes);
    }

    #[test]
    fn test_tab_left_arrow_prev() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::Left));
        assert_eq!(app.tab, Tab::General);
    }

    #[test]
    fn test_tab_left_arrow_wraps() {
        let mut app = AppState::new();
        assert_eq!(app.tab, Tab::General);
        app.handle_key_event(key(KeyCode::Left));
        assert_eq!(app.tab, Tab::Processes);
    }

    #[test]
    fn test_tab_right_arrow_wraps() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::Right));
        assert_eq!(app.tab, Tab::General);
    }

    // -- Mouse handling (STORY-04) --

    fn mouse_scroll(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn test_mouse_scroll_down() {
        let mut app = app_with_processes();
        assert_eq!(app.scroll_offset, 0);
        app.handle_mouse_event(mouse_scroll(MouseEventKind::ScrollDown));
        assert_eq!(app.scroll_offset, 1);
    }

    #[test]
    fn test_mouse_scroll_up() {
        let mut app = app_with_processes();
        app.scroll_offset = 3;
        app.handle_mouse_event(mouse_scroll(MouseEventKind::ScrollUp));
        assert_eq!(app.scroll_offset, 2);
    }

    #[test]
    fn test_mouse_scroll_clamp_min() {
        let mut app = app_with_processes();
        app.scroll_offset = 0;
        app.handle_mouse_event(mouse_scroll(MouseEventKind::ScrollUp));
        assert_eq!(app.scroll_offset, 0);
    }

    #[test]
    fn test_mouse_scroll_clamp_max() {
        let mut app = app_with_processes();
        app.scroll_offset = app.process_count().saturating_sub(1);
        app.handle_mouse_event(mouse_scroll(MouseEventKind::ScrollDown));
        assert_eq!(app.scroll_offset, app.process_count().saturating_sub(1));
    }
}
