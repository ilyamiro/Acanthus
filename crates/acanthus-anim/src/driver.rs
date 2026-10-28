use std::cell::RefCell;

use slotmap::{SlotMap, new_key_type};

use crate::Animation;

new_key_type! {
    struct AnimKey;
}

thread_local! {
    static RUNNING: RefCell<SlotMap<AnimKey, Option<Box<dyn Animation>>>> = RefCell::new(SlotMap::with_key());
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Handle to an animation registered with [`start`].
pub struct AnimHandle(AnimKey);

impl AnimHandle {
    /// Stops and drops the animation.
    pub fn stop(self) {
        let removed = RUNNING.with(|r| r.borrow_mut().remove(self.0));
        drop(removed);
    }

    /// Whether the animation is still registered.
    pub fn is_running(self) -> bool {
        RUNNING.with(|r| r.borrow().contains_key(self.0))
    }
}

/// Registers an animation on the thread-local driver and starts it.
pub fn start(mut anim: impl Animation) -> AnimHandle {
    anim.start();
    AnimHandle(RUNNING.with(|r| r.borrow_mut().insert(Some(Box::new(anim)))))
}

/// Whether any animation is running; the frame loop uses it to keep requesting frames.
pub fn is_active() -> bool {
    RUNNING.with(|r| !r.borrow().is_empty())
}

/// Advances every running animation by `dt` seconds and drops the finished ones.
pub fn tick(dt: f32) {
    let keys: Vec<AnimKey> = RUNNING.with(|r| r.borrow().keys().collect());
    for key in keys {
        let Some(mut anim) = RUNNING.with(|r| r.borrow_mut().get_mut(key).and_then(Option::take)) else {
            continue;
        };
        let finished = anim.advance(dt).is_some();
        let dropped = RUNNING.with(|r| {
            let mut r = r.borrow_mut();
            match r.get_mut(key) {
                Some(slot) if !finished => {
                    *slot = Some(anim);
                    None
                }
                Some(_) => {
                    r.remove(key);
                    Some(anim)
                }
                None => Some(anim),
            }
        });
        drop(dropped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Easing, Tween};
    use acanthus_core::Signal;

    #[test]
    fn driver_runs_until_finished() {
        let s = Signal::new(0.0f32);
        let h = start(Tween::new(&s, 1.0, 100, Easing::Linear));
        assert!(h.is_running());
        tick(0.05);
        assert!((s.get_untracked() - 0.5).abs() < 1e-4);
        tick(0.06);
        assert!(!h.is_running());
        assert_eq!(s.get_untracked(), 1.0);
    }

    #[test]
    fn stop_halts_animation() {
        let s = Signal::new(0.0f32);
        let h = start(Tween::new(&s, 1.0, 100, Easing::Linear));
        h.stop();
        tick(0.05);
        assert_eq!(s.get_untracked(), 0.0);
    }
}
