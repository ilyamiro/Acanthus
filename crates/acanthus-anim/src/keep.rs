use std::cell::RefCell;
use std::rc::Rc;

use crate::{AnimHandle, Animatable, Animated, Easing, Loop, Motion, SpringParams, Tween, start};
use acanthus_core::{Color, Effect, ListSignal, Scope, Signal, untrack};

#[derive(Default)]
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

#[derive(Default)]
/// A [`Scope`] that owns the derived signals, effects and animations of a view, so they all drop together.
pub struct Keep(Scope);

impl std::ops::Deref for Keep {
    type Target = Scope;

    fn deref(&self) -> &Scope {
        &self.0
    }
}

impl std::ops::DerefMut for Keep {
    fn deref_mut(&mut self) -> &mut Scope {
        &mut self.0
    }
}

impl Keep {
    /// Lightens `color` by `factor` when `compact`, otherwise returns it unchanged.
    pub fn tone(&mut self, color: &Signal<Color>, compact: bool, factor: f32) -> Signal<Color> {
        if !compact {
            return color.clone();
        }
        let color = color.clone();
        self.derive(move || color.get().lighten(factor))
    }

    /// Derives `color` with a fixed alpha.
    pub fn alpha(&mut self, color: &Signal<Color>, alpha: f32) -> Signal<Color> {
        let color = color.clone();
        self.derive(move || color.get().with_alpha(alpha))
    }

    /// Loops `signal` from 0 to `to` over `ms` milliseconds while `running` is true.
    pub fn looping(&mut self, running: &Signal<bool>, signal: &Signal<f32>, to: f32, ms: u32) {
        let held = Rc::new(Running::default());
        let (running, signal, slot) = (running.clone(), signal.clone(), held.clone());
        self.add(Effect::new(move || {
            let on = running.get();
            untrack(|| {
                slot.stop();
                if on {
                    let tween = Tween::new(&signal, to, ms, Easing::Linear).from(0.0);
                    *slot.0.borrow_mut() = Some(start(Loop::infinite(tween)));
                }
            });
        }));
        self.add(held);
    }

    /// Follows `source` with a spring.
    pub fn spring(&mut self, source: &Signal<f32>, params: SpringParams) -> Signal<f32> {
        self.animate(source, Motion::Spring(params))
    }

    /// Follows `source` with an eased tween.
    pub fn ease<T: Animatable>(&mut self, source: &Signal<T>, duration: u32, easing: Easing) -> Signal<T> {
        self.animate(source, Motion::tween(duration, easing))
    }

    fn animate<T: Animatable>(&mut self, source: &Signal<T>, motion: Motion) -> Signal<T> {
        let animated = Animated::new(source, motion);
        let signal = animated.signal().clone();
        self.add(animated);
        signal
    }
}

/// A model of the indices `0..count` that is rebuilt whenever `count` changes, plus the [`Keep`] holding its binding.
pub fn counted(count: Signal<usize>) -> (ListSignal<usize>, Keep) {
    let mut keep = Keep::default();
    let model = ListSignal::new((0..count.get_untracked()).collect());
    let target = model.clone();
    keep.add(Signal::new(0usize).bind(move || {
        let wanted = count.get();
        if target.len() != wanted {
            target.replace((0..wanted).collect());
        }
        wanted
    }));
    (model, keep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wave_loop_keeps_running_while_the_fill_it_follows_rises() {
        let fill = Signal::new(0.0f32);
        let phase = Signal::new(0.0f32);
        let mut keep = Keep::default();
        let waving = {
            let fill = fill.clone();
            keep.derive(move || {
                let level = fill.get();
                level > 0.0 && level < 1.0
            })
        };
        keep.looping(&waving, &phase, std::f32::consts::TAU, 800);
        let mut peak = 0.0f32;
        for step in 1..=25 {
            fill.set(step as f32 * 0.03);
            crate::tick(0.016);
            peak = peak.max(phase.get_untracked());
        }
        assert!(peak > 2.5, "phase only reached {peak}");
        fill.set(1.0);
        let frozen = phase.get_untracked();
        crate::tick(0.1);
        assert_eq!(phase.get_untracked(), frozen);
        drop(keep);
    }
}
