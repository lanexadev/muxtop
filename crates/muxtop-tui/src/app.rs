// Application state machine.

use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use nucleo::pattern::{Atom, AtomKind, CaseMatching, Normalization};
use nucleo::{Config, Matcher, Utf32Str};

use muxtop_core::actions;
use muxtop_core::process::{
    ProcessInfo, SortField, SortOrder, build_process_tree, filter_processes, flatten_tree,
    sort_processes,
};
use muxtop_core::system::SystemSnapshot;

use crate::CliConfig;
use crate::terminal::TermCaps;

// ---------------------------------------------------------------------------
// Confirm dialog
// ---------------------------------------------------------------------------

/// A pending action that requires user confirmation (y/n).
#[derive(Debug, Clone)]
pub enum ConfirmAction {
    /// Kill a process with the given signal.
    Kill { pid: u32, name: String, signal: i32 },
    /// Change the nice value of a process.
    Renice { pid: u32, name: String, delta: i32 },
}

impl ConfirmAction {
    /// Human-readable description for the confirmation dialog.
    pub fn prompt(&self) -> String {
        match self {
            ConfirmAction::Kill { pid, name, signal } => {
                let sig_name = if *signal == libc::SIGKILL {
                    "SIGKILL"
                } else {
                    "SIGTERM"
                };
                format!("Send {sig_name} to {name} (PID {pid})?  [y/n]")
            }
            ConfirmAction::Renice { pid, name, delta } => {
                let direction = if *delta > 0 {
                    "lower priority (+1)"
                } else {
                    "higher priority (-1)"
                };
                format!("Renice {name} (PID {pid}) to {direction}?  [y/n]")
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Command registry
// ---------------------------------------------------------------------------

/// All available commands in the palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Quit,
    ToggleTreeView,
    SortByCpu,
    SortByMem,
    SortByPid,
    SortByName,
    SortByUser,
    ToggleSortOrder,
    CycleSort,
    SwitchToGeneral,
    SwitchToProcesses,
    OpenFilter,
    NextTab,
    PrevTab,
    KillProcess,
    ForceKillProcess,
    NiceDown,
    NiceUp,
    ClearFilter,
}

impl Command {
    pub const ALL: &[Command] = &[
        Command::Quit,
        Command::ToggleTreeView,
        Command::SortByCpu,
        Command::SortByMem,
        Command::SortByPid,
        Command::SortByName,
        Command::SortByUser,
        Command::ToggleSortOrder,
        Command::CycleSort,
        Command::SwitchToGeneral,
        Command::SwitchToProcesses,
        Command::OpenFilter,
        Command::NextTab,
        Command::PrevTab,
        Command::KillProcess,
        Command::ForceKillProcess,
        Command::NiceDown,
        Command::NiceUp,
        Command::ClearFilter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Command::Quit => "Quit",
            Command::ToggleTreeView => "Toggle tree view",
            Command::SortByCpu => "Sort by CPU",
            Command::SortByMem => "Sort by Memory",
            Command::SortByPid => "Sort by PID",
            Command::SortByName => "Sort by Name",
            Command::SortByUser => "Sort by User",
            Command::ToggleSortOrder => "Toggle sort order",
            Command::CycleSort => "Cycle sort field",
            Command::SwitchToGeneral => "Switch to General tab",
            Command::SwitchToProcesses => "Switch to Processes tab",
            Command::OpenFilter => "Open filter",
            Command::NextTab => "Next tab",
            Command::PrevTab => "Previous tab",
            Command::KillProcess => "Kill process (SIGTERM)",
            Command::ForceKillProcess => "Force kill (SIGKILL)",
            Command::NiceDown => "Lower priority (+1)",
            Command::NiceUp => "Raise priority (-1)",
            Command::ClearFilter => "Clear filter",
        }
    }

    pub fn shortcut(self) -> &'static str {
        match self {
            Command::Quit => "q",
            Command::ToggleTreeView => "t",
            Command::SortByCpu => "F3",
            Command::SortByMem => "F4",
            Command::SortByPid => "F1",
            Command::SortByName => "F2",
            Command::SortByUser => "F5",
            Command::ToggleSortOrder => "S",
            Command::CycleSort => "s",
            Command::SwitchToGeneral => "Alt+1",
            Command::SwitchToProcesses => "Alt+2",
            Command::OpenFilter => "/",
            Command::NextTab => "Tab",
            Command::PrevTab => "Shift+Tab",
            Command::KillProcess => "F9",
            Command::ForceKillProcess => "F10",
            Command::NiceDown => "F7",
            Command::NiceUp => "F8",
            Command::ClearFilter => "Esc",
        }
    }

    /// The search haystack: label + shortcut combined for better fuzzy matching.
    fn search_text(self) -> String {
        format!("{} {}", self.label(), self.shortcut())
    }
}

// ---------------------------------------------------------------------------
// Palette state
// ---------------------------------------------------------------------------

/// State for the command palette overlay.
pub struct PaletteState {
    pub input: String,
    pub selected: usize,
    /// Filtered commands with match scores (higher = better).
    pub filtered: Vec<(Command, Option<u16>)>,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self::new()
    }
}

