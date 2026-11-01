//! Wavy seek bar and fluid wave: canvases drawing a travelling sine, one as a progress line that can be dragged, one as a filled swell.

use std::f32::consts::TAU;
use std::rc::Rc;

use acanthus_anim::Keep;
use acanthus_core::{Color, Signal, Vec2};
use acanthus_scene::{CursorShape, Node, NodeId, Path, PointerPhase, Scene};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::BASE;

/// Duration of one cycle of the travelling phase, in milliseconds.
pub const PHASE_MS: u32 = 1500;

/// Distance between sampled points of a wave, in pixels.
const STEP: f32 = 2.0;

/// Vertical offset of a sine of `amplitude` and `wavelength` at `x`, shifted by `phase` radians.
pub fn wave_y(x: f32, amplitude: f32, wavelength: f32, phase: f32) -> f32 {
    amplitude * (TAU * x / wavelength.max(1.0) - phase).sin()
}

/// Polyline of a wave from `x = 0` to `length` around the height `mid`.
pub fn wave_path(length: f32, mid: f32, amplitude: f32, wavelength: f32, phase: f32) -> Path {
    let steps = (length / STEP).ceil().max(1.0) as u32;
    (0..=steps).fold(Path::new(), |path, i| {
        let x = (i as f32 * STEP).min(length);
        let p = Vec2::new(x, mid + wave_y(x, amplitude, wavelength, phase));
        if i == 0 { path.move_to(p) } else { path.line_to(p) }
    })
}

/// Fraction of the bar (0 to 1) under a pointer at `x` on a bar of `width`.
pub fn seek_fraction(x: f32, width: f32) -> f32 {
    (x / width.max(1.0)).clamp(0.0, 1.0)
}

/// Description of a wavy seek bar.
pub struct WaveSeek {
    /// Progress from 0 to 1.
    pub value: Signal<f32>,
    /// The wave travels and keeps its height while true; otherwise it settles to a straight line.
    pub playing: Signal<bool>,
    /// Colour of the played part.
    pub color: Signal<Color>,
    /// Colour of the part still to play.
    pub track: Signal<Color>,
    /// Width of the bar.
    pub width: f32,
    /// Height of the bar, which also bounds the wave.
    pub height: f32,
    /// Peak height of the wave.
    pub amplitude: f32,
    /// Distance between two crests.
    pub wavelength: f32,
    /// Stroke width of both lines.
    pub thickness: f32,
    /// Called with the fraction under the pointer on press and while dragging.
    pub on_seek: Rc<dyn Fn(f32)>,
}

/// Builds the seek bar under `parent` and returns it.
pub fn wave_seek(scene: &mut Scene, parent: NodeId, bar: WaveSeek) -> NodeId {
    let WaveSeek { value, playing, color, track, width, height, amplitude, wavelength, thickness, on_seek } = bar;
    let mut keep = Keep::default();
    let phase = Signal::new(0.0_f32);
    keep.looping(&playing, &phase, TAU, PHASE_MS);
    let lift = {
        let wanted = keep.derive({
            let playing = playing.clone();
            move || if playing.get() { 1.0_f32 } else { 0.0 }
        });
        keep.ease(&wanted, BASE, acanthus_anim::Easing::OutCubic)
    };
    let mid = height / 2.0;
    let pointer = move |event: acanthus_scene::PointerEvent| {
        if matches!(event.phase, PointerPhase::Press | PointerPhase::Drag) {
            on_seek(seek_fraction(event.position.x, width));
        }
    };
    scene.add(
        parent,
        Node::canvas(move |painting| {
            let played = value.get().clamp(0.0, 1.0) * width;
            let rest = Path::new().move_to(Vec2::new(played, mid)).line_to(Vec2::new(width, mid));
            painting.stroke(&rest, track.get(), thickness);
            if played > 0.0 {
                let amp = amplitude.min((height - thickness) / 2.0).max(0.0) * lift.get();
                painting.stroke(&wave_path(played, mid, amp, wavelength, phase.get()), color.get(), thickness);
            }
        })
        .size(width, height)
        .cursor(CursorShape::Pointer)
        .on_pointer(pointer)
        .own(keep),
    )
}

/// Description of a fluid wave.
pub struct FluidWave {
    /// Fill colour of the swell.
    pub color: Signal<Color>,
    /// The swell moves while true and holds its shape otherwise.
    pub running: Signal<bool>,
    /// Width of the area.
    pub width: f32,
    /// Height of the area; the swell fills from its surface down to the bottom.
    pub height: f32,
    /// Peak height of the surface.
    pub amplitude: f32,
    /// Distance between two crests of the main sine.
    pub wavelength: f32,
}

