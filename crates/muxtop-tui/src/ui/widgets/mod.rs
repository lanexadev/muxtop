// Shared widget layer.
//
// Before this module every tab hand-rolled its own column header, scroll maths,
// zebra striping, selection styling, filter bar and empty state. Four copies
// meant four subtly different behaviours and four places to fix every bug.
// Views now describe *what* they show (columns and cell contents); everything
// about *how* a table behaves lives here.

pub mod badge;
pub mod columns;
pub mod empty;
pub mod meter;
pub mod overlay;
pub mod panel;
pub mod scrollbar;
pub mod spark;
pub mod table;

pub use columns::{Align, Column, ColumnLayout, Width};
pub use table::{Cell, Row, Spec};

/// Compute the scroll offset that keeps `selected` on screen.
///
/// Returns the offset unchanged when the selection is already visible, which is
/// what lets an explicit scroll (mouse wheel, page keys) survive: the caller is
/// responsible for moving `selected` along with the offset when it wants the
/// view to stay put. muxtop 0.4 moved the offset alone, so the next frame
/// snapped straight back to the selection and the wheel appeared dead.
pub fn viewport_offset(selected: usize, offset: usize, height: usize) -> usize {
    if height == 0 {
        return 0;
    }
    if selected < offset {
        selected
    } else if selected >= offset + height {
        selected.saturating_sub(height - 1)
    } else {
        offset
    }
}

/// Clamp a selection index to a list that may have shrunk under it.
pub fn clamp_selection(selected: usize, count: usize) -> usize {
    if count == 0 {
        0
    } else {
        selected.min(count - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_keeps_selection_visible() {
        // Selection above the window pulls the window up to it.
        assert_eq!(viewport_offset(3, 10, 20), 3);
        // Selection below the window pulls the window down to it.
        assert_eq!(viewport_offset(30, 0, 20), 11);
        // Selection already inside the window leaves the offset alone, so an
        // explicit scroll survives.
        assert_eq!(viewport_offset(5, 2, 20), 2);
    }

    #[test]
    fn viewport_handles_zero_height() {
        assert_eq!(viewport_offset(42, 7, 0), 0);
    }

    #[test]
    fn viewport_handles_height_one() {
        assert_eq!(viewport_offset(0, 0, 1), 0);
        assert_eq!(viewport_offset(5, 0, 1), 5);
    }

    #[test]
    fn clamp_selection_on_empty_and_shrunk_lists() {
        assert_eq!(clamp_selection(9, 0), 0);
        assert_eq!(clamp_selection(9, 3), 2);
        assert_eq!(clamp_selection(1, 3), 1);
    }
}
