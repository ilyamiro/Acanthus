use std::cell::RefCell;
use std::rc::Rc;

use acanthus_anim::{AnimHandle, Animation, Easing, start};
use acanthus_core::{Signal, Vec2, batch};

use crate::input::ScrollDelta;

pub const FLICK_DECELERATION: f32 = 1500.0;
pub const MAXIMUM_FLICK_VELOCITY: f32 = 2500.0;
pub const FLICK_START_DISTANCE: f32 = 15.0;
pub const MINIMUM_FLICK_VELOCITY: f32 = 75.0;
pub const FIXUP_DURATION: f32 = 0.4;
pub const OVERSHOOT_DISTANCE: f32 = 200.0;
pub const VELOCITY_WINDOW: f32 = 0.1;
const SETTLED_VELOCITY: f32 = 1e-3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FlickDirection {
    #[default]
    Auto,
    Horizontal,
    Vertical,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BoundsBehavior {
    StopAtBounds,
    DragOverBounds,
    OvershootBounds,
    #[default]
    DragAndOvershootBounds,
}

impl BoundsBehavior {
    pub fn drag_over(self) -> bool {
        matches!(self, BoundsBehavior::DragOverBounds | BoundsBehavior::DragAndOvershootBounds)
    }

    pub fn overshoot(self) -> bool {
        matches!(self, BoundsBehavior::OvershootBounds | BoundsBehavior::DragAndOvershootBounds)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FlickPlan {
    pub velocity: f32,
    pub acceleration: f32,
    pub end: f32,
    pub fixup_to: Option<f32>,
}

pub(crate) fn plan_flick(
    position: f32,
    velocity: f32,
    min: f32,
    max: f32,
    overshoot: bool,
    overshoot_limit: f32,
) -> Option<FlickPlan> {
    let velocity = velocity.clamp(-MAXIMUM_FLICK_VELOCITY, MAXIMUM_FLICK_VELOCITY);
    if velocity.abs() < SETTLED_VELOCITY {
        return None;
    }
    let sign = velocity.signum();
    let edge = if velocity > 0.0 { max } else { min };
    let distance_to_edge = (edge - position).abs();
    let squared = velocity * velocity;
    let natural = squared / (2.0 * FLICK_DECELERATION);
    if distance_to_edge > 0.0 && natural < distance_to_edge {
        return Some(FlickPlan {
            velocity,
            acceleration: FLICK_DECELERATION,
            end: position + sign * natural,
            fixup_to: None,
        });
    }
    if overshoot {
        let total = distance_to_edge + overshoot_limit.max(0.0);
        let acceleration =
            if total > 0.0 { (squared / (2.0 * total)).max(FLICK_DECELERATION) } else { FLICK_DECELERATION };
        let travelled = squared / (2.0 * acceleration);
        return Some(FlickPlan { velocity, acceleration, end: position + sign * travelled, fixup_to: Some(edge) });
    }
    if distance_to_edge <= 0.0 {
        return None;
    }
    Some(FlickPlan { velocity, acceleration: squared / (2.0 * distance_to_edge), end: edge, fixup_to: None })
}

enum Phase {
    Decelerate,
    Fixup { from: f32, to: f32, elapsed: f32 },
}

struct FlickMotion {
    position: Signal<f32>,
    flicking: Signal<bool>,
    moving: Signal<bool>,
    plan: FlickPlan,
    current: f32,
    speed: f32,
    phase: Phase,
}

impl FlickMotion {
    fn finish(&self) {
        batch(|| {
            self.flicking.set(false);
            self.moving.set(false);
        });
    }

    fn begin_fixup(&mut self, to: f32) {
        self.phase = Phase::Fixup { from: self.current, to, elapsed: 0.0 };
    }
}

impl Animation for FlickMotion {
    fn start(&mut self) {}

    fn advance(&mut self, dt: f32) -> Option<f32> {
        let mut dt = dt;
        if let Phase::Decelerate = self.phase {
            let sign = self.plan.velocity.signum();
            let stop_in = self.speed / self.plan.acceleration;
            let step = dt.min(stop_in);
            self.current += sign * (self.speed * step - 0.5 * self.plan.acceleration * step * step);
            self.speed -= self.plan.acceleration * step;
            dt -= step;
            if step >= stop_in || self.speed <= SETTLED_VELOCITY {
                self.current = self.plan.end;
                self.position.set(self.current);
                match self.plan.fixup_to {
                    Some(to) if (to - self.current).abs() > 0.0 => self.begin_fixup(to),
                    _ => {
                        self.finish();
                        return Some(dt);
                    }
                }
            } else {
                self.position.set(self.current);
                return None;
            }
        }
        if let Phase::Fixup { from, to, elapsed } = &mut self.phase {
            *elapsed += dt;
            if *elapsed >= FIXUP_DURATION {
                let leftover = *elapsed - FIXUP_DURATION;
                self.position.set(*to);
                self.finish();
                return Some(leftover);
            }
            let t = Easing::OutQuad.apply(*elapsed / FIXUP_DURATION);
            self.current = *from + (*to - *from) * t;
            self.position.set(self.current);
        }
        None
    }
}

struct FixupMotion {
    position: Signal<f32>,
    flicking: Signal<bool>,
    moving: Signal<bool>,
    from: f32,
    to: f32,
    elapsed: f32,
}

impl Animation for FixupMotion {
    fn start(&mut self) {}

    fn advance(&mut self, dt: f32) -> Option<f32> {
        self.elapsed += dt;
        if self.elapsed >= FIXUP_DURATION {
            self.position.set(self.to);
            batch(|| {
                self.flicking.set(false);
                self.moving.set(false);
            });
            return Some(self.elapsed - FIXUP_DURATION);
        }
        let t = Easing::OutQuad.apply(self.elapsed / FIXUP_DURATION);
        self.position.set(self.from + (self.to - self.from) * t);
        None
    }
}

#[derive(Default)]
struct Drag {
    origin_pointer: Vec2,
    origin_content: Vec2,
    started: bool,
    samples: Vec<(f32, Vec2)>,
}

#[derive(Default)]
struct State {
    motion: [Option<AnimHandle>; 2],
    drag: Option<Drag>,
}

#[derive(Clone)]
pub struct Flickable {
    pub content_x: Signal<f32>,
    pub content_y: Signal<f32>,
    pub content_width: Signal<f32>,
    pub content_height: Signal<f32>,
    pub direction: Signal<FlickDirection>,
    pub bounds: Signal<BoundsBehavior>,
    pub interactive: Signal<bool>,
    pub moving: Signal<bool>,
    pub dragging: Signal<bool>,
    pub flicking: Signal<bool>,
    state: Rc<RefCell<State>>,
}

impl Default for Flickable {
    fn default() -> Self {
        Flickable::new()
    }
}

impl Flickable {
    pub fn new() -> Self {
        Flickable {
            content_x: Signal::new(0.0),
            content_y: Signal::new(0.0),
            content_width: Signal::new(0.0),
            content_height: Signal::new(0.0),
            direction: Signal::new(FlickDirection::Auto),
            bounds: Signal::new(BoundsBehavior::default()),
            interactive: Signal::new(true),
            moving: Signal::new(false),
            dragging: Signal::new(false),
            flicking: Signal::new(false),
            state: Rc::new(RefCell::new(State::default())),
        }
    }

    pub fn direction(self, v: FlickDirection) -> Self {
        self.direction.set(v);
        self
    }

    pub fn bounds(self, v: BoundsBehavior) -> Self {
        self.bounds.set(v);
        self
    }

    pub fn content_size(self, w: f32, h: f32) -> Self {
        self.content_width.set(w);
        self.content_height.set(h);
        self
    }

    pub fn position(&self) -> Vec2 {
        Vec2::new(self.content_x.get_untracked(), self.content_y.get_untracked())
    }

    pub fn max_position(&self, viewport: Vec2) -> Vec2 {
        Vec2::new(
            (self.content_width.get_untracked() - viewport.x).max(0.0),
            (self.content_height.get_untracked() - viewport.y).max(0.0),
        )
    }

    fn axis_signal(&self, axis: usize) -> &Signal<f32> {
        if axis == 0 { &self.content_x } else { &self.content_y }
    }

    fn axis_allowed(&self, axis: usize, viewport: Vec2) -> bool {
        let max = self.max_position(viewport);
        match self.direction.get_untracked() {
            FlickDirection::Auto => max[axis] > 0.0,
            FlickDirection::Horizontal => axis == 0,
            FlickDirection::Vertical => axis == 1,
            FlickDirection::Both => true,
        }
    }

    pub fn can_flick(&self, viewport: Vec2) -> bool {
        self.interactive.get_untracked() && (self.axis_allowed(0, viewport) || self.axis_allowed(1, viewport))
    }

    pub fn stop(&self) {
        let handles = {
            let mut state = self.state.borrow_mut();
            [state.motion[0].take(), state.motion[1].take()]
        };
        for handle in handles.into_iter().flatten() {
            handle.stop();
        }
        batch(|| {
            self.flicking.set(false);
            if !self.dragging.get_untracked() {
                self.moving.set(false);
            }
        });
    }

    pub fn scroll_to(&self, x: f32, y: f32, viewport: Vec2) {
        self.stop();
        let max = self.max_position(viewport);
        batch(|| {
            self.content_x.set(x.clamp(0.0, max.x));
            self.content_y.set(y.clamp(0.0, max.y));
        });
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging.get_untracked()
    }

    pub(crate) fn press(&self, pointer: Vec2, time: f32) {
        self.stop();
        self.state.borrow_mut().drag = Some(Drag {
            origin_pointer: pointer,
            origin_content: self.position(),
            started: false,
            samples: vec![(time, self.position())],
        });
    }

    pub(crate) fn drag_to(&self, pointer: Vec2, time: f32, viewport: Vec2) -> bool {
        let mut began = false;
        let position = {
            let mut state = self.state.borrow_mut();
            let Some(drag) = state.drag.as_mut() else { return false };
            let delta = pointer - drag.origin_pointer;
            if !drag.started {
                let crossed =
                    (0..2).any(|axis| self.axis_allowed(axis, viewport) && delta[axis].abs() >= FLICK_START_DISTANCE);
                if !crossed {
                    return false;
                }
                drag.started = true;
                began = true;
                drag.origin_pointer = pointer;
                drag.origin_content = self.position();
                drag.samples.clear();
                drag.samples.push((time, drag.origin_content));
                None
            } else {
                let delta = pointer - drag.origin_pointer;
                let max = self.max_position(viewport);
                let bounds = self.bounds.get_untracked();
                let mut next = drag.origin_content;
                for axis in 0..2 {
                    if !self.axis_allowed(axis, viewport) {
                        continue;
                    }
                    let raw = drag.origin_content[axis] - delta[axis];
                    next[axis] = if raw < 0.0 {
                        if bounds.drag_over() { raw / 2.0 } else { 0.0 }
                    } else if raw > max[axis] {
                        if bounds.drag_over() { max[axis] + (raw - max[axis]) / 2.0 } else { max[axis] }
                    } else {
                        raw
                    };
                }
                drag.samples.push((time, next));
                let oldest = time - VELOCITY_WINDOW * 2.0;
                drag.samples.retain(|(t, _)| *t >= oldest);
                Some(next)
            }
        };
        if began {
            batch(|| {
                self.dragging.set(true);
                self.moving.set(true);
            });
        }
        if let Some(position) = position {
            batch(|| {
                self.content_x.set(position.x);
                self.content_y.set(position.y);
            });
        }
        began
    }

    pub(crate) fn release(&self, time: f32, viewport: Vec2) {
        let Some(drag) = self.state.borrow_mut().drag.take() else { return };
        if !drag.started {
            return;
        }
        self.dragging.set(false);
        let velocity = velocity_from(&drag.samples, time);
        let max = self.max_position(viewport);
        let bounds = self.bounds.get_untracked();
        let mut any = false;
        for axis in 0..2 {
            if !self.axis_allowed(axis, viewport) {
                continue;
            }
            let position = self.position()[axis];
            let limit = max[axis];
            let outside = position < 0.0 || position > limit;
            let signal = self.axis_signal(axis).clone();
            let overshoot_limit = OVERSHOOT_DISTANCE.min(viewport[axis] / 3.0);
            let handle = if outside {
                let to = position.clamp(0.0, limit);
                Some(start(FixupMotion {
                    position: signal,
                    flicking: self.flicking.clone(),
                    moving: self.moving.clone(),
                    from: position,
                    to,
                    elapsed: 0.0,
                }))
            } else if velocity[axis].abs() >= MINIMUM_FLICK_VELOCITY {
                plan_flick(position, velocity[axis], 0.0, limit, bounds.overshoot(), overshoot_limit).map(|plan| {
                    start(FlickMotion {
                        position: signal,
                        flicking: self.flicking.clone(),
                        moving: self.moving.clone(),
                        plan,
                        current: position,
                        speed: plan.velocity.abs(),
                        phase: Phase::Decelerate,
                    })
                })
            } else {
                None
            };
            if let Some(handle) = handle {
                self.state.borrow_mut().motion[axis] = Some(handle);
                any = true;
            }
        }
        batch(|| {
            self.flicking.set(any);
            self.moving.set(any);
        });
    }

    pub(crate) fn wheel(&self, delta: ScrollDelta, viewport: Vec2) -> bool {
        if !self.interactive.get_untracked() {
            return false;
        }
        let max = self.max_position(viewport);
        let mut moved = false;
        let mut next = self.position();
        for (axis, amount) in [delta.x, delta.y].into_iter().enumerate() {
            if amount == 0.0 || !self.axis_allowed(axis, viewport) || max[axis] <= 0.0 {
                continue;
            }
            next[axis] = (next[axis] + amount).clamp(0.0, max[axis]);
            moved = true;
        }
        if moved {
            self.stop();
            batch(|| {
                self.content_x.set(next.x);
                self.content_y.set(next.y);
            });
        }
        moved
    }
}

fn velocity_from(samples: &[(f32, Vec2)], now: f32) -> Vec2 {
    let Some(&(last_time, last_position)) = samples.last() else { return Vec2::ZERO };
    if now - last_time > VELOCITY_WINDOW {
        return Vec2::ZERO;
    }
    let Some(&(first_time, first_position)) = samples.iter().find(|(t, _)| last_time - *t <= VELOCITY_WINDOW) else {
        return Vec2::ZERO;
    };
    let span = last_time - first_time;
    if span <= f32::EPSILON {
        return Vec2::ZERO;
    }
    (last_position - first_position) / span
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_flick_decelerates_to_a_computed_stop() {
        let plan = plan_flick(100.0, 600.0, 0.0, 10_000.0, false, 0.0).unwrap();
        assert_eq!(plan.acceleration, FLICK_DECELERATION);
        assert!((plan.end - (100.0 + 600.0 * 600.0 / 3000.0)).abs() < 1e-3);
        assert_eq!(plan.fixup_to, None);
    }

    #[test]
    fn velocity_is_clamped_to_the_maximum() {
        let plan = plan_flick(0.0, -9000.0, -100_000.0, 100_000.0, false, 0.0).unwrap();
        assert_eq!(plan.velocity, -MAXIMUM_FLICK_VELOCITY);
    }

    #[test]
    fn flick_into_the_edge_stops_exactly_on_it() {
        let plan = plan_flick(90.0, 2000.0, 0.0, 100.0, false, 0.0).unwrap();
        assert_eq!(plan.end, 100.0);
        assert!(plan.acceleration > FLICK_DECELERATION);
        assert!((plan.acceleration - 2000.0 * 2000.0 / 20.0).abs() < 1e-1);
    }

    #[test]
    fn flick_into_the_edge_overshoots_and_fixes_up() {
        let plan = plan_flick(90.0, 2000.0, 0.0, 100.0, true, 200.0).unwrap();
        assert_eq!(plan.fixup_to, Some(100.0));
        assert!(plan.end > 100.0 && plan.end <= 300.0 + 1e-3);
    }

    #[test]
    fn at_the_edge_without_overshoot_there_is_nothing_to_do() {
        assert!(plan_flick(100.0, 1000.0, 0.0, 100.0, false, 0.0).is_none());
    }

    #[test]
    fn velocity_uses_only_recent_samples() {
        let samples = vec![(0.0, Vec2::new(0.0, 0.0)), (0.05, Vec2::new(0.0, 10.0)), (0.1, Vec2::new(0.0, 30.0))];
        let v = velocity_from(&samples, 0.1);
        assert!((v.y - 300.0).abs() < 1e-3);
        assert_eq!(velocity_from(&samples, 0.5), Vec2::ZERO);
    }
}