impl PaletteState {
    pub fn new() -> Self {
        let filtered = Command::ALL.iter().map(|&cmd| (cmd, None)).collect();
        Self {
            input: String::new(),
            selected: 0,
            filtered,
        }
    }

    /// Recompute filtered results using nucleo fuzzy matching.
    pub fn refilter(&mut self) {
        if self.input.is_empty() {
            self.filtered = Command::ALL.iter().map(|&cmd| (cmd, None)).collect();
        } else {
            let mut matcher = Matcher::new(Config::DEFAULT);
            let atom = Atom::new(
                &self.input,
                CaseMatching::Ignore,
                Normalization::Smart,
                AtomKind::Fuzzy,
                false,
            );

            let mut scored: Vec<(Command, u16)> = Command::ALL
                .iter()
                .filter_map(|&cmd| {
                    let haystack = cmd.search_text();
                    let mut buf = Vec::new();
                    let haystack_utf32 = Utf32Str::new(&haystack, &mut buf);
                    atom.score(haystack_utf32, &mut matcher)
                        .map(|score| (cmd, score))
                })
                .collect();

            scored.sort_by(|a, b| b.1.cmp(&a.1));
            self.filtered = scored.into_iter().map(|(cmd, s)| (cmd, Some(s))).collect();
        }

        // Clamp selection
        if self.filtered.is_empty() {
            self.selected = 0;
        } else {
            self.selected = self.selected.min(self.filtered.len() - 1);
        }
    }
}

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

