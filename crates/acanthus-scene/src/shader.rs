use std::sync::{Arc, OnceLock};

use acanthus_core::{Color, Vec2};

/// Handle of a GPU shader effect drawn by `Node::shader`.
///
/// The variants are the effects the engine ships. An application adds its own
/// with [`register_app_effects`] and addresses them as `ShaderEffect::App(index)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShaderEffect {
    GridPattern,
    DashedEllipse,
    Vignette,
    CornerCutout,
    RoundedMask,
    MarqueeScroll,
    WallpaperBlend,
    WidgetOutline,
    SeekbarWave,
    CurtainWipe,
    ScreenWipe,
    FluidWave,
    CircularWaveGauge,
    CircularWaveGaugeSmooth,
    VisualizerBars,
    VisualizerWave,
    VisualizerRingWave,
    Ghost,
    BorderCharge,
    /// Effect number `n` of the list passed to [`register_app_effects`].
    App(u16),
}

/// WGSL source of an application-defined effect.
///
/// `wgsl` is the effect body: the renderer prepends the shared effect base
/// (`effect_base.wgsl`) before compiling it.
#[derive(Debug, Clone, Copy)]
pub struct AppEffect {
    /// Label shown in GPU debug tools.
    pub label: &'static str,
    /// Effect body source.
    pub wgsl: &'static str,
}

static APP_EFFECTS: OnceLock<&'static [AppEffect]> = OnceLock::new();

/// Registers the application's shader effects, once per process.
///
/// Must be called before the first frame is rendered. Effect `n` of the slice is
/// `ShaderEffect::App(n)`. A second call is ignored, so the list has to be the same.
pub fn register_app_effects(effects: &'static [AppEffect]) {
    let _ = APP_EFFECTS.set(effects);
}

/// The effects passed to [`register_app_effects`], empty when none were.
pub fn app_effects() -> &'static [AppEffect] {
    APP_EFFECTS.get().copied().unwrap_or(&[])
}

impl ShaderEffect {
    /// Every effect the engine ships, in index order.
    pub const ALL: [ShaderEffect; 19] = [
        ShaderEffect::GridPattern,
        ShaderEffect::DashedEllipse,
        ShaderEffect::Vignette,
        ShaderEffect::CornerCutout,
        ShaderEffect::RoundedMask,
        ShaderEffect::MarqueeScroll,
        ShaderEffect::WallpaperBlend,
        ShaderEffect::WidgetOutline,
        ShaderEffect::SeekbarWave,
        ShaderEffect::CurtainWipe,
        ShaderEffect::ScreenWipe,
        ShaderEffect::FluidWave,
        ShaderEffect::CircularWaveGauge,
        ShaderEffect::CircularWaveGaugeSmooth,
        ShaderEffect::VisualizerBars,
        ShaderEffect::VisualizerWave,
        ShaderEffect::VisualizerRingWave,
        ShaderEffect::Ghost,
        ShaderEffect::BorderCharge,
    ];

