use acanthus_core::Color;
use smithay_client_toolkit::reexports::protocols::xdg::shell::client::xdg_positioner::{
    Anchor, ConstraintAdjustment, Gravity,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PopupEdge {
    #[default]
    None,
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
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

#[derive(Debug, Clone)]
pub struct PopupConfig {
    pub anchor_rect: (i32, i32, i32, i32),
    pub size: (u32, u32),
    pub anchor: PopupEdge,
    pub gravity: PopupEdge,
    pub offset: (i32, i32),
    pub adjustment: ConstraintAdjustment,
    pub grab: bool,
    pub clear: Color,
}

impl PopupConfig {
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

    pub fn anchor(self, anchor: PopupEdge) -> Self {
        PopupConfig { anchor, ..self }
    }

    pub fn gravity(self, gravity: PopupEdge) -> Self {
        PopupConfig { gravity, ..self }
    }

    pub fn offset(self, x: i32, y: i32) -> Self {
        PopupConfig { offset: (x, y), ..self }
    }

    pub fn adjustment(self, adjustment: ConstraintAdjustment) -> Self {
        PopupConfig { adjustment, ..self }
    }

    pub fn grab(self, grab: bool) -> Self {
        PopupConfig { grab, ..self }
    }

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