/// Duration before a status message auto-clears.
const STATUS_TIMEOUT_SECS: u64 = 5;

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
    pub palette: PaletteState,
    /// Pending confirm dialog (kill/renice).
    pub confirm: Option<ConfirmAction>,
    /// Status message with creation time (auto-clears after timeout).
    pub status_message: Option<(String, Instant)>,
    /// Detected terminal capabilities.
    pub term_caps: TermCaps,
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
            palette: PaletteState::new(),
            confirm: None,
            status_message: None,
            term_caps: TermCaps::default(),
            running: true,
            last_snapshot: None,
            visible_processes: Vec::new(),
            visible_tree: Vec::new(),
        }
    }

    /// Create AppState from CLI configuration and detected terminal capabilities.
    pub fn with_config(config: CliConfig, term_caps: TermCaps) -> Self {
        Self {
            sort_field: config.sort_field,
            tree_mode: config.tree_mode,
            filter_input: config.filter.unwrap_or_default(),
            term_caps,
            ..Self::new()
        }
    }

    /// Returns the current status message if it hasn't expired.
    pub fn active_status(&self) -> Option<&str> {
        self.status_message.as_ref().and_then(|(msg, created)| {
            if created.elapsed().as_secs() < STATUS_TIMEOUT_SECS {
                Some(msg.as_str())
            } else {
                None
            }
        })
    }

    /// Set a status message (auto-expires after STATUS_TIMEOUT_SECS).
    fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    /// Clear expired status messages.
    fn clear_status_if_expired(&mut self) {
        if let Some((_, created)) = &self.status_message
            && created.elapsed().as_secs() >= STATUS_TIMEOUT_SECS
        {
            self.status_message = None;
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
        // Clear expired status on any key press.
        self.clear_status_if_expired();

        // Confirm dialog captures y/n/Esc only.
        if self.confirm.is_some() {
            self.handle_confirm_key(key);
            return;
        }

        // Palette mode captures most keys.
        if self.show_palette {
            self.handle_palette_key(key);
            return;
        }

        // Filter mode captures most keys as text input.
        if self.filter_active {
            self.handle_filter_key(key);
            return;
        }

        // Clear status on any normal key press.
        self.status_message = None;

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
            KeyCode::Char('S') | KeyCode::Char('I') => {
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

            // Kill process — F9 (Processes tab only)
            KeyCode::F(9) => {
                if self.tab == Tab::Processes {
                    self.request_kill(libc::SIGTERM);
                }
            }

            // Force kill — F10 (Processes tab only)
            KeyCode::F(10) => {
                if self.tab == Tab::Processes {
                    self.request_kill(libc::SIGKILL);
                }
            }

            // Renice — F7 lower priority (+1), F8 higher priority (-1)
            KeyCode::F(7) => {
                if self.tab == Tab::Processes {
                    self.request_renice(1);
                }
            }
            KeyCode::F(8) => {
                if self.tab == Tab::Processes {
                    self.request_renice(-1);
                }
            }

            // Clear filter with Esc (when not in filter input mode)
            KeyCode::Esc => {
                if !self.filter_input.is_empty() {
                    self.filter_input.clear();
                    self.recompute_visible();
                }
            }

            // Filter mode
            KeyCode::Char('/') => {
                self.filter_active = true;
            }

            // Command palette toggle
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.open_palette();
            }

            _ => {}
        }
    }

    /// Request to kill the currently selected process (opens confirm dialog).
    fn request_kill(&mut self, signal: i32) {
        if let Some(proc) = self.selected_process() {
            self.confirm = Some(ConfirmAction::Kill {
                pid: proc.pid,
                name: proc.name.clone(),
                signal,
            });
        }
    }

    /// Request to renice the currently selected process (opens confirm dialog).
    fn request_renice(&mut self, delta: i32) {
        if let Some(proc) = self.selected_process() {
            self.confirm = Some(ConfirmAction::Renice {
                pid: proc.pid,
                name: proc.name.clone(),
                delta,
            });
        }
    }

    /// Handle keys while the confirm dialog is open.
    fn handle_confirm_key(&mut self, key: KeyEvent) {
        // Ctrl+C always quits.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit();
            return;
        }

        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(action) = self.confirm.take() {
                    self.execute_confirm(action);
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.confirm = None;
            }
            _ => {} // Block all other keys
        }
    }

    /// Execute a confirmed action and set status message.
    fn execute_confirm(&mut self, action: ConfirmAction) {
        match action {
            ConfirmAction::Kill { pid, name, signal } => {
                let sig_name = if signal == libc::SIGKILL {
                    "SIGKILL"
                } else {
                    "SIGTERM"
                };
                match actions::kill_process(pid, signal) {
                    Ok(()) => self.set_status(format!("Sent {sig_name} to {name} (PID {pid})")),
                    Err(e) => self.set_status(format!("Kill failed: {e}")),
                }
            }
            ConfirmAction::Renice { pid, name, delta } => {
                // We don't know the current nice value, so we read it first
                // via libc::getpriority, then apply the delta.
                let current = unsafe {
                    // Clear errno before getpriority (it can return -1 legitimately).
                    #[cfg(target_os = "macos")]
                    {
                        *libc::__error() = 0;
                    }
                    #[cfg(target_os = "linux")]
                    {
                        *libc::__errno_location() = 0;
                    }
                    libc::getpriority(libc::PRIO_PROCESS, pid as libc::id_t)
                };
                let new_nice = current + delta;
                match actions::renice_process(pid, new_nice) {
                    Ok(()) => {
                        self.set_status(format!("Reniced {name} (PID {pid}) to nice={new_nice}"))
                    }
                    Err(e) => self.set_status(format!("Renice failed: {e}")),
                }
            }
        }
    }

    /// Open the command palette with a fresh state.
    fn open_palette(&mut self) {
        self.show_palette = true;
        self.palette = PaletteState::new();
    }

    /// Close the command palette and clear state.
    fn close_palette(&mut self) {
        self.show_palette = false;
        self.palette.input.clear();
        self.palette.selected = 0;
    }

    /// Handle keys while the command palette is open.
    fn handle_palette_key(&mut self, key: KeyEvent) {
        // Ctrl+C always quits.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit();
            return;
        }

        match key.code {
            KeyCode::Esc => {
                self.close_palette();
            }
            // Ctrl+P also closes the palette (toggle).
            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.close_palette();
            }
            KeyCode::Enter => {
                if let Some(&(cmd, _)) = self.palette.filtered.get(self.palette.selected) {
                    self.close_palette();
                    self.execute_command(cmd);
                }
            }
            KeyCode::Down => {
                if !self.palette.filtered.is_empty() {
                    self.palette.selected =
                        (self.palette.selected + 1).min(self.palette.filtered.len() - 1);
                }
            }
            KeyCode::Up => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
            }
            KeyCode::Backspace => {
                self.palette.input.pop();
                self.palette.refilter();
            }
            KeyCode::Char(c) => {
                self.palette.input.push(c);
                self.palette.refilter();
            }
            _ => {}
        }
    }

    /// Execute a command from the palette.
    fn execute_command(&mut self, cmd: Command) {
        match cmd {
            Command::Quit => self.quit(),
            Command::ToggleTreeView => {
                self.tree_mode = !self.tree_mode;
                self.selected = 0;
                self.scroll_offset = 0;
                self.recompute_visible();
            }
            Command::SortByCpu => {
                self.sort_field = SortField::Cpu;
                self.recompute_visible();
            }
            Command::SortByMem => {
                self.sort_field = SortField::Mem;
                self.recompute_visible();
            }
            Command::SortByPid => {
                self.sort_field = SortField::Pid;
                self.recompute_visible();
            }
            Command::SortByName => {
                self.sort_field = SortField::Name;
                self.recompute_visible();
            }
            Command::SortByUser => {
                self.sort_field = SortField::User;
                self.recompute_visible();
            }
            Command::ToggleSortOrder => {
                self.sort_order = match self.sort_order {
                    SortOrder::Asc => SortOrder::Desc,
                    SortOrder::Desc => SortOrder::Asc,
                };
                self.recompute_visible();
            }
            Command::CycleSort => {
                self.sort_field = next_sort_field(self.sort_field);
                self.recompute_visible();
            }
            Command::SwitchToGeneral => {
                self.tab = Tab::General;
            }
            Command::SwitchToProcesses => {
                self.tab = Tab::Processes;
            }
            Command::OpenFilter => {
                self.filter_active = true;
            }
            Command::NextTab => {
                self.tab = self.tab.next();
            }
            Command::PrevTab => {
                self.tab = self.tab.prev();
            }
            Command::KillProcess => {
                if self.tab == Tab::Processes {
                    self.request_kill(libc::SIGTERM);
                }
            }
            Command::ForceKillProcess => {
                if self.tab == Tab::Processes {
                    self.request_kill(libc::SIGKILL);
                }
            }
            Command::NiceDown => {
                if self.tab == Tab::Processes {
                    self.request_renice(1);
                }
            }
            Command::NiceUp => {
                if self.tab == Tab::Processes {
                    self.request_renice(-1);
                }
            }
            Command::ClearFilter => {
                self.filter_input.clear();
                self.recompute_visible();
            }
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

    // -- Command registry tests (Epic 6) --

    #[test]
    fn test_command_registry_count() {
        assert!(
            Command::ALL.len() >= 14,
            "Registry should have at least 14 commands, got {}",
            Command::ALL.len()
        );
    }

    #[test]
    fn test_command_labels_non_empty() {
        for cmd in Command::ALL {
            assert!(!cmd.label().is_empty(), "Command {:?} has empty label", cmd);
        }
    }

    #[test]
    fn test_command_shortcuts_non_empty() {
        for cmd in Command::ALL {
            assert!(
                !cmd.shortcut().is_empty(),
                "Command {:?} has empty shortcut",
                cmd
            );
        }
    }

    // -- Palette state tests (Epic 6) --

    #[test]
    fn test_palette_state_new() {
        let ps = PaletteState::new();
        assert!(ps.input.is_empty());
        assert_eq!(ps.selected, 0);
        assert_eq!(ps.filtered.len(), Command::ALL.len());
    }

    #[test]
    fn test_palette_refilter_empty_shows_all() {
        let mut ps = PaletteState::new();
        ps.refilter();
        assert_eq!(ps.filtered.len(), Command::ALL.len());
    }

    #[test]
    fn test_palette_refilter_fuzzy_match() {
        let mut ps = PaletteState::new();
        ps.input = "sort cpu".to_string();
        ps.refilter();
        assert!(!ps.filtered.is_empty(), "Should match at least one command");
        assert_eq!(
            ps.filtered[0].0,
            Command::SortByCpu,
            "First result should be Sort by CPU"
        );
    }

    #[test]
    fn test_palette_refilter_no_match() {
        let mut ps = PaletteState::new();
        ps.input = "zzzzznonexistent".to_string();
        ps.refilter();
        assert!(ps.filtered.is_empty(), "Should have no matches");
    }

    #[test]
    fn test_palette_refilter_clamps_selection() {
        let mut ps = PaletteState::new();
        ps.selected = 100;
        ps.input = "quit".to_string();
        ps.refilter();
        assert!(ps.selected < ps.filtered.len());
    }

    // -- Palette key handling tests (Epic 6) --

    #[test]
    fn test_palette_opens_with_ctrl_p() {
        let mut app = AppState::new();
        assert!(!app.show_palette);
        app.handle_key_event(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(app.show_palette);
        assert!(app.palette.input.is_empty());
    }

    #[test]
    fn test_palette_closes_with_esc() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.input = "test".to_string();
        app.handle_key_event(key(KeyCode::Esc));
        assert!(!app.show_palette);
        assert!(app.palette.input.is_empty());
    }

    #[test]
    fn test_palette_closes_with_ctrl_p() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.handle_key_event(key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(!app.show_palette);
    }

    #[test]
    fn test_palette_typing_captures_input() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.handle_key_event(key(KeyCode::Char('s')));
        app.handle_key_event(key(KeyCode::Char('o')));
        app.handle_key_event(key(KeyCode::Char('r')));
        app.handle_key_event(key(KeyCode::Char('t')));
        assert_eq!(app.palette.input, "sort");
    }

    #[test]
    fn test_palette_backspace() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.input = "sor".to_string();
        app.handle_key_event(key(KeyCode::Backspace));
        assert_eq!(app.palette.input, "so");
    }

    #[test]
    fn test_palette_blocks_quit() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.handle_key_event(key(KeyCode::Char('q')));
        assert!(app.running(), "Pressing 'q' in palette should NOT quit");
        assert_eq!(app.palette.input, "q", "Should type 'q' into palette");
    }

    #[test]
    fn test_palette_ctrl_c_quits() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.handle_key_event(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.running(), "Ctrl+C should quit even with palette open");
    }

    #[test]
    fn test_palette_navigate_down() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        assert_eq!(app.palette.selected, 0);
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.palette.selected, 1);
    }

    #[test]
    fn test_palette_navigate_up() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        app.palette.selected = 3;
        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.palette.selected, 2);
    }

    #[test]
    fn test_palette_navigate_clamp_top() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.selected = 0;
        app.handle_key_event(key(KeyCode::Up));
        assert_eq!(app.palette.selected, 0);
    }

    #[test]
    fn test_palette_navigate_clamp_bottom() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        let max = app.palette.filtered.len() - 1;
        app.palette.selected = max;
        app.handle_key_event(key(KeyCode::Down));
        assert_eq!(app.palette.selected, max);
    }

    // -- Command execution tests (Epic 6) --

    #[test]
    fn test_palette_execute_quit() {
        let mut app = AppState::new();
        app.execute_command(Command::Quit);
        assert!(!app.running());
    }

    #[test]
    fn test_palette_execute_toggle_tree() {
        let mut app = app_with_processes();
        assert!(!app.tree_mode);
        app.execute_command(Command::ToggleTreeView);
        assert!(app.tree_mode);
    }

    #[test]
    fn test_palette_execute_sort_cpu() {
        let mut app = app_with_processes();
        app.sort_field = SortField::Pid;
        app.execute_command(Command::SortByCpu);
        assert!(matches!(app.sort_field, SortField::Cpu));
    }

    #[test]
    fn test_palette_execute_sort_mem() {
        let mut app = app_with_processes();
        app.execute_command(Command::SortByMem);
        assert!(matches!(app.sort_field, SortField::Mem));
    }

    #[test]
    fn test_palette_execute_toggle_sort_order() {
        let mut app = app_with_processes();
        assert!(matches!(app.sort_order, SortOrder::Desc));
        app.execute_command(Command::ToggleSortOrder);
        assert!(matches!(app.sort_order, SortOrder::Asc));
    }

    #[test]
    fn test_palette_execute_switch_tab() {
        let mut app = AppState::new();
        app.execute_command(Command::SwitchToProcesses);
        assert_eq!(app.tab, Tab::Processes);
        app.execute_command(Command::SwitchToGeneral);
        assert_eq!(app.tab, Tab::General);
    }

    #[test]
    fn test_palette_execute_open_filter() {
        let mut app = AppState::new();
        app.execute_command(Command::OpenFilter);
        assert!(app.filter_active);
    }

    #[test]
    fn test_palette_enter_executes_and_closes() {
        let mut app = AppState::new();
        app.show_palette = true;
        app.palette.refilter();
        // First command in the list is Quit
        app.palette.selected = 0;
        assert_eq!(app.palette.filtered[0].0, Command::Quit);
        app.handle_key_event(key(KeyCode::Enter));
        assert!(!app.show_palette);
        assert!(!app.running());
    }

    // -- Epic 7: Confirm dialog tests --

    #[test]
    fn test_confirm_dialog_opens_on_f9() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(9)));
        assert!(app.confirm.is_some());
        if let Some(ConfirmAction::Kill { signal, .. }) = &app.confirm {
            assert_eq!(*signal, libc::SIGTERM);
        } else {
            panic!("Expected Kill confirm action");
        }
    }

    #[test]
    fn test_confirm_dialog_opens_on_f10_sigkill() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(10)));
        assert!(app.confirm.is_some());
        if let Some(ConfirmAction::Kill { signal, .. }) = &app.confirm {
            assert_eq!(*signal, libc::SIGKILL);
        } else {
            panic!("Expected Kill confirm with SIGKILL");
        }
    }

    #[test]
    fn test_confirm_cancel_with_n() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(9)));
        assert!(app.confirm.is_some());
        app.handle_key_event(key(KeyCode::Char('n')));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn test_confirm_cancel_with_esc() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(9)));
        assert!(app.confirm.is_some());
        app.handle_key_event(key(KeyCode::Esc));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn test_confirm_blocks_other_keys() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(9)));
        assert!(app.confirm.is_some());
        // Pressing 'q' should NOT quit
        app.handle_key_event(key(KeyCode::Char('q')));
        assert!(app.running());
        assert!(app.confirm.is_some());
    }

    #[test]
    fn test_confirm_ctrl_c_quits() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(9)));
        assert!(app.confirm.is_some());
        app.handle_key_event(key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.running());
    }

    #[test]
    fn test_kill_no_op_on_general_tab() {
        let mut app = app_with_processes();
        app.tab = Tab::General;
        app.handle_key_event(key(KeyCode::F(9)));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn test_kill_no_op_without_process() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        // No snapshot → no selected process
        app.handle_key_event(key(KeyCode::F(9)));
        assert!(app.confirm.is_none());
    }

    #[test]
    fn test_renice_opens_confirm_on_f7() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(7)));
        assert!(app.confirm.is_some());
        if let Some(ConfirmAction::Renice { delta, .. }) = &app.confirm {
            assert_eq!(*delta, 1);
        } else {
            panic!("Expected Renice confirm action");
        }
    }

    #[test]
    fn test_renice_opens_confirm_on_f8() {
        let mut app = app_with_processes();
        app.tab = Tab::Processes;
        app.handle_key_event(key(KeyCode::F(8)));
        assert!(app.confirm.is_some());
        if let Some(ConfirmAction::Renice { delta, .. }) = &app.confirm {
            assert_eq!(*delta, -1);
        } else {
            panic!("Expected Renice confirm action with delta -1");
        }
    }

    // -- Epic 7: Clear filter & reverse sort --

    #[test]
    fn test_esc_clears_filter() {
        let mut app = app_with_processes();
        app.filter_input = "firefox".to_string();
        app.recompute_visible();
        let before = app.visible_processes.len();
        app.handle_key_event(key(KeyCode::Esc));
        assert!(app.filter_input.is_empty());
        assert!(app.visible_processes.len() >= before);
    }

    #[test]
    fn test_esc_no_op_when_filter_empty() {
        let mut app = app_with_processes();
        assert!(app.filter_input.is_empty());
        let count = app.process_count();
        app.handle_key_event(key(KeyCode::Esc));
        assert_eq!(app.process_count(), count);
    }

    #[test]
    fn test_i_reverses_sort_order() {
        let mut app = app_with_processes();
        assert!(matches!(app.sort_order, SortOrder::Desc));
        app.handle_key_event(key(KeyCode::Char('I')));
        assert!(matches!(app.sort_order, SortOrder::Asc));
    }

    // -- Epic 7: Status message --

    #[test]
    fn test_status_message_set_and_read() {
        let mut app = AppState::new();
        app.set_status("Test message".to_string());
        assert_eq!(app.active_status(), Some("Test message"));
    }

    #[test]
    fn test_status_clears_on_key_press() {
        let mut app = app_with_processes();
        app.set_status("Test message".to_string());
        assert!(app.status_message.is_some());
        // Any normal key press should clear status
        app.handle_key_event(key(KeyCode::Down));
        assert!(app.status_message.is_none());
    }

    // -- Epic 7: Command registry expanded --

    #[test]
    fn test_command_registry_has_new_commands() {
        assert!(
            Command::ALL.len() >= 19,
            "Registry should have at least 19 commands, got {}",
            Command::ALL.len()
        );
        assert!(Command::ALL.contains(&Command::KillProcess));
        assert!(Command::ALL.contains(&Command::ForceKillProcess));
        assert!(Command::ALL.contains(&Command::NiceDown));
        assert!(Command::ALL.contains(&Command::NiceUp));
        assert!(Command::ALL.contains(&Command::ClearFilter));
    }

    #[test]
    fn test_palette_execute_clear_filter() {
        let mut app = app_with_processes();
        app.filter_input = "test".to_string();
        app.execute_command(Command::ClearFilter);
        assert!(app.filter_input.is_empty());
    }

    // -- Epic 9: ConfirmAction::prompt() tests --

    #[test]
    fn test_confirm_prompt_kill_sigterm() {
        let action = ConfirmAction::Kill {
            pid: 1234,
            name: "firefox".to_string(),
            signal: libc::SIGTERM,
        };
        let prompt = action.prompt();
        assert!(prompt.contains("SIGTERM"), "Should contain SIGTERM");
        assert!(prompt.contains("firefox"), "Should contain process name");
        assert!(prompt.contains("1234"), "Should contain PID");
        assert!(prompt.contains("[y/n]"), "Should contain y/n prompt");
    }

    #[test]
    fn test_confirm_prompt_kill_sigkill() {
        let action = ConfirmAction::Kill {
            pid: 999,
            name: "chrome".to_string(),
            signal: libc::SIGKILL,
        };
        let prompt = action.prompt();
        assert!(prompt.contains("SIGKILL"), "Should contain SIGKILL");
        assert!(prompt.contains("chrome"), "Should contain process name");
        assert!(prompt.contains("999"), "Should contain PID");
    }

    #[test]
    fn test_confirm_prompt_renice_down() {
        let action = ConfirmAction::Renice {
            pid: 42,
            name: "bash".to_string(),
            delta: 1,
        };
        let prompt = action.prompt();
        assert!(
            prompt.contains("lower priority"),
            "delta>0 should say 'lower priority'"
        );
        assert!(prompt.contains("bash"), "Should contain process name");
        assert!(prompt.contains("42"), "Should contain PID");
    }

    #[test]
    fn test_confirm_prompt_renice_up() {
        let action = ConfirmAction::Renice {
            pid: 77,
            name: "vim".to_string(),
            delta: -1,
        };
        let prompt = action.prompt();
        assert!(
            prompt.contains("higher priority"),
            "delta<0 should say 'higher priority'"
        );
        assert!(prompt.contains("vim"), "Should contain process name");
    }

    // -- Epic 9: next_sort_field() tests --

    #[test]
    fn test_next_sort_field_full_cycle() {
        let mut field = SortField::Cpu;
        field = next_sort_field(field);
        assert!(matches!(field, SortField::Mem));
        field = next_sort_field(field);
        assert!(matches!(field, SortField::Pid));
        field = next_sort_field(field);
        assert!(matches!(field, SortField::Name));
        field = next_sort_field(field);
        assert!(matches!(field, SortField::User));
        field = next_sort_field(field);
        assert!(matches!(field, SortField::Cpu), "Should cycle back to Cpu");
    }

    #[test]
    fn test_next_sort_field_is_deterministic() {
        // Same input always gives same output
        assert!(matches!(next_sort_field(SortField::Cpu), SortField::Mem));
        assert!(matches!(next_sort_field(SortField::Cpu), SortField::Mem));
    }

    // -- Epic 9: with_config tests --

    #[test]
    fn test_with_config_sort_field() {
        let config = crate::CliConfig {
            sort_field: SortField::Name,
            ..Default::default()
        };
        let app = AppState::with_config(config, TermCaps::default());
        assert!(matches!(app.sort_field, SortField::Name));
    }

    #[test]
    fn test_with_config_tree_mode() {
        let config = crate::CliConfig {
            tree_mode: true,
            ..Default::default()
        };
        let app = AppState::with_config(config, TermCaps::default());
        assert!(app.tree_mode);
    }

    #[test]
    fn test_with_config_filter() {
        let config = crate::CliConfig {
            filter: Some("rust".to_string()),
            ..Default::default()
        };
        let app = AppState::with_config(config, TermCaps::default());
        assert_eq!(app.filter_input, "rust");
    }

    #[test]
    fn test_with_config_no_filter() {
        let config = crate::CliConfig::default();
        let app = AppState::with_config(config, TermCaps::default());
        assert!(app.filter_input.is_empty());
    }

    // -- Epic 9: AppState edge cases --

    #[test]
    fn test_apply_empty_snapshot() {
        let mut app = AppState::new();
        let snap = make_snapshot(vec![]);
        app.apply_snapshot(snap);
        assert!(app.visible_processes.is_empty());
        assert_eq!(app.process_count(), 0);
        assert!(app.selected_process().is_none());
    }

    #[test]
    fn test_page_down_navigation() {
        let mut app = app_with_processes();
        app.handle_key_event(key(KeyCode::PageDown));
        // 5 processes, PageDown moves by 20, clamped to 4
        assert_eq!(app.selected, 4);
    }

    #[test]
    fn test_page_up_from_bottom() {
        let mut app = app_with_processes();
        app.selected = 4;
        app.handle_key_event(key(KeyCode::PageUp));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_home_key_navigation() {
        let mut app = app_with_processes();
        app.selected = 3;
        app.handle_key_event(key(KeyCode::Home));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn test_end_key_navigation() {
        let mut app = app_with_processes();
        app.handle_key_event(key(KeyCode::End));
        assert_eq!(app.selected, app.process_count() - 1);
    }
}
