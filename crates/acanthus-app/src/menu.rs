//! Placement of menus on `xdg_popup`: the popup configuration for a context menu below an anchor
//! and for a submenu beside its parent row. Both grab input, so the compositor delivers the
//! click-outside dismissal and stacks nested menus.

use acanthus_wayland::{ConstraintAdjustment, PopupConfig, PopupEdge};

/// Padding between a menu's frame and its first row, which a submenu is shifted by so its first row lines up with the parent row.
pub const SUBMENU_OVERLAP: i32 = 4;

/// A context menu of `size` hanging below `anchor` (x, y, width, height in the parent surface), flipping above it or sliding when there is no room.
pub fn context_popup(anchor: (i32, i32, i32, i32), size: (u32, u32)) -> PopupConfig {
    PopupConfig::new(anchor, size).anchor(PopupEdge::BottomLeft).gravity(PopupEdge::BottomRight).grab(true)
}

/// A submenu of `size` beside the row of its parent menu.
///
/// `parent_width` is the parent menu's width and `row_top`, `row_height` locate the opening row inside
/// it; the submenu opens to the right and flips to the left edge when the screen ends.
pub fn submenu_popup(parent_width: i32, row_top: i32, row_height: i32, size: (u32, u32)) -> PopupConfig {
    PopupConfig::new((0, row_top, parent_width, row_height), size)
        .anchor(PopupEdge::TopRight)
        .gravity(PopupEdge::BottomRight)
        .offset(0, -SUBMENU_OVERLAP)
        .adjustment(ConstraintAdjustment::SlideY | ConstraintAdjustment::FlipX)
        .grab(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_context_menu_hangs_below_its_anchor_and_grabs() {
        let config = context_popup((10, 0, 24, 24), (160, 200));
        assert_eq!(config.anchor_rect, (10, 0, 24, 24));
        assert_eq!(config.size, (160, 200));
        assert_eq!(config.anchor, PopupEdge::BottomLeft);
        assert_eq!(config.gravity, PopupEdge::BottomRight);
        assert!(config.grab);
        assert!(config.adjustment.contains(ConstraintAdjustment::FlipY));
    }

    #[test]
    fn a_submenu_opens_beside_the_row_and_flips_sideways() {
        let config = submenu_popup(180, 36, 28, (140, 90));
        assert_eq!(config.anchor_rect, (0, 36, 180, 28));
        assert_eq!(config.anchor, PopupEdge::TopRight);
        assert_eq!(config.offset, (0, -SUBMENU_OVERLAP));
        assert!(config.grab);
        assert!(config.adjustment.contains(ConstraintAdjustment::FlipX));
        assert!(!config.adjustment.contains(ConstraintAdjustment::FlipY));
    }
}