    /// Position of the effect in the renderer's pipeline table: engine effects first, then application ones.
    pub fn index(self) -> usize {
        match self {
            ShaderEffect::App(n) => Self::ALL.len() + n as usize,
            other => Self::ALL.iter().position(|e| *e == other).unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShaderData(pub Arc<[[f32; 4]]>);

impl ShaderData {
    pub fn from_vec4s(values: impl Into<Arc<[[f32; 4]]>>) -> Self {
        ShaderData(values.into())
    }

    pub fn from_floats(values: &[f32]) -> Self {
        let packed: Vec<[f32; 4]> = values
            .chunks(4)
            .map(|chunk| {
                let mut v = [0.0; 4];
                v[..chunk.len()].copy_from_slice(chunk);
                v
            })
            .collect();
        ShaderData(packed.into())
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShaderParams(pub [[f32; 4]; 8]);

impl ShaderParams {
    pub fn pack<const N: usize>(slots: [[f32; 4]; N]) -> Self {
        let mut all = [[0.0; 4]; 8];
        all[..N].copy_from_slice(&slots);
        ShaderParams(all)
    }

    pub fn grid_pattern(step: f32, line_width: f32, color: Color) -> Self {
        Self::pack([[step, line_width, 0.0, 0.0], color.premultiplied(), [0.0; 4], [0.0; 4]])
    }

    pub fn dashed_ellipse(radii: Vec2, dash: Vec2, line_width: f32, color: Color) -> Self {
        Self::pack([[radii.x, radii.y, dash.x, dash.y], [line_width, 0.0, 0.0, 0.0], color.premultiplied(), [0.0; 4]])
    }

    pub fn vignette(color: Color, inner: f32, outer: f32, max_alpha: f32) -> Self {
        Self::pack([color.premultiplied(), [inner, outer, max_alpha, 0.0], [0.0; 4], [0.0; 4]])
    }

    pub fn corner_cutout(corner_index: f32, color: Color) -> Self {
        Self::pack([[corner_index, 0.0, 0.0, 0.0], color.premultiplied(), [0.0; 4], [0.0; 4]])
    }

    pub fn rounded_mask(radii: [f32; 4], fill_alpha: f32) -> Self {
        Self::pack([radii, [fill_alpha, 0.0, 0.0, 0.0], [0.0; 4], [0.0; 4]])
    }

    pub fn marquee_scroll(src_size: Vec2, offset: f32) -> Self {
        Self::pack([[src_size.x, src_size.y, offset, 0.0], [0.0; 4], [0.0; 4], [0.0; 4]])
    }

    pub fn wallpaper_blend(progress: f32, transition: f32, origin: Vec2, params: [f32; 4]) -> Self {
        Self::pack([[progress, transition, origin.x, origin.y], params, [0.0; 4], [0.0; 4]])
    }

    pub fn ghost(body: Color, pupil: Color, shift: Vec2) -> Self {
        Self::pack([[shift.x, shift.y, 0.0, 0.0], body.premultiplied(), pupil.premultiplied(), [0.0; 4]])
    }

    pub fn widget_outline(
        color: Color,
        radius: f32,
        inset: f32,
        line_width: f32,
        amplitude: f32,
        frequency: f32,
        phase: f32,
    ) -> Self {
        Self::pack([
            color.premultiplied(),
            [radius, inset, line_width, amplitude],
            [frequency, phase, 0.0, 0.0],
            [0.0; 4],
        ])
    }

    pub fn border_charge(radius: f32, inset: f32, line_width: f32, progress: f32) -> Self {
        Self::pack([[0.0; 4], [radius, inset, line_width, progress], [0.0; 4], [0.0; 4]])
    }

    pub fn seekbar_wave(
        color: Color,
        geometry: [f32; 4],
        motion_params: [f32; 4],
        shape: [[f32; 4]; 2],
        motion: [[f32; 4]; 2],
    ) -> Self {
        Self::pack([color.premultiplied(), geometry, motion_params, shape[0], shape[1], motion[0], motion[1]])
    }

    pub fn curtain_wipe(reveal: f32, amplitude: f32, colors: [Color; 5]) -> Self {
        Self::pack([
            [reveal, amplitude, 0.0, 0.0],
            colors[0].premultiplied(),
            colors[1].premultiplied(),
            colors[2].premultiplied(),
            colors[3].premultiplied(),
            colors[4].premultiplied(),
        ])
    }

    pub fn screen_wipe(reveal: f32, phase: f32, amplitude: f32, colors: [Color; 5]) -> Self {
        Self::pack([
            [reveal, phase, amplitude, 0.0],
            colors[0].premultiplied(),
            colors[1].premultiplied(),
            colors[2].premultiplied(),
            colors[3].premultiplied(),
            colors[4].premultiplied(),
        ])
    }

    pub fn fluid_wave(
        radius: f32,
        fill_level: f32,
        wave: [f32; 2],
        vertical: bool,
        global_alpha: f32,
        from: Color,
        to: Color,
    ) -> Self {
        Self::pack([
            [radius, fill_level, wave[0], wave[1]],
            [if vertical { 1.0 } else { 0.0 }, global_alpha, 0.0, 0.0],
            from.premultiplied(),
            to.premultiplied(),
        ])
    }

    pub fn circular_wave_gauge(
        fill_ratio: f32,
        stroke_width: f32,
        sine_wave: bool,
        accent: Color,
        params: [f32; 4],
    ) -> Self {
        Self::pack([
            [fill_ratio, stroke_width, if sine_wave { 1.0 } else { 0.0 }, 0.0],
            accent.premultiplied(),
            params,
            [0.0; 4],
        ])
    }

    pub fn circular_wave_gauge_smooth(
        fill_ratio: f32,
        stroke_width: f32,
        sine_wave: bool,
        accent: Color,
        params: [f32; 4],
        track: Color,
    ) -> Self {
        Self::pack([
            [fill_ratio, stroke_width, if sine_wave { 1.0 } else { 0.0 }, 0.0],
            accent.premultiplied(),
            params,
            track.premultiplied(),
        ])
    }

    #[allow(clippy::too_many_arguments)]
    pub fn visualizer_bars(
        color: Color,
        count: usize,
        vertical: bool,
        alignment: f32,
        spacing: f32,
        lengths: [f32; 2],
        radius_ratio: f32,
        opacity: [f32; 2],
        edge_fade: bool,
        ring: [f32; 2],
        tint_strength: f32,
    ) -> Self {
        Self::pack([
            color.premultiplied(),
            [count as f32, if vertical { 1.0 } else { 0.0 }, alignment, spacing],
            [lengths[0], lengths[1], radius_ratio, opacity[0]],
            [opacity[1], if edge_fade { 1.0 } else { 0.0 }, ring[0], ring[1]],
            [tint_strength, 0.0, 0.0, 0.0],
        ])
    }

    pub fn visualizer_wave(color: Color, count: usize, vertical: bool, alignment: f32, max_length: f32) -> Self {
        Self::pack([
            color.premultiplied(),
            [count as f32, if vertical { 1.0 } else { 0.0 }, alignment, max_length],
            [0.0; 4],
            [0.0; 4],
        ])
    }

    pub fn visualizer_ring_wave(
        color: Color,
        count: usize,
        lengths: [f32; 2],
        opacity: [f32; 2],
        ring_radius: f32,
        tint_strength: f32,
    ) -> Self {
        Self::pack([
            color.premultiplied(),
            [count as f32, lengths[0], lengths[1], opacity[0]],
            [opacity[1], ring_radius, tint_strength, 0.0],
            [0.0; 4],
        ])
    }
}
