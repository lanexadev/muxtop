// Responsive column model shared by every table in muxtop.
//
// muxtop 0.4 hardcoded column widths per view, so below ~100 columns the most
// information-dense field (COMMAND, IMAGE) was crushed while fixed-width
// metadata kept its full allocation. Columns now declare a priority and the
// layout drops the least useful ones first — a 60-column tmux pane and a
// 200-column monitor both get a table that makes sense.

use std::borrow::Cow;

use ratatui::text::{Line, Span};

use crate::ui::glyphs::Glyphs;
use crate::ui::theme::Theme;

/// Horizontal alignment of a column's cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

/// How a column claims horizontal space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Width {
    /// Exactly this many cells, including the trailing gap.
    Fixed(u16),
    /// Absorbs whatever is left, never below `min`. At most one flex column
    /// per table; a second one is treated as `Fixed(min)`.
    Flex { min: u16 },
}

/// Priority of a column, i.e. the order in which columns are dropped when the
/// terminal is too narrow.
pub const PRIO_ESSENTIAL: u8 = 0;
pub const PRIO_HIGH: u8 = 1;
pub const PRIO_MEDIUM: u8 = 2;
pub const PRIO_LOW: u8 = 3;

/// A table column: what it is called, how wide it wants to be, and how
/// dispensable it is.
#[derive(Debug, Clone, Copy)]
pub struct Column {
    pub label: &'static str,
    pub width: Width,
    pub align: Align,
    /// Higher values are dropped first. `PRIO_ESSENTIAL` columns are never
    /// dropped, no matter how narrow the terminal gets.
    pub priority: u8,
}

impl Column {
    pub const fn fixed(label: &'static str, width: u16, align: Align, priority: u8) -> Self {
        Self {
            label,
            width: Width::Fixed(width),
            align,
            priority,
        }
    }

    pub const fn flex(label: &'static str, min: u16, priority: u8) -> Self {
        Self {
            label,
            width: Width::Flex { min },
            align: Align::Left,
            priority,
        }
    }
}

/// The columns that survived, in display order, with their resolved widths.
#[derive(Debug, Clone, Default)]
pub struct ColumnLayout {
    /// `(index into the original column slice, resolved width)`.
    visible: Vec<(usize, u16)>,
}

impl ColumnLayout {
    /// Fit `columns` into `width` cells.
    ///
    /// Drops the lowest-priority columns until the fixed part fits, then hands
    /// the remainder to the flex column. Essential columns are kept even when
    /// that means overflowing a terminal too narrow to hold them — an unusable
    /// table is still better than an empty one.
    pub fn resolve(columns: &[Column], width: u16) -> Self {
        let width = width as u32;

        // Drop order: lowest priority first, then right-most first so a table
        // sheds trailing detail before it sheds leading identity.
        let mut droppable: Vec<usize> = (0..columns.len())
            .filter(|&i| columns[i].priority != PRIO_ESSENTIAL)
            .collect();
        droppable.sort_by_key(|&i| (std::cmp::Reverse(columns[i].priority), std::cmp::Reverse(i)));

        let mut dropped = vec![false; columns.len()];
        let mut drop_iter = droppable.into_iter();

        loop {
            let required: u32 = columns
                .iter()
                .enumerate()
                .filter(|(i, _)| !dropped[*i])
                .map(|(_, c)| u32::from(min_width(c)))
                .sum();

            if required <= width {
                break;
            }
            match drop_iter.next() {
                Some(i) => dropped[i] = true,
                None => break, // only essential columns left; let them overflow
            }
        }

        let kept: Vec<usize> = (0..columns.len()).filter(|&i| !dropped[i]).collect();
        let fixed_total: u32 = kept
            .iter()
            .map(|&i| match columns[i].width {
                Width::Fixed(w) => u32::from(w),
                Width::Flex { min } => u32::from(min),
            })
            .sum();

        // The first flex column takes the slack; any further ones stay at min.
        let flex_idx = kept
            .iter()
            .copied()
            .find(|&i| matches!(columns[i].width, Width::Flex { .. }));
        let slack = width.saturating_sub(fixed_total);

        let visible = kept
            .into_iter()
            .map(|i| {
                let w = match columns[i].width {
                    Width::Fixed(w) => u32::from(w),
                    Width::Flex { min } => {
                        if Some(i) == flex_idx {
                            u32::from(min) + slack
                        } else {
                            u32::from(min)
                        }
                    }
                };
                (i, w.min(u32::from(u16::MAX)) as u16)
            })
            .collect();

        Self { visible }
    }

