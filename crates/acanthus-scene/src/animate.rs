use std::any::Any;

use acanthus_anim::{Animatable, Animated, Motion};
use acanthus_core::Signal;

use crate::node::{Kind, Node};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A node property that [`Node::animate`] can make follow its value with a motion, like a QML `Behavior on <property>`.
pub enum Prop {
    /// Horizontal position.
    X,
    /// Vertical position.
    Y,
    /// Explicit width.
    Width,
    /// Explicit height.
    Height,
    /// Opacity.
    Opacity,
    /// Scale factor.
    Scale,
    /// Rotation in degrees.
    Rotation,
    /// Fill colour of a rectangle or colour of a text node.
    Color,
    /// Corner radius of a rectangle.
    Radius,
    /// Border width of a rectangle.
    BorderWidth,
    /// Border colour of a rectangle.
    BorderColor,
}

impl Prop {
    /// Whether the property only changes how the node is composited, so a render thread can advance its animation without the UI thread (Qt's `XAnimator`, `YAnimator`, `OpacityAnimator`, `ScaleAnimator`, `RotationAnimator`).
    pub fn animator_eligible(self) -> bool {
        matches!(self, Prop::X | Prop::Y | Prop::Opacity | Prop::Scale | Prop::Rotation)
    }

    /// The property name as written in markup.
    pub fn name(self) -> &'static str {
        match self {
            Prop::X => "x",
            Prop::Y => "y",
            Prop::Width => "width",
            Prop::Height => "height",
            Prop::Opacity => "opacity",
            Prop::Scale => "scale",
            Prop::Rotation => "rotation",
            Prop::Color => "color",
            Prop::Radius => "radius",
            Prop::BorderWidth => "border-width",
            Prop::BorderColor => "border-color",
        }
    }
}

fn follow<T: Animatable>(slot: &mut Signal<T>, motion: Motion) -> Box<dyn Any> {
    let animated = Animated::new(slot, motion);
    *slot = animated.signal().clone();
    Box::new(animated)
}

