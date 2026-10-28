use std::cell::RefCell;
use std::rc::Rc;

use acanthus_anim::{Action, AnimHandle, Easing, Keep, Loop, Pause, Sequential, Tween, start};
use acanthus_core::{Color, Signal};
use acanthus_scene::{Layout, Node, NodeId, Scene};

pub const MARQUEE_PAUSE_MS: u32 = 3000;

pub fn can_marquee(active: bool, text_width: f32, width: f32) -> bool {
    active && text_width > width && width > 0.0
}

pub fn scroll_distance(text_width: f32, spacing: f32) -> f32 {
    text_width + spacing
}

pub fn scroll_duration_ms(distance: f32, speed: f32) -> u32 {
    (distance * speed).round().max(1000.0) as u32
}

pub fn scroll_x(progress: f32, distance: f32) -> f32 {
    -(progress * distance).round()
}

pub struct Spec {
    pub text: Signal<String>,
    pub color: Signal<Color>,
    pub family: Signal<String>,
    pub size: f32,
    pub weight: u16,
    pub width: Signal<f32>,
    pub spacing: f32,
    pub speed: f32,
    pub active: Signal<bool>,
}

struct Run(RefCell<Option<AnimHandle>>);

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(handle) = self.0.borrow_mut().take() {
            handle.stop();
        }
    }
}

pub fn build(scene: &mut Scene, parent: NodeId, spec: Spec) -> NodeId {
    let Spec { text, color, family, size, weight, width, spacing, speed, active } = spec;
    let mut keep = Keep::default();
    let label = |visible: Option<&Signal<bool>>| {
        let node = Node::text(&text).font_family(&family).font_size(size).font_weight(weight).color(&color);
        match visible {
            Some(visible) => node.visible(visible),
            None => node,
        }
    };
    let main = label(None);
    let measured = main.implicit_size();
    let height = {
        let measured = measured.clone();
        keep.derive(move || measured.get().y)
    };
    let can = {
        let (measured, active, width) = (measured.clone(), active, width.clone());
        keep.derive(move || can_marquee(active.get(), measured.get().x, width.get()))
    };
    let distance = {
        let measured = measured.clone();
        keep.derive(move || scroll_distance(measured.get().x, spacing))
    };
    let progress = Signal::new(0.0f32);
    let run = Rc::new(Run(RefCell::default()));
    {
        let (can, distance, text, progress, run) =
            (can.clone(), distance.clone(), text.clone(), progress.clone(), run.clone());
        keep.add(Signal::new(()).bind(move || {
            let (running, distance) = (can.get(), distance.get());
            text.get();
            if let Some(old) = run.0.borrow_mut().take() {
                old.stop();
            }
            progress.set(0.0);
            if running {
                let reset = {
                    let progress = progress.clone();
                    Action::new(move || progress.set(0.0))
                };
                let cycle = Sequential::new()
                    .then(Pause::new(MARQUEE_PAUSE_MS))
                    .then(Tween::new(&progress, 1.0, scroll_duration_ms(distance, speed), Easing::Linear))
                    .then(reset);
                *run.0.borrow_mut() = Some(start(Loop::infinite(cycle)));
            }
        }));
    }
    keep.add(run);
    let x = {
        let (can, progress, distance) = (can.clone(), progress, distance);
        keep.derive(move || if can.get() { scroll_x(progress.get(), distance.get()) } else { 0.0 })
    };

    let container = scene.add(parent, Node::item().width(&width).height(&height).clip(true).own(keep));
    let row = scene.add(container, Node::item().layout(Layout::row(spacing)).x(&x));
    scene.add(row, main);
    scene.add(row, label(Some(&can)));
    container
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marquee_maths() {
        assert!(can_marquee(true, 200.0, 120.0));
        assert!(!can_marquee(false, 200.0, 120.0));
        assert!(!can_marquee(true, 100.0, 120.0));
        assert!(!can_marquee(true, 100.0, 0.0));
        assert_eq!(scroll_distance(200.0, 40.0), 240.0);
        assert_eq!(scroll_duration_ms(240.0, 30.0), 7200);
        assert_eq!(scroll_duration_ms(10.0, 30.0), 1000);
        assert_eq!(scroll_x(0.5, 241.0), -121.0);
    }
}