    /// Fit `columns` into `width`, first skipping `skip` non-essential columns
    /// from the left.
    ///
    /// This is what `h` / `l` drive: on a narrow terminal, scrolling right
    /// reveals the columns that the width budget dropped, without ever hiding
    /// the identity column that tells you which row you are looking at.
    pub fn resolve_scrolled(columns: &[Column], width: u16, skip: usize) -> Self {
        if skip == 0 {
            return Self::resolve(columns, width);
        }
        let mut remaining = skip;
        let kept: Vec<Column> = columns
            .iter()
            .copied()
            .filter(|c| {
                if c.priority == PRIO_ESSENTIAL || remaining == 0 {
                    true
                } else {
                    remaining -= 1;
                    false
                }
            })
            .collect();

        // Map the surviving columns back to their original indices, so callers
        // can keep indexing cells by declaration order.
        let mut original = Vec::with_capacity(kept.len());
        let mut remaining = skip;
        for (i, c) in columns.iter().enumerate() {
            if c.priority == PRIO_ESSENTIAL || remaining == 0 {
                original.push(i);
            } else {
                remaining -= 1;
            }
        }

        let inner = Self::resolve(&kept, width);
        Self {
            visible: inner
                .visible
                .into_iter()
                .map(|(i, w)| (original[i], w))
                .collect(),
        }
    }

    /// The surviving columns, as `(original index, width)` pairs.
    pub fn visible(&self) -> &[(usize, u16)] {
        &self.visible
    }

    /// Whether the column at `idx` in the original slice survived.
    pub fn shows(&self, idx: usize) -> bool {
        self.visible.iter().any(|&(i, _)| i == idx)
    }

    /// Total width claimed by the surviving columns.
    pub fn total_width(&self) -> u16 {
        self.visible.iter().map(|&(_, w)| w).sum()
    }

    /// Number of surviving columns.
    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }
}

fn min_width(c: &Column) -> u16 {
    match c.width {
        Width::Fixed(w) => w,
        Width::Flex { min } => min,
    }
}

/// Pad or truncate `text` to exactly `width` cells.
///
/// Columns include their own trailing gap, so a right-aligned cell reserves one
/// cell of padding to keep numbers from touching the next column.
pub fn cell(text: &str, width: u16, align: Align, glyphs: &Glyphs) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    match align {
        Align::Left => {
            let t = glyphs.truncate(text, width);
            let len = t.chars().count();
            let mut s = String::with_capacity(width);
            s.push_str(&t);
            s.extend(std::iter::repeat_n(' ', width.saturating_sub(len)));
            s
        }
        Align::Right => {
            // Reserve one trailing cell so `100.0` does not butt against the
            // next column's first character.
            let inner = width.saturating_sub(1).max(1);
            let t = glyphs.truncate(text, inner);
            let len = t.chars().count();
            let mut s = String::with_capacity(width);
            s.extend(std::iter::repeat_n(' ', inner.saturating_sub(len)));
            s.push_str(&t);
            if width > inner {
                s.push(' ');
            }
            s
        }
    }
}

