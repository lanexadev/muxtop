use std::time::Duration;

use crossterm::event::{self, Event as CrosstermEvent, KeyEvent, MouseEvent};
use tokio::sync::mpsc;

use crate::error::TuiError;
use muxtop_core::system::SystemSnapshot;

/// Tick rate for the event loop (~60Hz).
pub const TICK_RATE: Duration = Duration::from_millis(16);

/// Application events.
//
// `large_enum_variant`: same trade-off as `WireMessage::Snapshot` — the hot
// path emits a snapshot 1 Hz; boxing would mean a heap allocation per
// emit and per consumer poll, which v0.3.1 perf work showed as material.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Event {
    /// No input event within the tick window.
    Tick,
    /// Keyboard event.
    Key(KeyEvent),
    /// Mouse event.
    Mouse(MouseEvent),
    /// Terminal resize (width, height).
    Resize(u16, u16),
    /// New system snapshot from the collector.
    Snapshot(SystemSnapshot),
}

/// Polls crossterm events and mpsc channel for system snapshots.
pub struct EventHandler {
    rx: mpsc::Receiver<SystemSnapshot>,
}

impl EventHandler {
    pub fn new(rx: mpsc::Receiver<SystemSnapshot>) -> Self {
        Self { rx }
    }

    /// Poll for the next event. Non-blocking — returns Tick if nothing happens within TICK_RATE.
    pub fn poll_event(&mut self) -> Result<Event, TuiError> {
        // G-04: Drain ALL pending snapshots, keeping only the latest.
        // Prevents snapshot starvation and ensures the TUI always shows fresh data.
        let mut latest_snapshot = None;
        while let Ok(snapshot) = self.rx.try_recv() {
            latest_snapshot = Some(snapshot);
        }
        if let Some(snapshot) = latest_snapshot {
            return Ok(Event::Snapshot(snapshot));
        }

        // Poll crossterm for keyboard/mouse/resize events
        if event::poll(TICK_RATE)? {
            match event::read()? {
                CrosstermEvent::Key(key) => Ok(Event::Key(key)),
                CrosstermEvent::Mouse(mouse) => Ok(Event::Mouse(mouse)),
                CrosstermEvent::Resize(w, h) => Ok(Event::Resize(w, h)),
                _ => Ok(Event::Tick),
            }
        } else {
            Ok(Event::Tick)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn test_event_variants_constructible() {
        let tick = Event::Tick;
        assert!(!format!("{tick:?}").is_empty());

        let key = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!format!("{key:?}").is_empty());

        let mouse = Event::Mouse(MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
        assert!(!format!("{mouse:?}").is_empty());

        let resize = Event::Resize(80, 24);
        assert!(!format!("{resize:?}").is_empty());

        // Snapshot variant requires a SystemSnapshot
        use muxtop_core::network::NetworkSnapshot;
        use muxtop_core::system::{CpuSnapshot, LoadSnapshot, MemorySnapshot};
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
                interfaces: vec![],
                total_rx: 0,
                total_tx: 0,
            },
            containers: None,
            kube: None,
            timestamp_ms: 0,
        };
        let snapshot_event = Event::Snapshot(snap);
        assert!(!format!("{snapshot_event:?}").is_empty());
    }

    #[test]
    fn test_event_handler_new() {
        let (_tx, rx) = mpsc::channel::<SystemSnapshot>(1);
        let _handler = EventHandler::new(rx);
        // Construction should not panic.
    }

    #[test]
    fn test_tick_rate_is_16ms() {
        assert_eq!(TICK_RATE, Duration::from_millis(16));
    }
}
