use std::cell::Cell;
use std::rc::Rc;

use acanthus_core::Signal;

use crate::{Animatable, Easing};

/// A steppable animation driven by [`crate::start`] and [`crate::tick`].
pub trait Animation: 'static {
    /// Resets the animation and applies its initial state.
    fn start(&mut self);
    /// Advances by `dt` seconds. Returns `None` while running, or `Some(leftover)` with the unused time once finished.
    fn advance(&mut self, dt: f32) -> Option<f32>;
}

/// Interpolates a signal to a target over a fixed duration with an [`Easing`] curve, like QML `PropertyAnimation`.
pub struct Tween<T: Animatable> {
    target: Signal<T>,
    from: Option<T>,
    to: T,
    duration: f32,
    easing: Easing,
    start_value: Option<T>,
    elapsed: f32,
}

impl<T: Animatable> Tween<T> {
    /// Tweens `target` to `to` over `duration_ms` milliseconds.
    pub fn new(target: &Signal<T>, to: T, duration_ms: u32, easing: Easing) -> Self {
        Tween {
            target: target.clone(),
            from: None,
            to,
            duration: duration_ms as f32 / 1000.0,
            easing,
            start_value: None,
            elapsed: 0.0,
        }
    }

    /// Starts from `from` instead of the current value of the target.
    pub fn from(mut self, from: T) -> Self {
        self.from = Some(from);
        self
    }
}

impl<T: Animatable> Animation for Tween<T> {
    fn start(&mut self) {
        self.elapsed = 0.0;
        let start = self.from.clone().unwrap_or_else(|| self.target.get_untracked());
        self.target.set(start.clone());
        self.start_value = Some(start);
    }