/// Builds the swell under `parent` and returns it: two sines of different length and speed added together.
pub fn fluid_wave(scene: &mut Scene, parent: NodeId, wave: FluidWave) -> NodeId {
    let FluidWave { color, running, width, height, amplitude, wavelength } = wave;
    let mut keep = Keep::default();
    let phase = Signal::new(0.0_f32);
    keep.looping(&running, &phase, TAU, PHASE_MS * 2);
    scene.add(
        parent,
        Node::canvas(move |painting| {
            let steps = (width / STEP).ceil().max(1.0) as u32;
            let base = height - amplitude * 1.5;
            let p = phase.get();
            let mut path = Path::new().move_to(Vec2::new(0.0, height));
            for i in 0..=steps {
                let x = (i as f32 * STEP).min(width);
                let y =
                    base + wave_y(x, amplitude, wavelength, p) + wave_y(x, amplitude * 0.5, wavelength * 0.6, -2.0 * p);
                path = path.line_to(Vec2::new(x, y.clamp(0.0, height)));
            }
            painting.fill(&path.line_to(Vec2::new(width, height)).close(), color.get());
        })
        .size(width, height)
        .own(keep),
    )
}

/// The seek bar as a [`Component`] named `WaveSeek`.
pub struct WaveSeekComponent;

impl Component for WaveSeekComponent {
    fn name(&self) -> &'static str {
        "WaveSeek"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::number("value", || Prop::number(0.0)),
            PropSpec::bool("playing", || Prop::bool(true)),
            PropSpec::number("width", || Prop::number(240.0)),
            PropSpec::number("height", || Prop::number(16.0)),
            PropSpec::number("amplitude", || Prop::number(3.0)),
            PropSpec::number("wavelength", || Prop::number(24.0)),
            PropSpec::number("thickness", || Prop::number(3.0)),
            PropSpec::number_event("on_seek"),
            PropSpec::themed("color", "primary"),
            PropSpec::themed("track", "surface1"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let get = |name: &str| props.number(name).get_untracked();
        wave_seek(
            scene,
            parent,
            WaveSeek {
                value: props.number("value"),
                playing: props.bool("playing"),
                color: props.color("color"),
                track: props.color("track"),
                width: get("width"),
                height: get("height"),
                amplitude: get("amplitude"),
                wavelength: get("wavelength"),
                thickness: get("thickness"),
                on_seek: props.number_event("on_seek"),
            },
        )
    }
}

/// The swell as a [`Component`] named `FluidWave`.
pub struct FluidWaveComponent;

impl Component for FluidWaveComponent {
    fn name(&self) -> &'static str {
        "FluidWave"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::bool("running", || Prop::bool(true)),
            PropSpec::number("width", || Prop::number(240.0)),
            PropSpec::number("height", || Prop::number(48.0)),
            PropSpec::number("amplitude", || Prop::number(6.0)),
            PropSpec::number("wavelength", || Prop::number(120.0)),
            PropSpec::themed("color", "primary"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let get = |name: &str| props.number(name).get_untracked();
        fluid_wave(
            scene,
            parent,
            FluidWave {
                color: props.color("color"),
                running: props.bool("running"),
                width: get("width"),
                height: get("height"),
                amplitude: get("amplitude"),
                wavelength: get("wavelength"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use acanthus_scene::{Fonts, MouseButton};

    use super::*;
    use crate::component::Registry;

    #[test]
    fn the_sine_has_its_amplitude_and_period() {
        assert!(wave_y(0.0, 3.0, 24.0, 0.0).abs() < 1e-5);
        assert!((wave_y(6.0, 3.0, 24.0, 0.0) - 3.0).abs() < 1e-4);
        assert!((wave_y(24.0, 3.0, 24.0, 0.0)).abs() < 1e-4);
        assert!((wave_y(0.0, 3.0, 24.0, -std::f32::consts::FRAC_PI_2) - 3.0).abs() < 1e-5);
        assert_eq!(seek_fraction(-5.0, 100.0), 0.0);
        assert_eq!(seek_fraction(25.0, 100.0), 0.25);
        assert_eq!(seek_fraction(500.0, 100.0), 1.0);
    }

    #[test]
    fn pressing_and_dragging_seek_and_the_components_build() {
        let mut registry = Registry::new();
        registry.register(WaveSeekComponent);
        registry.register(FluidWaveComponent);
        let seen = Rc::new(Cell::new(-1.0_f32));
        let mut given: Vec<(String, Prop)> =
            ["color", "track"].map(|n| (n.to_owned(), Prop::color(Color::hex(0x778899)))).to_vec();
        let sink = seen.clone();
        given.push(("on_seek".to_owned(), Prop::NumberEvent(Rc::new(move |v| sink.set(v)))));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let bar = registry.build("WaveSeek", &mut scene, root, given).unwrap();
        let wave = registry
            .build("FluidWave", &mut scene, root, vec![("color".to_owned(), Prop::color(Color::hex(0x112233)))])
            .unwrap();
        scene.set_viewport(Vec2::new(400.0, 200.0), 1.0);
        scene.frame();
        assert!(scene.geometry(bar).is_some_and(|g| g.w == 240.0 && g.h == 16.0));
        assert!(scene.geometry(wave).is_some_and(|g| g.h == 48.0));
        scene.pointer_motion(Vec2::new(60.0, 8.0));
        scene.pointer_button(MouseButton::Left, true);
        assert_eq!(seen.get(), 0.25);
        scene.pointer_motion(Vec2::new(480.0, 8.0));
        assert_eq!(seen.get(), 1.0);
        scene.pointer_button(MouseButton::Left, false);
    }
}
