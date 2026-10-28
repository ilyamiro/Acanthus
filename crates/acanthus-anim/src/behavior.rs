use std::cell::{Cell, RefCell};
use std::rc::Rc;

use acanthus_core::{Effect, Signal, untrack};

use crate::{AnimHandle, Animatable, Easing, Spring, SpringParams, Tween, start};

#[derive(Debug, Clone, Copy, PartialEq)]
/// How a change of a source signal is animated.
pub enum Motion {
    /// Eased interpolation; a duration of 0 snaps to the new value.
    Tween {
        /// Duration in milliseconds.
        duration_ms: u32,
        /// Easing curve.
        easing: Easing,
    },
    /// Spring physics.
    Spring(SpringParams),
}

impl Motion {
    /// A [`Motion::Tween`] with the given duration and easing.
    pub fn tween(duration_ms: u32, easing: Easing) -> Self {
        Motion::Tween { duration_ms, easing }
    }
}

impl From<(u32, Easing)> for Motion {
    fn from((duration_ms, easing): (u32, Easing)) -> Self {
        Motion::Tween { duration_ms, easing }
    }
}

struct Running(RefCell<Option<AnimHandle>>);

impl Running {
    fn stop(&self) {
        if let Some(handle) = self.0.borrow_mut().take() {
            handle.stop();
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A signal that follows a source signal with a [`Motion`], like a QML `Behavior`. Dropping it stops the running animation.
pub struct Animated<T: Animatable> {
    value: Signal<T>,
    _running: Rc<Running>,
    _effect: Effect,
}

impl<T: Animatable> Animated<T> {
    /// Follows `source` with a fixed motion.
    pub fn new(source: &Signal<T>, motion: Motion) -> Self {
        Animated::with(source, move |_| motion)
    }

    /// Follows `source` with a motion chosen from each new target value.
    pub fn with<M: Into<Motion>>(source: &Signal<T>, motion: impl Fn(&T) -> M + 'static) -> Self {
        let value = Signal::new(source.get_untracked());
        let running = Rc::new(Running(RefCell::new(None)));
        let velocity: Rc<Cell<[f32; 4]>> = Rc::default();
        let first = Cell::new(true);
        let source = source.clone();
        let target = value.clone();
        let slot = running.clone();
        let effect = Effect::new(move || {
            let to = source.get();
            if first.replace(false) {
                return;
            }
            untrack(|| {
                slot.stop();
                let handle = match motion(&to).into() {
                    Motion::Tween { duration_ms: 0, .. } => {
                        target.set(to);
                        return;
                    }
                    Motion::Tween { duration_ms, easing } => start(Tween::new(&target, to, duration_ms, easing)),
                    Motion::Spring(params) => start(Spring::new(&target, to, params).with_velocity(velocity.clone())),
                };
                *slot.0.borrow_mut() = Some(handle);
            });
        });
        Animated { value, _running: running, _effect: effect }
    }

    /// The animated signal.
    pub fn signal(&self) -> &Signal<T> {
        &self.value
    }

    /// Reads the current animated value, tracked by effects.
    pub fn get(&self) -> T {
        self.value.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tick;

    #[test]
    fn follows_source_with_animation() {
        let src = Signal::new(0.0f32);
        let anim = Animated::new(&src, Motion::tween(100, Easing::Linear));
        assert_eq!(anim.get(), 0.0);
        src.set(10.0);
        assert_eq!(anim.get(), 0.0);
        tick(0.05);
        assert!((anim.get() - 5.0).abs() < 1e-3);
        src.set(0.0);
        tick(0.05);
        assert!((anim.get() - 2.5).abs() < 1e-3);
        tick(0.1);
        assert_eq!(anim.get(), 0.0);
    }

    #[test]
    fn spring_retarget_keeps_running() {
        let src = Signal::new(0.0f32);
        let anim = Animated::new(&src, Motion::Spring(SpringParams::new(4.0, 0.4)));
        src.set(100.0);
        for _ in 0..5 {
            tick(0.016);
        }
        let mid = anim.get();
        assert!(mid > 0.0 && mid < 100.0);
        src.set(50.0);
        for _ in 0..2000 {
            tick(0.016);
        }
        assert_eq!(anim.get(), 50.0);
    }

    #[test]
    fn motion_can_depend_on_the_target_and_zero_snaps() {
        let src = Signal::new(0.0f32);
        let anim = Animated::with(&src, |to: &f32| if *to > 5.0 { (100, Easing::Linear) } else { (0, Easing::Linear) });
        src.set(10.0);
        tick(0.05);
        assert!((anim.get() - 5.0).abs() < 1e-3);
        tick(0.1);
        src.set(1.0);
        assert_eq!(anim.get(), 1.0);
    }
}
