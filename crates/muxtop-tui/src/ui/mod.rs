// Layout & rendering for the TUI.

mod confirm;
mod containers;
mod general;
mod kube;
mod network;
mod palette;
mod processes;
pub mod sanitize;
pub mod theme;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Tabs},
};

use crate::ConnectionMode;
use crate::app::{AppState, Tab};
use theme::Theme;

/// Labels for future tabs (not yet implemented).
const FUTURE_TABS: &[&str] = &["GPU [soon]"];

/// Render the full application layout: Header, TabBar, Content, Footer.
pub fn draw_root(frame: &mut Frame, app: &AppState) {
    let theme = Theme::new(app.term_caps.color_support);

    let [header_area, tabbar_area, content_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(2),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, header_area, app, &theme);
    draw_tabbar(frame, tabbar_area, app, &theme);
    draw_content(frame, content_area, app, &theme);
    draw_footer(frame, footer_area, app, &theme);

    // Confirm dialog overlay.
    if app.confirm.is_some() {
        confirm::draw_confirm(frame, app, &theme);
    }

    // Command palette overlay (renders on top of everything).
    if app.show_palette {
        palette::draw_palette(frame, app, &theme);
    }
}

/// Render the header line: app name, version, and optional remote indicator.
fn draw_header(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let version = env!("CARGO_PKG_VERSION");
    let mut spans = vec![
        Span::styled(
            " muxtop ",
            Style::default()
                .bg(theme.accent_primary)
                .fg(theme.bg)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" v{version} "),
            Style::default().bg(theme.header_bg).fg(theme.fg),
        ),
    ];

    if let ConnectionMode::Remote {
        ref hostname,
        ref addr,
    } = app.connection_mode
    {
        spans.push(Span::styled(
            format!(" → remote:{hostname}:{} ", addr.port()),
            Style::default()
                .bg(theme.header_bg)
                .fg(theme.accent_secondary),
        ));
    }

    let header = Paragraph::new(Line::from(spans));
    frame.render_widget(header, area);
}

/// Render the tab bar with active highlight and future tabs.
fn draw_tabbar(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    let active_idx = Tab::ALL.iter().position(|&t| t == app.tab).unwrap_or(0);

    let mut titles: Vec<Line<'_>> = Tab::ALL.iter().map(|t| Line::from(t.label())).collect();

    for &future in FUTURE_TABS {
        titles.push(Line::styled(future, Style::default().fg(theme.text_dim)));
    }

    let tabs = Tabs::new(titles)
        .select(active_idx)
        .highlight_style(
            Style::default()
                .fg(theme.accent_primary)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().fg(theme.text_dim))
        .divider(" | ")
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_type(ratatui::widgets::BorderType::Rounded),
        );

    frame.render_widget(tabs, area);
}

/// Render the content area based on the active tab.
fn draw_content(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    match app.tab {
        Tab::General => general::draw_general_tab(frame, area, app, theme),
        Tab::Processes => processes::draw_processes_tab(frame, area, app, theme),
        Tab::Network => network::draw_network_tab(frame, area, app, theme),
        Tab::Containers => containers::draw_containers_tab(frame, area, app, theme),
        Tab::Kube => kube::draw_kube_tab(frame, area, app, theme),
    }
}

