// The table.
//
// Processes, Network, Containers and Kubernetes each hand-rolled their own
// header row, virtualised body, zebra striping, selection styling and scroll
// arithmetic. Four copies meant four behaviours; the mouse-wheel bug existed in
// exactly one of them because the other three had no wheel handling at all.
//
// Views now describe their columns and hand back cell contents. Everything
// else — layout, truncation, striping, the selection marker, the scrollbar,
// the empty state — happens here, once.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Color;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::ui::Render;

use super::columns::{self, Column, ColumnLayout};
use super::{empty::EmptyState, scrollbar, viewport_offset};

/// One cell of a row.
pub struct Cell {
    pub text: String,
    /// Overrides the row's foreground — for a state marker or a hot value.
    pub color: Option<Color>,
}

impl Cell {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: None,
        }
    }

    pub fn colored(text: impl Into<String>, color: Color) -> Self {
        Self {
            text: text.into(),
            color: Some(color),
        }
    }
}

/// A row: one cell per column, in the order the columns were declared.
pub struct Row {
    pub cells: Vec<Cell>,
}

impl Row {
    pub fn new(cells: Vec<Cell>) -> Self {
        Self { cells }
    }
}

/// Everything the table needs that is not a cell.
pub struct Spec<'a> {
    pub columns: &'a [Column],
    /// Index into `columns` of the active sort column.
    pub sort_col: Option<usize>,
    pub descending: bool,
    pub total: usize,
    pub selected: usize,
    pub scroll: usize,
    /// Horizontal column scroll (`h` / `l`).
    pub col_scroll: usize,
    /// Shown instead of the body when `total` is zero.
    pub empty: EmptyState<'a>,
}

