use acanthus_core::Signal;

/// Short transitions in milliseconds: colour changes on hover.
pub const FAST: u32 = 150;
/// Default transitions in milliseconds: scale, rotation, appearance.
pub const BASE: u32 = 250;
/// Long transitions in milliseconds: settling after a press.
pub const SLOW: u32 = 400;

thread_local! {
    static REDUCED: Signal<bool> = Signal::new(false);
}

/// Turns reduced motion on or off for this thread. While on, [`Animated`](crate::Animated) signals jump to their target and [`Keep::looping`](crate::Keep::looping) loops rest at their midpoint; the change applies to running animations at once.
pub fn set_reduced_motion(on: bool) {
    REDUCED.with(|r| r.set(on));
}

/// Whether reduced motion is on; read inside an effect it makes the effect follow the switch.
pub fn reduced_motion() -> bool {
    REDUCED.with(|r| r.get())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Animated, Easing, Keep, Motion, tick};

    struct Reset;

    impl Drop for Reset {
        fn drop(&mut self) {
            set_reduced_motion(false);
        }
    }

    #[test]
    fn a_followed_signal_jumps_while_reduced() {
        let _reset = Reset;
        let src = Signal::new(0.0f32);
        let anim = Animated::new(&src, Motion::tween(100, Easing::Linear));
        set_reduced_motion(true);
        src.set(10.0);
        assert_eq!(anim.get(), 10.0);
        set_reduced_motion(false);
        src.set(0.0);
        tick(0.05);
        assert!((anim.get() - 5.0).abs() < 1e-3);
    }

    #[test]
    fn switching_on_mid_flight_lands_on_the_target() {
        let _reset = Reset;
        let src = Signal::new(0.0f32);
        let anim = Animated::new(&src, Motion::tween(100, Easing::Linear));
        src.set(10.0);
        tick(0.03);
        set_reduced_motion(true);
        assert_eq!(anim.get(), 10.0);
        tick(0.1);
        assert_eq!(anim.get(), 10.0);
    }

    #[test]
    fn a_spring_jumps_while_reduced() {
        let _reset = Reset;
        let src = Signal::new(0.0f32);
        let anim = Animated::new(&src, Motion::Spring(crate::SpringParams::new(4.0, 0.4)));
        set_reduced_motion(true);
        src.set(100.0);
        assert_eq!(anim.get(), 100.0);
    }

    #[test]
    fn a_loop_rests_at_its_midpoint_and_resumes() {
        let _reset = Reset;
        let running = Signal::new(true);
        let phase = Signal::new(0.0f32);
        let mut keep = Keep::default();
        keep.looping(&running, &phase, 4.0, 800);
        tick(0.1);
        set_reduced_motion(true);
        assert_eq!(phase.get_untracked(), 2.0);
        tick(0.3);
        assert_eq!(phase.get_untracked(), 2.0);
        set_reduced_motion(false);
        tick(0.1);
        assert_ne!(phase.get_untracked(), 2.0);
    }
}