/// Render the footer with context-aware shortcut hints or a status message.
fn draw_footer(frame: &mut Frame, area: Rect, app: &AppState, theme: &Theme) {
    // Status message takes priority over shortcuts.
    if let Some(status) = app.active_status() {
        let style = if status.contains("failed") || status.contains("denied") {
            Style::default().fg(theme.bg).bg(theme.danger)
        } else {
            Style::default().fg(theme.bg).bg(theme.success)
        };
        let footer = Paragraph::new(Line::from(Span::styled(format!(" {status} "), style)));
        frame.render_widget(footer, area);
        return;
    }

    let shortcuts = match app.tab {
        Tab::General => vec![
            key_hint("q", "Quit", theme),
            Span::raw(" "),
            key_hint("Tab", "Switch", theme),
            Span::raw(" "),
            key_hint("/", "Filter", theme),
            Span::raw(" "),
            key_hint("t", "Tree", theme),
            Span::raw(" "),
            key_hint("^P", "Palette", theme),
        ],
        Tab::Processes => {
            let mut hints = vec![
                key_hint("q", "Quit", theme),
                Span::raw(" "),
                key_hint("/", "Filter", theme),
                Span::raw(" "),
                key_hint("s", "Sort", theme),
                Span::raw(" "),
                key_hint("t", "Tree", theme),
            ];
            // Hide kill/renice hints in remote mode.
            if !app.is_remote() {
                hints.push(Span::raw(" "));
                hints.push(key_hint("F9", "Kill", theme));
                hints.push(Span::raw(" "));
                hints.push(key_hint("F7/F8", "Nice", theme));
            }
            hints.push(Span::raw(" "));
            hints.push(key_hint("^P", "Palette", theme));
            hints
        }
        Tab::Network => vec![
            key_hint("q", "Quit", theme),
            Span::raw(" "),
            key_hint("j/k", "Select", theme),
            Span::raw(" "),
            key_hint("/", "Filter", theme),
            Span::raw(" "),
            key_hint("s", "Sort", theme),
            Span::raw(" "),
            key_hint("^P", "Palette", theme),
        ],
        Tab::Containers => {
            let mut hints = vec![
                key_hint("q", "Quit", theme),
                Span::raw(" "),
                key_hint("j/k", "Select", theme),
                Span::raw(" "),
                key_hint("/", "Filter", theme),
                Span::raw(" "),
                key_hint("s", "Sort", theme),
            ];
            if !app.is_remote() {
                hints.push(Span::raw(" "));
                hints.push(key_hint("F9", "Stop", theme));
                hints.push(Span::raw(" "));
                hints.push(key_hint("F10", "Kill", theme));
                hints.push(Span::raw(" "));
                hints.push(key_hint("F11", "Restart", theme));
            }
            hints.push(Span::raw(" "));
            hints.push(key_hint("^P", "Palette", theme));
            hints
        }
        Tab::Kube => vec![
            key_hint("q", "Quit", theme),
            Span::raw(" "),
            key_hint("Tab", "Switch", theme),
            Span::raw(" "),
            key_hint("^P", "Palette", theme),
        ],
    };
    let footer = Paragraph::new(Line::from(shortcuts)).style(Style::default().bg(theme.header_bg));
    frame.render_widget(footer, area);
}

