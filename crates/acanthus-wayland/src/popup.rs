use acanthus_core::Color;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity,
};

/// Edge or corner of the anchor rectangle or of the popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupEdge {
    /// Centred.
    #[default]
    None,
    /// Top edge.
    Top,
    /// Bottom edge.
    Bottom,
    /// Left edge.
    Left,
    /// Right edge.
    Right,
    /// Top left corner.
    TopLeft,
    /// Top right corner.
    TopRight,
    /// Bottom left corner.
    BottomLeft,
    /// Bottom right corner.
    BottomRight,
}

impl PopupEdge {
    pub(crate) fn anchor(self) -> Anchor {
        match self {
            PopupEdge::None => Anchor::None,
            PopupEdge::Top => Anchor::Top,
            PopupEdge::Bottom => Anchor::Bottom,
            PopupEdge::Left => Anchor::Left,
            PopupEdge::Right => Anchor::Right,
            PopupEdge::TopLeft => Anchor::TopLeft,
            PopupEdge::TopRight => Anchor::TopRight,
            PopupEdge::BottomLeft => Anchor::BottomLeft,
            PopupEdge::BottomRight => Anchor::BottomRight,
        }
    }

    pub(crate) fn gravity(self) -> Gravity {
        match self {
            PopupEdge::None => Gravity::None,
            PopupEdge::Top => Gravity::Top,
            PopupEdge::Bottom => Gravity::Bottom,
            PopupEdge::Left => Gravity::Left,
            PopupEdge::Right => Gravity::Right,
            PopupEdge::TopLeft => Gravity::TopLeft,
            PopupEdge::TopRight => Gravity::TopRight,
            PopupEdge::BottomLeft => Gravity::BottomLeft,
            PopupEdge::BottomRight => Gravity::BottomRight,
        }
    }
}

/// Placement of a popup relative to its parent.
#[derive(Debug, Clone)]
pub struct PopupConfig {
    /// Rectangle in the parent the popup attaches to: x, y, width, height.
    pub anchor_rect: (i32, i32, i32, i32),
    /// Size of the popup in logical pixels.
    pub size: (u32, u32),
    /// Point of the anchor rectangle the popup attaches to.
    pub anchor: PopupEdge,
    /// Direction in which the popup extends from the anchor point.
    pub gravity: PopupEdge,
    /// Offset from the anchor point.
    pub offset: (i32, i32),
    /// How the compositor may move the popup to keep it on screen.
    pub adjustment: ConstraintAdjustment,
    /// Whether the popup grabs input, so clicks outside dismiss it.
    pub grab: bool,
    /// Colour the popup is cleared to before drawing.
    pub clear: Color,
}

impl PopupConfig {
    /// Popup of `size` at `anchor_rect`, anchored bottom left, extending bottom right, adjusted by sliding and flipping.
    pub fn new(anchor_rect: (i32, i32, i32, i32), size: (u32, u32)) -> Self {
        PopupConfig {
            anchor_rect,
            size,
            anchor: PopupEdge::BottomLeft,
            gravity: PopupEdge::BottomRight,
            offset: (0, 0),
            adjustment: ConstraintAdjustment::SlideX
                | ConstraintAdjustment::SlideY
                | ConstraintAdjustment::FlipX
                | ConstraintAdjustment::FlipY,
            grab: false,
            clear: Color::TRANSPARENT,
        }
    }

    /// Sets the anchor point.
    pub fn anchor(self, anchor: PopupEdge) -> Self {
        PopupConfig { anchor, ..self }
    }

    /// Sets the gravity.
    pub fn gravity(self, gravity: PopupEdge) -> Self {
        PopupConfig { gravity, ..self }
    }

    /// Sets the offset from the anchor point.
    pub fn offset(self, x: i32, y: i32) -> Self {
        PopupConfig { offset: (x, y), ..self }
    }

    /// Sets the constraint adjustment.
    pub fn adjustment(self, adjustment: ConstraintAdjustment) -> Self {
        PopupConfig { adjustment, ..self }
    }

    /// Sets whether the popup grabs input.
    pub fn grab(self, grab: bool) -> Self {
        PopupConfig { grab, ..self }
    }

    /// Sets the clear colour.
    pub fn clear(self, clear: Color) -> Self {
        PopupConfig { clear, ..self }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_map_to_matching_positioner_values() {
        assert_eq!(PopupEdge::TopRight.anchor(), Anchor::TopRight);
        assert_eq!(PopupEdge::BottomLeft.gravity(), Gravity::BottomLeft);
        assert_eq!(PopupEdge::None.anchor(), Anchor::None);
    }

    #[test]
    fn default_popup_slides_and_flips() {
        let config = PopupConfig::new((0, 0, 10, 10), (100, 50));
        assert!(config.adjustment.contains(ConstraintAdjustment::SlideX | ConstraintAdjustment::FlipY));
        assert!(!config.grab);
    }
}