impl Node {
    /// Makes `prop` follow its current binding with `motion`: every change of the value the node was given glides to it instead of jumping (QML `Behavior on x`). Apply it after the property is set; a property the node's kind does not have is ignored with a warning. Calling it again on the same property chains the motions.
    pub fn animate(mut self, prop: Prop, motion: impl Into<Motion>) -> Self {
        let motion = motion.into();
        let followed = match prop {
            Prop::X => Some(follow(&mut self.x, motion)),
            Prop::Y => Some(follow(&mut self.y, motion)),
            Prop::Opacity => Some(follow(&mut self.opacity, motion)),
            Prop::Scale => Some(follow(&mut self.scale, motion)),
            Prop::Rotation => Some(follow(&mut self.rotation, motion)),
            Prop::Width => self.width.as_mut().map(|s| follow(s, motion)),
            Prop::Height => self.height.as_mut().map(|s| follow(s, motion)),
            Prop::Color => match &mut self.kind {
                Kind::Rect(s) => Some(follow(&mut s.color, motion)),
                Kind::Text(s) => Some(follow(&mut s.color, motion)),
                _ => None,
            },
            Prop::Radius => match &mut self.kind {
                Kind::Rect(s) => Some(follow(&mut s.radius, motion)),
                _ => None,
            },
            Prop::BorderWidth => match &mut self.kind {
                Kind::Rect(s) => Some(follow(&mut s.border_width, motion)),
                _ => None,
            },
            Prop::BorderColor => match &mut self.kind {
                Kind::Rect(s) => Some(follow(&mut s.border_color, motion)),
                _ => None,
            },
        };
        match followed {
            Some(animated) => self.owned.push(animated),
            None => tracing::warn!("animate({}) has no effect on this node", prop.name()),
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acanthus_anim::{Easing, tick};
    use acanthus_core::Color;

    fn linear(ms: u32) -> Motion {
        Motion::tween(ms, Easing::Linear)
    }

    #[test]
    fn a_bound_position_glides_to_the_new_value() {
        let x = Signal::new(0.0f32);
        let node = Node::item().x(&x).animate(Prop::X, linear(100));
        x.set(10.0);
        assert_eq!(node.x.get_untracked(), 0.0);
        tick(0.05);
        assert!((node.x.get_untracked() - 5.0).abs() < 1e-3);
        tick(0.06);
        assert_eq!(node.x.get_untracked(), 10.0);
    }

    #[test]
    fn opacity_scale_and_rotation_follow_their_sources() {
        let v = Signal::new(0.0f32);
        let node = Node::item()
            .opacity(&v)
            .scale(&v)
            .rotation(&v)
            .animate(Prop::Opacity, linear(100))
            .animate(Prop::Scale, linear(100))
            .animate(Prop::Rotation, linear(100));
        v.set(1.0);
        tick(0.05);
        for s in [&node.opacity, &node.scale, &node.rotation] {
            assert!((s.get_untracked() - 0.5).abs() < 1e-3);
        }
    }

    #[test]
    fn rectangle_colour_radius_and_border_follow() {
        let color = Signal::new(Color::rgba(0.0, 0.0, 0.0, 1.0));
        let radius = Signal::new(0.0f32);
        let node = Node::rect()
            .color(&color)
            .radius(&radius)
            .border(&radius, &color)
            .animate(Prop::Color, linear(100))
            .animate(Prop::Radius, linear(100))
            .animate(Prop::BorderWidth, linear(100))
            .animate(Prop::BorderColor, linear(100));
        color.set(Color::rgba(1.0, 1.0, 1.0, 1.0));
        radius.set(8.0);
        tick(0.05);
        let Kind::Rect(style) = &node.kind else { panic!("not a rect") };
        assert!((style.color.get_untracked().r - 0.5).abs() < 1e-3);
        assert!((style.radius.get_untracked() - 4.0).abs() < 1e-3);
        assert!((style.border_width.get_untracked() - 4.0).abs() < 1e-3);
        assert!((style.border_color.get_untracked().g - 0.5).abs() < 1e-3);
    }

    #[test]
    fn text_colour_and_explicit_size_follow() {
        let color = Signal::new(Color::rgba(0.0, 0.0, 0.0, 1.0));
        let w = Signal::new(0.0f32);
        let node =
            Node::text("a").color(&color).width(&w).animate(Prop::Color, linear(100)).animate(Prop::Width, linear(100));
        color.set(Color::rgba(1.0, 1.0, 1.0, 1.0));
        w.set(20.0);
        tick(0.05);
        let Kind::Text(style) = &node.kind else { panic!("not text") };
        assert!((style.color.get_untracked().b - 0.5).abs() < 1e-3);
        assert!((node.width.as_ref().unwrap().get_untracked() - 10.0).abs() < 1e-3);
    }

    #[test]
    fn a_property_the_kind_lacks_changes_nothing() {
        let node = Node::item().animate(Prop::Radius, linear(100)).animate(Prop::Width, linear(100));
        assert!(node.owned.is_empty());
        assert!(node.width.is_none());
    }

    #[test]
    fn dropping_the_node_stops_the_animation() {
        let x = Signal::new(0.0f32);
        let node = Node::item().x(&x).animate(Prop::X, linear(100));
        let follower = node.x.clone();
        x.set(10.0);
        drop(node);
        tick(0.05);
        assert_eq!(follower.get_untracked(), 0.0);
    }

    #[test]
    fn only_composited_properties_are_animator_eligible() {
        let eligible: Vec<_> = [Prop::X, Prop::Y, Prop::Opacity, Prop::Scale, Prop::Rotation, Prop::Width, Prop::Color]
            .into_iter()
            .filter(|p| p.animator_eligible())
            .collect();
        assert_eq!(eligible, [Prop::X, Prop::Y, Prop::Opacity, Prop::Scale, Prop::Rotation]);
    }
}