    fn advance(&mut self, dt: f32) -> Option<f32> {
        let start = self.start_value.as_ref()?;
        self.elapsed += dt;
        if self.elapsed >= self.duration {
            self.target.set(self.to.clone());
            return Some(self.elapsed - self.duration);
        }
        let t = self.easing.apply(self.elapsed / self.duration);
        self.target.set(start.lerp(&self.to, t));
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Constants of a spring, with the same meaning as QML `SpringAnimation`.
pub struct SpringParams {
    /// Spring strength.
    pub spring: f32,
    /// Damping factor.
    pub damping: f32,
    /// Mass; 1.0 by default.
    pub mass: f32,
    /// Distance and speed below which the spring is considered settled.
    pub epsilon: f32,
    /// Velocity clamp; 0.0 disables it.
    pub max_velocity: f32,
}

impl Default for SpringParams {
    fn default() -> Self {
        SpringParams { spring: 0.0, damping: 0.0, mass: 1.0, epsilon: 0.01, max_velocity: 0.0 }
    }
}

impl SpringParams {
    /// Parameters with the given spring and damping and the defaults for the rest.
    pub fn new(spring: f32, damping: f32) -> Self {
        SpringParams { spring, damping, ..Default::default() }
    }

    /// Sets the mass.
    pub fn mass(self, mass: f32) -> Self {
        SpringParams { mass, ..self }
    }

    /// Sets the settling threshold.
    pub fn epsilon(self, epsilon: f32) -> Self {
        SpringParams { epsilon, ..self }
    }
}

const SPRING_STEP: f32 = 0.016;

/// Moves a signal to a target with spring physics, stepped at a fixed 16 ms like Qt.
pub struct Spring<T: Animatable> {
    target: Signal<T>,
    to: T,
    params: SpringParams,
    velocity: Rc<Cell<[f32; 4]>>,
    current: [f32; 4],
    pending: f32,
}

impl<T: Animatable> Spring<T> {
    /// Springs `target` towards `to`.
    pub fn new(target: &Signal<T>, to: T, params: SpringParams) -> Self {
        Spring {
            target: target.clone(),
            to,
            params,
            velocity: Rc::new(Cell::new([0.0; 4])),
            current: [0.0; 4],
            pending: 0.0,
        }
    }

    /// Shares the velocity cell so a retargeted spring keeps its momentum.
    pub fn with_velocity(mut self, velocity: Rc<Cell<[f32; 4]>>) -> Self {
        self.velocity = velocity;
        self
    }

    fn step(&mut self) -> bool {
        let p = self.params;
        let to = self.to.components();
        let mut v = self.velocity.get();
        let mut settled = true;
        for i in 0..4 {
            let diff = to[i] - self.current[i];
            v[i] += (p.spring * diff - p.damping * v[i]) / p.mass;
            if p.max_velocity > 0.0 {
                v[i] = v[i].clamp(-p.max_velocity, p.max_velocity);
            }
            self.current[i] += v[i] * SPRING_STEP;
            if v[i].abs() >= p.epsilon || (to[i] - self.current[i]).abs() >= p.epsilon {
                settled = false;
            }
        }
        self.velocity.set(v);
        settled
    }
}

impl<T: Animatable> Animation for Spring<T> {
    fn start(&mut self) {
        self.current = self.target.get_untracked().components();
        self.pending = 0.0;
    }

    fn advance(&mut self, dt: f32) -> Option<f32> {
        self.pending += dt;
        while self.pending >= SPRING_STEP {
            self.pending -= SPRING_STEP;
            if self.step() {
                self.velocity.set([0.0; 4]);
                self.target.set(self.to.clone());
                return Some(self.pending);
            }
        }
        self.target.set(T::from_components(self.current));
        None
    }
}

/// Does nothing for a duration, like QML `PauseAnimation`.
pub struct Pause {
    duration: f32,
    elapsed: f32,
}

impl Pause {
    /// Pauses for `duration_ms` milliseconds.
    pub fn new(duration_ms: u32) -> Self {
        Pause { duration: duration_ms as f32 / 1000.0, elapsed: 0.0 }
    }
}

impl Animation for Pause {
    fn start(&mut self) {
        self.elapsed = 0.0;
    }

    fn advance(&mut self, dt: f32) -> Option<f32> {
        self.elapsed += dt;
        (self.elapsed >= self.duration).then_some(self.elapsed - self.duration)
    }
}

/// Runs a closure once when reached, like QML `ScriptAction`.
pub struct Action(Box<dyn FnMut()>);

impl Action {
    /// Wraps the closure to run.
    pub fn new(f: impl FnMut() + 'static) -> Self {
        Action(Box::new(f))
    }
}

impl Animation for Action {
    fn start(&mut self) {}

    fn advance(&mut self, dt: f32) -> Option<f32> {
        (self.0)();
        Some(dt)
    }
}

#[derive(Default)]
/// Runs its animations one after another, like QML `SequentialAnimation`.
pub struct Sequential {
    items: Vec<Box<dyn Animation>>,
    index: usize,
}

impl Sequential {
    /// Creates an empty sequence.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an animation to the sequence.
    pub fn then(mut self, anim: impl Animation) -> Self {
        self.items.push(Box::new(anim));
        self
    }
}

impl Animation for Sequential {
    fn start(&mut self) {
        self.index = 0;
        if let Some(first) = self.items.first_mut() {
            first.start();
        }
    }

    fn advance(&mut self, mut dt: f32) -> Option<f32> {
        while let Some(item) = self.items.get_mut(self.index) {
            dt = item.advance(dt)?;
            self.index += 1;
            if let Some(next) = self.items.get_mut(self.index) {
                next.start();
            }
        }
        Some(dt)
    }
}

#[derive(Default)]
/// Runs its animations together and finishes with the last one, like QML `ParallelAnimation`.
pub struct Parallel {
    items: Vec<(Box<dyn Animation>, Option<f32>)>,
}

impl Parallel {
    /// Creates an empty group.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an animation to the group.
    pub fn with(mut self, anim: impl Animation) -> Self {
        self.items.push((Box::new(anim), None));
        self
    }
}

impl Animation for Parallel {
    fn start(&mut self) {
        for (anim, done) in &mut self.items {
            *done = None;
            anim.start();
        }
    }

    fn advance(&mut self, dt: f32) -> Option<f32> {
        for (anim, done) in &mut self.items {
            if done.is_none() {
                *done = anim.advance(dt);
            }
        }
        self.items.iter().try_fold(dt, |acc, (_, done)| done.map(|l| acc.min(l)))
    }
}

/// Repeats an animation, like QML `loops`.
pub struct Loop {
    inner: Box<dyn Animation>,
    count: Option<u32>,
    done: u32,
}

impl Loop {
    /// Repeats forever.
    pub fn infinite(anim: impl Animation) -> Self {
        Loop { inner: Box::new(anim), count: None, done: 0 }
    }

    /// Repeats `count` times.
    pub fn times(anim: impl Animation, count: u32) -> Self {
        Loop { inner: Box::new(anim), count: Some(count), done: 0 }
    }
}

impl Animation for Loop {
    fn start(&mut self) {
        self.done = 0;
        self.inner.start();
    }

    fn advance(&mut self, mut dt: f32) -> Option<f32> {
        loop {
            let leftover = self.inner.advance(dt)?;
            self.done += 1;
            if self.count.is_some_and(|c| self.done >= c) {
                return Some(leftover);
            }
            self.inner.start();
            if leftover <= 0.0 || leftover >= dt {
                return None;
            }
            dt = leftover;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(anim: &mut dyn Animation, dt: f32, max_steps: usize) -> usize {
        anim.start();
        for i in 0..max_steps {
            if anim.advance(dt).is_some() {
                return i + 1;
            }
        }
        max_steps
    }

    #[test]
    fn tween_reaches_target() {
        let s = Signal::new(0.0f32);
        let mut t = Tween::new(&s, 100.0, 200, Easing::OutCubic);
        let steps = run(&mut t, 0.016, 100);
        assert_eq!(s.get_untracked(), 100.0);
        assert_eq!(steps, 13);
    }

    #[test]
    fn sequential_runs_in_order_with_carry_over() {
        let s = Signal::new(0.0f32);
        let mut seq = Sequential::new()
            .then(Tween::new(&s, 10.0, 100, Easing::Linear))
            .then(Pause::new(100))
            .then(Tween::new(&s, 0.0, 100, Easing::Linear));
        seq.start();
        assert!(seq.advance(0.15).is_none());
        assert_eq!(s.get_untracked(), 10.0);
        assert!(seq.advance(0.1).is_none());
        assert!((s.get_untracked() - 5.0).abs() < 1e-3);
        assert!(seq.advance(0.1).is_some());
        assert_eq!(s.get_untracked(), 0.0);
    }

    #[test]
    fn parallel_finishes_with_longest() {
        let a = Signal::new(0.0f32);
        let b = Signal::new(0.0f32);
        let mut par = Parallel::new().with(Tween::new(&a, 1.0, 100, Easing::Linear)).with(Tween::new(
            &b,
            1.0,
            300,
            Easing::Linear,
        ));
        par.start();
        assert!(par.advance(0.2).is_none());
        assert_eq!(a.get_untracked(), 1.0);
        assert!(par.advance(0.2).is_some());
        assert_eq!(b.get_untracked(), 1.0);
    }

    #[test]
    fn spring_settles_on_target() {
        let s = Signal::new(0.0f32);
        let mut sp = Spring::new(&s, 50.0, SpringParams::new(3.0, 0.2).mass(0.2));
        let steps = run(&mut sp, 0.016, 10_000);
        assert!(steps < 10_000);
        assert_eq!(s.get_untracked(), 50.0);
    }

    #[test]
    fn finite_loop_repeats() {
        let count = Rc::new(Cell::new(0));
        let c = count.clone();
        let mut l =
            Loop::times(Sequential::new().then(Pause::new(100)).then(Action::new(move || c.set(c.get() + 1))), 3);
        run(&mut l, 0.05, 100);
        assert_eq!(count.get(), 3);
    }
}