/// Create a styled key hint: bold key + dim description.
fn key_hint<'a>(key: &'a str, desc: &'a str, theme: &Theme) -> Span<'a> {
    Span::styled(
        format!(" {key} {desc} "),
        Style::default().fg(theme.fg).bg(theme.selection_bg),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn render_with(app: &AppState, width: u16, height: u16) -> ratatui::buffer::Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_root(frame, app)).unwrap();
        terminal.backend().buffer().clone()
    }

    fn buffer_line_text(buf: &ratatui::buffer::Buffer, row: u16) -> String {
        let width = buf.area.width;
        (0..width)
            .map(|col| buf.cell((col, row)).map(|c| c.symbol()).unwrap_or(" "))
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    fn buffer_contains(buf: &ratatui::buffer::Buffer, needle: &str) -> bool {
        let height = buf.area.height;
        (0..height).any(|row| buffer_line_text(buf, row).contains(needle))
    }

    // -- STORY-02: Layout zones --

    #[test]
    fn test_draw_root_is_callable() {
        let app = AppState::new();
        let _buf = render_with(&app, 80, 24);
    }

    #[test]
    fn test_layout_zones_correct_heights() {
        let app = AppState::new();
        let buf = render_with(&app, 80, 24);
        // Header = row 0, TabBar = rows 1-2, Content = rows 3-22, Footer = row 23
        // Verify header row is non-empty
        let header_text = buffer_line_text(&buf, 0);
        assert!(!header_text.is_empty(), "Header should not be empty");
        // Verify footer row is non-empty
        let footer_text = buffer_line_text(&buf, 23);
        assert!(!footer_text.is_empty(), "Footer should not be empty");
    }

    #[test]
    fn test_layout_resize_reflows() {
        let app = AppState::new();
        // Small terminal
        let buf_small = render_with(&app, 80, 24);
        assert_eq!(buf_small.area.height, 24);
        // Large terminal
        let buf_large = render_with(&app, 120, 40);
        assert_eq!(buf_large.area.height, 40);
        // Header and footer should still be in first/last rows
        let header_small = buffer_line_text(&buf_small, 0);
        let header_large = buffer_line_text(&buf_large, 0);
        assert!(header_small.contains("muxtop"));
        assert!(header_large.contains("muxtop"));
    }

    #[test]
    fn test_layout_minimal_terminal_no_panic() {
        let app = AppState::new();
        // Only 4 rows — content area would be 0 rows
        let _buf = render_with(&app, 80, 4);
        // Just verify no panic
    }

    // Guard G-08: extreme terminal sizes
    #[test]
    fn test_layout_extreme_sizes_no_panic() {
        let app = AppState::new();
        let _buf = render_with(&app, 1, 1);
        let _buf = render_with(&app, 80, 2);
        let _buf = render_with(&app, 10, 5);
    }

    // -- STORY-03: Header --

    #[test]
    fn test_header_renders_name_and_version() {
        let app = AppState::new();
        let buf = render_with(&app, 80, 24);
        let header = buffer_line_text(&buf, 0);
        assert!(header.contains("muxtop"), "Header should contain 'muxtop'");
        assert!(
            header.contains(&format!("v{}", env!("CARGO_PKG_VERSION"))),
            "Header should contain version"
        );
    }

    // -- STORY-04: TabBar --

    #[test]
    fn test_tabbar_renders_tab_names() {
        let app = AppState::new();
        let buf = render_with(&app, 80, 24);
        // Tab names should appear in rows 1-2 (tabbar area)
        let tabbar_text = format!(
            "{} {}",
            buffer_line_text(&buf, 1),
            buffer_line_text(&buf, 2)
        );
        assert!(
            tabbar_text.contains("General"),
            "TabBar should show General"
        );
        assert!(
            tabbar_text.contains("Processes"),
            "TabBar should show Processes"
        );
    }

    #[test]
    fn test_tabbar_active_tab_has_teal_style() {
        let app = AppState::new(); // default: General
        let buf = render_with(&app, 80, 24);
        // Find "General" in the tabbar row and check its style
        let row = 1; // first row of tabbar (tab titles rendered here)
        let line = buffer_line_text(&buf, row);
        if let Some(start) = line.find('G') {
            let cell = buf.cell((start as u16, row)).unwrap();
            // The FG color should match accent_primary
            let theme = theme::Theme::new(crate::terminal::ColorSupport::TrueColor);
            assert_eq!(
                cell.fg, theme.accent_primary,
                "Active tab 'General' should have accent foreground"
            );
        }
    }

    #[test]
    fn test_tabbar_inactive_tab_no_teal() {
        let app = AppState::new(); // default: General active
        let buf = render_with(&app, 80, 24);
        let row = 1;
        let line = buffer_line_text(&buf, row);
        // Find "Processes" — it should NOT be accent_primary
        if let Some(start) = line.find('P') {
            let cell = buf.cell((start as u16, row)).unwrap();
            let theme = theme::Theme::new(crate::terminal::ColorSupport::TrueColor);
            assert_ne!(
                cell.fg, theme.accent_primary,
                "Inactive tab 'Processes' should NOT have accent foreground"
            );
        }
    }

    #[test]
    fn test_tabbar_future_tabs_shown_grayed() {
        let app = AppState::new();
        let buf = render_with(&app, 120, 24); // wider to fit all tabs
        assert!(
            buffer_contains(&buf, "[soon]"),
            "TabBar should show '[soon]' for future tabs"
        );
    }

    // -- STORY-05: Content stubs --

    #[test]
    fn test_content_dispatches_by_tab() {
        use muxtop_core::process::ProcessInfo;
        use muxtop_core::system::*;

        // Provide a snapshot so tabs render distinct content (not both "Waiting for data...")
        let snap = SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 25.0,
                cores: vec![CoreSnapshot {
                    name: "cpu0".to_string(),
                    usage: 25.0,
                    frequency: 3600,
                }],
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
            processes: vec![ProcessInfo {
                pid: 1,
                parent_pid: None,
                name: "proc".to_string(),
                command: "/usr/bin/proc".to_string(),
                user: "user".to_string(),
                cpu_percent: 10.0,
                memory_bytes: 1000,
                memory_percent: 1.0,
                status: "Running".to_string(),
            }],
            networks: muxtop_core::network::NetworkSnapshot {
                interfaces: vec![],
                total_rx: 0,
                total_tx: 0,
            },
            containers: None,
            kube: None,
            timestamp_ms: 0,
        };

        let mut app = AppState::new();
        app.apply_snapshot(snap);

        app.tab = Tab::General;
        let buf_general = render_with(&app, 80, 24);

        app.tab = Tab::Processes;
        let buf_processes = render_with(&app, 80, 24);

        // Content area rows 3..22 should differ
        let general_content = buffer_line_text(&buf_general, 3);
        let processes_content = buffer_line_text(&buf_processes, 3);
        assert_ne!(
            general_content, processes_content,
            "Content should differ between tabs"
        );
    }

    // -- STORY-06: Footer --

    #[test]
    fn test_footer_renders_general_shortcuts() {
        let mut app = AppState::new();
        app.tab = Tab::General;
        let buf = render_with(&app, 80, 24);
        let footer = buffer_line_text(&buf, 23);
        assert!(
            footer.contains("Quit"),
            "General footer should contain Quit hint"
        );
    }

    #[test]
    fn test_footer_renders_processes_shortcuts() {
        let mut app = AppState::new();
        app.tab = Tab::Processes;
        let buf = render_with(&app, 80, 24);
        let footer = buffer_line_text(&buf, 23);
        assert!(
            footer.contains("Sort"),
            "Processes footer should contain Sort hint"
        );
        assert!(
            footer.contains("Tree"),
            "Processes footer should contain Tree hint"
        );
    }

    // -- Network tab tests (Epic 12) --

    #[test]
    fn test_tabbar_shows_network() {
        let app = AppState::new();
        let buf = render_with(&app, 120, 24);
        let tabbar_text = format!(
            "{} {}",
            buffer_line_text(&buf, 1),
            buffer_line_text(&buf, 2)
        );
        assert!(
            tabbar_text.contains("Network"),
            "TabBar should show Network"
        );
    }

    #[test]
    fn test_network_tab_waiting_for_data() {
        let mut app = AppState::new();
        app.tab = Tab::Network;
        let buf = render_with(&app, 80, 24);
        assert!(
            buffer_contains(&buf, "Waiting for data"),
            "Network tab should show waiting message when no snapshot"
        );
    }

    #[test]
    fn test_network_tab_renders_no_panic() {
        let mut app = AppState::new();
        app.tab = Tab::Network;
        // No snapshot — should not panic
        let _buf = render_with(&app, 80, 24);
        let _buf = render_with(&app, 1, 1);
        let _buf = render_with(&app, 10, 5);
    }

    #[test]
    fn test_network_tab_with_interfaces() {
        use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
        use muxtop_core::system::*;

        let snap = SystemSnapshot {
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
            processes: vec![],
            networks: NetworkSnapshot {
                interfaces: vec![
                    NetworkInterfaceSnapshot {
                        name: "eth0".to_string(),
                        bytes_rx: 1_000_000,
                        bytes_tx: 500_000,
                        packets_rx: 1000,
                        packets_tx: 500,
                        errors_rx: 0,
                        errors_tx: 0,
                        mac_address: "00:11:22:33:44:55".to_string(),
                        is_up: true,
                    },
                    NetworkInterfaceSnapshot {
                        name: "lo".to_string(),
                        bytes_rx: 100,
                        bytes_tx: 100,
                        packets_rx: 10,
                        packets_tx: 10,
                        errors_rx: 0,
                        errors_tx: 0,
                        mac_address: "00:00:00:00:00:00".to_string(),
                        is_up: true,
                    },
                ],
                total_rx: 1_000_100,
                total_tx: 500_100,
            },
            containers: None,
            kube: None,
            timestamp_ms: 0,
        };

        let mut app = AppState::new();
        app.tab = Tab::Network;
        app.apply_snapshot(snap);
        let buf = render_with(&app, 100, 24);
        assert!(
            buffer_contains(&buf, "eth0"),
            "Network tab should show eth0 interface"
        );
        assert!(
            buffer_contains(&buf, "lo"),
            "Network tab should show lo interface"
        );
    }

    #[test]
    fn test_network_footer_shows_hints() {
        let mut app = AppState::new();
        app.tab = Tab::Network;
        let buf = render_with(&app, 80, 24);
        let footer = buffer_line_text(&buf, 23);
        assert!(
            footer.contains("Quit"),
            "Network footer should contain Quit"
        );
        assert!(
            footer.contains("Sort"),
            "Network footer should contain Sort"
        );
        assert!(
            footer.contains("Filter"),
            "Network footer should contain Filter"
        );
    }

    #[test]
    fn test_network_tab_summary_bar() {
        use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
        use muxtop_core::system::*;

        let snap = SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 0.0,
                cores: vec![],
            },
            memory: MemorySnapshot {
                total: 0,
                used: 0,
                available: 0,
                swap_total: 0,
                swap_used: 0,
            },
            load: LoadSnapshot {
                one: 0.0,
                five: 0.0,
                fifteen: 0.0,
                uptime_secs: 0,
            },
            processes: vec![],
            networks: NetworkSnapshot {
                interfaces: vec![NetworkInterfaceSnapshot {
                    name: "eth0".to_string(),
                    bytes_rx: 1000,
                    bytes_tx: 500,
                    packets_rx: 10,
                    packets_tx: 5,
                    errors_rx: 0,
                    errors_tx: 0,
                    mac_address: "00:00:00:00:00:00".to_string(),
                    is_up: true,
                }],
                total_rx: 1000,
                total_tx: 500,
            },
            containers: None,
            kube: None,
            timestamp_ms: 0,
        };

        let mut app = AppState::new();
        app.tab = Tab::Network;
        app.apply_snapshot(snap);
        let buf = render_with(&app, 100, 24);
        assert!(
            buffer_contains(&buf, "Total"),
            "Network tab should show summary bar with Total"
        );
        assert!(
            buffer_contains(&buf, "Interfaces"),
            "Network tab should show interface count"
        );
    }

    #[test]
    fn test_network_tab_shows_header_columns() {
        use muxtop_core::network::{NetworkInterfaceSnapshot, NetworkSnapshot};
        use muxtop_core::system::*;

        let snap = SystemSnapshot {
            cpu: CpuSnapshot {
                global_usage: 0.0,
                cores: vec![],
            },
            memory: MemorySnapshot {
                total: 0,
                used: 0,
                available: 0,
                swap_total: 0,
                swap_used: 0,
            },
            load: LoadSnapshot {
                one: 0.0,
                five: 0.0,
                fifteen: 0.0,
                uptime_secs: 0,
            },
            processes: vec![],
            networks: NetworkSnapshot {
                interfaces: vec![NetworkInterfaceSnapshot {
                    name: "eth0".to_string(),
                    bytes_rx: 0,
                    bytes_tx: 0,
                    packets_rx: 0,
                    packets_tx: 0,
                    errors_rx: 0,
                    errors_tx: 0,
                    mac_address: "00:00:00:00:00:00".to_string(),
                    is_up: true,
                }],
                total_rx: 0,
                total_tx: 0,
            },
            containers: None,
            kube: None,
            timestamp_ms: 0,
        };

        let mut app = AppState::new();
        app.tab = Tab::Network;
        app.apply_snapshot(snap);
        let buf = render_with(&app, 100, 24);
        assert!(
            buffer_contains(&buf, "INTERFACE"),
            "Should show INTERFACE header"
        );
        assert!(buffer_contains(&buf, "RX/s"), "Should show RX/s header");
        assert!(buffer_contains(&buf, "TX/s"), "Should show TX/s header");
    }
}