/// Draw a table into `area`.
///
/// Returns the scroll offset actually used, so the caller can store it and keep
/// the viewport stable across frames.
pub fn draw<F>(
    frame: &mut Frame,
    area: Rect,
    r: &Render<'_>,
    spec: &Spec<'_>,
    mut row_at: F,
) -> usize
where
    F: FnMut(usize) -> Row,
{
    if area.height == 0 || area.width == 0 {
        return spec.scroll;
    }

    // Reserve the rightmost column for the scrollbar, but only when there is
    // something to scroll and room to show it.
    let needs_scrollbar = area.width > 4 && spec.total > area.height.saturating_sub(1) as usize;
    let (table_area, bar_area) = if needs_scrollbar {
        let [t, b] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        (t, Some(b))
    } else {
        (area, None)
    };

    // One column of the table is the selection marker.
    let marker_w: u16 = 1;
    let content_w = table_area.width.saturating_sub(marker_w);
    let layout = ColumnLayout::resolve_scrolled(spec.columns, content_w, spec.col_scroll);

    let [header_area, body_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(table_area);

    // -- header --
    let mut header_spans = vec![Span::styled(" ", r.theme.table_header())];
    header_spans.extend(
        columns::header_line(
            spec.columns,
            &layout,
            spec.sort_col,
            spec.descending,
            r.theme,
            r.glyphs,
        )
        .spans,
    );
    frame.render_widget(
        Paragraph::new(Line::from(header_spans)).style(r.theme.table_header()),
        header_area,
    );

    if body_area.height == 0 {
        return spec.scroll;
    }

    // -- empty state --
    if spec.total == 0 {
        super::empty::render(frame, body_area, &spec.empty, r.theme);
        return 0;
    }

    // -- body --
    let height = body_area.height as usize;
    let scroll = viewport_offset(spec.selected, spec.scroll, height);
    let end = (scroll + height).min(spec.total);

    let lines: Vec<Line<'static>> = (scroll..end)
        .map(|idx| {
            let selected = idx == spec.selected;
            let base = if selected {
                r.theme.selected_row()
            } else {
                r.theme.row(idx % 2 == 1)
            };
            let row = row_at(idx);

            let mut spans = Vec::with_capacity(layout.len() + 1);
            spans.push(Span::styled(
                if selected {
                    r.glyphs.sel_edge.to_string()
                } else {
                    " ".to_string()
                },
                if selected {
                    base.fg(r.theme.selection_edge)
                } else {
                    base
                },
            ));
            for &(col_idx, width) in layout.visible() {
                let column = &spec.columns[col_idx];
                let cell = row.cells.get(col_idx);
                let text = cell.map_or("", |c| c.text.as_str());
                let style = match cell.and_then(|c| c.color) {
                    // A per-cell colour survives selection: the state marker is
                    // exactly what you still want to see on the current row.
                    Some(color) => base.fg(color),
                    None => base,
                };
                spans.push(Span::styled(
                    columns::cell(text, width, column.align, r.glyphs),
                    style,
                ));
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), body_area);

    if let Some(bar) = bar_area {
        let bar = Rect {
            y: body_area.y,
            height: body_area.height,
            ..bar
        };
        scrollbar::vertical(frame, bar, spec.total, height, scroll, r.theme, r.glyphs);
    }

    scroll
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;
    use crate::terminal::ColorSupport;
    use crate::ui::glyphs::Glyphs;
    use crate::ui::theme::Theme;
    use crate::ui::widgets::columns::{Align, PRIO_ESSENTIAL, PRIO_LOW, PRIO_MEDIUM};
    use ratatui::{Terminal, backend::TestBackend};

    const COLS: &[Column] = &[
        Column::fixed("PID", 7, Align::Right, PRIO_ESSENTIAL),
        Column::fixed("USER", 10, Align::Left, PRIO_MEDIUM),
        Column::fixed("CPU%", 7, Align::Right, PRIO_LOW),
        Column::flex("COMMAND", 10, PRIO_ESSENTIAL),
    ];

    struct Fixture {
        app: AppState,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                app: AppState::new(),
            }
        }

        fn render(
            &mut self,
            w: u16,
            h: u16,
            total: usize,
            selected: usize,
            scroll: usize,
            unicode: bool,
        ) -> Vec<String> {
            let theme = Theme::new(if unicode {
                ColorSupport::TrueColor
            } else {
                ColorSupport::Basic
            });
            let glyphs = Glyphs::new(unicode);
            let r = Render {
                app: &self.app,
                theme: &theme,
                glyphs: &glyphs,
                breakpoint: crate::terminal::Breakpoint::from_width(w),
            };
            let spec = Spec {
                columns: COLS,
                sort_col: Some(2),
                descending: true,
                total,
                selected,
                scroll,
                col_scroll: 0,
                empty: EmptyState::empty("Nothing here", Some("Really nothing.")),
            };
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|f| {
                    draw(f, Rect::new(0, 0, w, h), &r, &spec, |i| {
                        Row::new(vec![
                            Cell::new(format!("{}", 1000 + i)),
                            Cell::new("lucas"),
                            Cell::new(format!("{}.0", i % 100)),
                            Cell::new(format!("/usr/bin/process-number-{i}")),
                        ])
                    });
                })
                .unwrap();
            let buf = terminal.backend().buffer().clone();
            (0..h)
                .map(|y| {
                    (0..w)
                        .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                        .collect::<String>()
                })
                .collect()
        }
    }

    #[test]
    fn header_and_rows_render() {
        let mut f = Fixture::new();
        let rows = f.render(80, 10, 50, 0, 0, true);
        assert!(rows[0].contains("PID"));
        assert!(rows[0].contains("COMMAND"));
        assert!(rows[1].contains("1000"));
        assert!(rows[2].contains("1001"));
    }

    #[test]
    fn active_sort_column_is_marked() {
        let mut f = Fixture::new();
        let rows = f.render(80, 10, 50, 0, 0, true);
        assert!(rows[0].contains("CPU%▼"));
    }

    #[test]
    fn selected_row_carries_a_marker() {
        // The one selection cue that survives a terminal with no usable
        // background colours.
        let mut f = Fixture::new();
        let rows = f.render(80, 10, 50, 2, 0, true);
        assert!(rows[3].starts_with('▎'), "row 2 should be marked: {rows:?}");
        assert!(!rows[1].starts_with('▎'));
    }

    #[test]
    fn body_is_virtualised() {
        // 100k rows must cost the same as ten.
        let mut f = Fixture::new();
        let rows = f.render(80, 10, 100_000, 0, 0, true);
        assert!(rows[1].contains("1000"));
        assert_eq!(rows.len(), 10);
    }

    #[test]
    fn scrolling_shows_the_right_window() {
        let mut f = Fixture::new();
        let rows = f.render(80, 10, 500, 300, 300, true);
        assert!(rows[1].contains("1300"), "window did not follow: {rows:?}");
    }

    #[test]
    fn scrollbar_appears_only_when_needed() {
        let mut f = Fixture::new();
        let long = f.render(80, 10, 500, 0, 0, true);
        assert!(long.iter().any(|r| r.contains('█') || r.contains('│')));
        let short = f.render(80, 10, 3, 0, 0, true);
        assert!(
            !short.iter().skip(1).any(|r| r.contains('│')),
            "no scrollbar when everything fits: {short:?}"
        );
    }

    #[test]
    fn empty_table_explains_itself() {
        let mut f = Fixture::new();
        let rows = f.render(80, 10, 0, 0, 0, true);
        let text = rows.join("\n");
        assert!(text.contains("Nothing here"));
        assert!(text.contains("Really nothing"));
        // The header stays: the columns are information even with no rows.
        assert!(rows[0].contains("PID"));
    }

    #[test]
    fn narrow_terminals_drop_the_least_useful_column() {
        let mut f = Fixture::new();
        let rows = f.render(30, 10, 50, 0, 0, true);
        assert!(rows[0].contains("PID"));
        assert!(
            !rows[0].contains("CPU%"),
            "the low-priority column should go first: {}",
            rows[0]
        );
        assert!(
            rows[0].contains("COMMAND"),
            "the identity column must survive: {}",
            rows[0]
        );
    }

    #[test]
    fn ascii_mode_emits_no_multibyte_characters() {
        let mut f = Fixture::new();
        let rows = f.render(80, 10, 500, 3, 0, false);
        let text = rows.join("\n");
        assert!(text.is_ascii(), "ASCII table leaked Unicode:\n{text}");
    }

    #[test]
    fn every_row_is_exactly_the_table_width() {
        let mut f = Fixture::new();
        for w in [20u16, 40, 80, 200] {
            let rows = f.render(w, 8, 50, 0, 0, true);
            for (i, row) in rows.iter().enumerate() {
                assert_eq!(
                    row.chars().count(),
                    w as usize,
                    "row {i} drifted at width {w}"
                );
            }
        }
    }

    #[test]
    fn degenerate_geometry_does_not_panic() {
        let mut f = Fixture::new();
        for (w, h) in [(1u16, 1u16), (2, 1), (1, 10), (5, 2), (80, 1)] {
            let _ = f.render(w, h, 50, 0, 0, true);
            let _ = f.render(w, h, 0, 0, 0, true);
        }
    }

    #[test]
    fn selection_beyond_the_end_does_not_panic() {
        let mut f = Fixture::new();
        let _ = f.render(80, 10, 5, 999, 999, true);
    }
}