/// Build a table's column-header row, marking the active sort column.
pub fn header_line(
    columns: &[Column],
    layout: &ColumnLayout,
    sort_col: Option<usize>,
    descending: bool,
    theme: &Theme,
    glyphs: &Glyphs,
) -> Line<'static> {
    let arrow = if descending {
        glyphs.sort_desc
    } else {
        glyphs.sort_asc
    };
    let base = theme.table_header();

    let spans = layout
        .visible()
        .iter()
        .map(|&(idx, width)| {
            let col = &columns[idx];
            let active = sort_col == Some(idx);
            let label: Cow<'static, str> = if active {
                Cow::Owned(format!("{}{arrow}", col.label))
            } else {
                Cow::Borrowed(col.label)
            };
            let style = if active {
                base.fg(theme.accent_primary).add_modifier(
                    ratatui::style::Modifier::BOLD | ratatui::style::Modifier::UNDERLINED,
                )
            } else {
                base
            };
            Span::styled(cell(&label, width, col.align, glyphs), style)
        })
        .collect::<Vec<_>>();

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::ColorSupport;

    const COLS: &[Column] = &[
        Column::fixed("PID", 7, Align::Right, PRIO_ESSENTIAL),
        Column::fixed("USER", 10, Align::Left, PRIO_MEDIUM),
        Column::fixed("CPU%", 7, Align::Right, PRIO_HIGH),
        Column::fixed("TIME", 9, Align::Right, PRIO_LOW),
        Column::flex("COMMAND", 10, PRIO_ESSENTIAL),
    ];

    fn widths(layout: &ColumnLayout) -> Vec<(usize, u16)> {
        layout.visible().to_vec()
    }

    #[test]
    fn wide_terminal_keeps_every_column() {
        let l = ColumnLayout::resolve(COLS, 140);
        assert_eq!(l.len(), COLS.len());
        assert_eq!(l.total_width(), 140, "flex column must absorb the slack");
    }

    #[test]
    fn narrow_terminal_drops_lowest_priority_first() {
        // 7 + 10 + 7 + 9 + 10 = 43 minimum. At 40 the LOW column goes first.
        let l = ColumnLayout::resolve(COLS, 40);
        assert!(!l.shows(3), "TIME (PRIO_LOW) should be dropped first");
        assert!(l.shows(1), "USER should survive before TIME is kept");
    }

    #[test]
    fn very_narrow_terminal_keeps_essential_columns() {
        let l = ColumnLayout::resolve(COLS, 20);
        assert!(l.shows(0), "PID is essential");
        assert!(l.shows(4), "COMMAND is essential");
        assert!(!l.shows(1));
        assert!(!l.shows(2));
        assert!(!l.shows(3));
    }

    #[test]
    fn absurdly_narrow_terminal_does_not_panic_or_empty_the_table() {
        let l = ColumnLayout::resolve(COLS, 1);
        assert!(!l.is_empty(), "essential columns are never all dropped");
        let l = ColumnLayout::resolve(COLS, 0);
        assert!(!l.is_empty());
    }

    #[test]
    fn flex_column_absorbs_the_remainder() {
        let l = ColumnLayout::resolve(COLS, 100);
        let (_, cmd_w) = widths(&l)
            .into_iter()
            .find(|&(i, _)| i == 4)
            .expect("COMMAND survives");
        // 100 - (7 + 10 + 7 + 9) = 67
        assert_eq!(cmd_w, 67);
    }

    #[test]
    fn no_columns_resolves_to_nothing() {
        let l = ColumnLayout::resolve(&[], 80);
        assert!(l.is_empty());
        assert_eq!(l.total_width(), 0);
    }

    // -- cell padding --

    #[test]
    fn cell_pads_left_aligned_to_exact_width() {
        let g = Glyphs::new(true);
        assert_eq!(cell("nginx", 10, Align::Left, &g), "nginx     ");
        assert_eq!(cell("nginx", 10, Align::Left, &g).chars().count(), 10);
    }

    #[test]
    fn cell_right_aligns_with_a_trailing_gap() {
        let g = Glyphs::new(true);
        assert_eq!(cell("42", 7, Align::Right, &g), "    42 ");
        assert_eq!(cell("42", 7, Align::Right, &g).chars().count(), 7);
    }

    #[test]
    fn cell_truncates_with_an_ellipsis() {
        let g = Glyphs::new(true);
        assert_eq!(cell("postgres", 5, Align::Left, &g), "post…");
        assert_eq!(cell("postgres", 5, Align::Left, &g).chars().count(), 5);
    }

    #[test]
    fn cell_is_exact_width_for_every_input() {
        let g = Glyphs::new(true);
        for width in 0u16..20 {
            for text in ["", "a", "nginx", "a-very-long-container-image-name"] {
                for align in [Align::Left, Align::Right] {
                    let c = cell(text, width, align, &g);
                    assert_eq!(
                        c.chars().count(),
                        width as usize,
                        "cell({text:?}, {width}, {align:?}) has the wrong width"
                    );
                }
            }
        }
    }

    #[test]
    fn cell_ascii_mode_stays_ascii() {
        let g = Glyphs::new(false);
        let c = cell("postgres", 5, Align::Left, &g);
        assert!(c.is_ascii(), "ASCII mode leaked a Unicode ellipsis: {c:?}");
    }

    // -- header --

    #[test]
    fn header_marks_the_active_sort_column() {
        let theme = Theme::new(ColorSupport::TrueColor);
        let g = Glyphs::new(true);
        let l = ColumnLayout::resolve(COLS, 140);
        let line = header_line(COLS, &l, Some(2), true, &theme, &g);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("CPU%▼"), "active column needs a sort arrow");
        assert!(!text.contains("PID▼"), "inactive columns must stay bare");
    }

    #[test]
    fn header_uses_ascii_arrows_without_unicode() {
        let theme = Theme::new(ColorSupport::TrueColor);
        let g = Glyphs::new(false);
        let l = ColumnLayout::resolve(COLS, 140);
        let line = header_line(COLS, &l, Some(2), false, &theme, &g);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("CPU%^"));
        assert!(text.is_ascii());
    }

    #[test]
    fn header_only_renders_surviving_columns() {
        let theme = Theme::new(ColorSupport::TrueColor);
        let g = Glyphs::new(true);
        let l = ColumnLayout::resolve(COLS, 20);
        let line = header_line(COLS, &l, None, true, &theme, &g);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(!text.contains("TIME"));
        assert!(text.contains("PID"));
    }
}
