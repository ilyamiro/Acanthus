use acanthus_core::{Color, Rect, Vec2};
use acanthus_render::{Gpu, Renderer, wgpu};
use acanthus_scene::Scene;
use smithay_client_toolkit::compositor::FrameCallbackData;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer, LayerSurface};
use smithay_client_toolkit::shell::xdg::popup::Popup;
use wayland_client::QueueHandle;
use wayland_client::protocol::{wl_output, wl_surface};

use crate::app::{State, SurfaceId};
use crate::scale::{ScaleObjects, buffer_size};

/// Placement and behaviour of a layer-shell surface.
#[derive(Debug, Clone)]
pub struct LayerConfig {
    /// Namespace the compositor sees.
    pub namespace: String,
    /// Layer the surface lives on.
    pub layer: Layer,
    /// Edges the surface is anchored to.
    pub anchor: Anchor,
    /// Width in logical pixels, 0 to stretch between anchors.
    pub width: u32,
    /// Height in logical pixels, 0 to stretch between anchors.
    pub height: u32,
    /// Space reserved for the surface, -1 to ignore other surfaces' zones.
    pub exclusive_zone: i32,
    /// Margins: top, right, bottom, left.
    pub margin: [i32; 4],
    /// Keyboard focus behaviour.
    pub keyboard: KeyboardInteractivity,
    /// Colour the surface is cleared to before drawing.
    pub clear: Color,
    /// Output the surface is placed on, `None` to let the compositor choose.
    pub output: Option<wl_output::WlOutput>,
}

impl LayerConfig {
    /// Config for `namespace` on the top layer, unanchored, with no reserved space and no keyboard focus.
    pub fn new(namespace: impl Into<String>) -> Self {
        LayerConfig {
            namespace: namespace.into(),
            layer: Layer::Top,
            anchor: Anchor::empty(),
            width: 0,
            height: 0,
            exclusive_zone: 0,
            margin: [0; 4],
            keyboard: KeyboardInteractivity::None,
            clear: Color::TRANSPARENT,
            output: None,
        }
    }

    /// Places the surface on `output`.
    pub fn output(self, output: wl_output::WlOutput) -> Self {
        LayerConfig { output: Some(output), ..self }
    }

    /// Sets the layer.
    pub fn layer(self, layer: Layer) -> Self {
        LayerConfig { layer, ..self }
    }

    /// Sets the anchored edges.
    pub fn anchor(self, anchor: Anchor) -> Self {
        LayerConfig { anchor, ..self }
    }

    /// Sets the size in logical pixels.
    pub fn size(self, width: u32, height: u32) -> Self {
        LayerConfig { width, height, ..self }
    }

    /// Sets the exclusive zone.
    pub fn exclusive_zone(self, exclusive_zone: i32) -> Self {
        LayerConfig { exclusive_zone, ..self }
    }

    /// Sets the margins.
    pub fn margin(self, top: i32, right: i32, bottom: i32, left: i32) -> Self {
        LayerConfig { margin: [top, right, bottom, left], ..self }
    }

    /// Sets the keyboard interactivity.
    pub fn keyboard(self, keyboard: KeyboardInteractivity) -> Self {
        LayerConfig { keyboard, ..self }
    }

    /// Sets the clear colour.
    pub fn clear(self, clear: Color) -> Self {
        LayerConfig { clear, ..self }
    }
}

pub(crate) enum Role {
    Layer(LayerSurface),
    Popup(Popup),
}

impl Role {
    pub fn wl_surface(&self) -> &wl_surface::WlSurface {
        match self {
            Role::Layer(layer) => layer.wl_surface(),
            Role::Popup(popup) => popup.wl_surface(),
        }
    }
}

pub(crate) struct SurfaceWindow {
    pub wgpu_surface: wgpu::Surface<'static>,
    pub renderer: Option<Renderer>,
    pub config: Option<wgpu::SurfaceConfiguration>,
    pub scale_objects: Option<ScaleObjects>,
    pub role: Role,
    pub parent: Option<SurfaceId>,
    pub entered: Vec<wl_output::WlOutput>,
    pub scene: Scene,
    pub clear: Color,
    pub requested: (u32, u32),
    pub size: (u32, u32),
    pub scale: f32,
    pub fractional: bool,
    pub configured: bool,
    pub frame_pending: bool,
    pub dirty: bool,
    pub applied_region: Option<Vec<Rect>>,
    pub applied_keyboard: Option<bool>,
    pub applied_raised: Option<bool>,
    pub applied_geometry: Option<acanthus_scene::LayerGeometry>,
}

impl SurfaceWindow {
    pub fn wl_surface(&self) -> &wl_surface::WlSurface {
        self.role.wl_surface()
    }

    pub fn set_integer_scale(&mut self, factor: i32) -> bool {
        let scale = factor.max(1) as f32;
        if self.fractional || self.scale == scale {
            return false;
        }
        self.scale = scale;
        true
    }

    pub fn set_fractional_scale(&mut self, scale: f32) -> bool {
        let changed = !self.fractional || self.scale != scale;
        self.fractional = true;
        self.scale = scale;
        changed
    }

    pub fn reconfigure(&mut self, gpu: &Gpu) {
        let (w, h) = (self.size.0.max(1), self.size.1.max(1));
        let (bw, bh) = buffer_size((w, h), self.scale);
        let config = gpu.surface_config(&self.wgpu_surface, bw, bh);
        self.wgpu_surface.configure(&gpu.device, &config);
        if self.config.as_ref().is_none_or(|c| c.format != config.format) || self.renderer.is_none() {
            let context = gpu.context(config.format, self.scene.images());
            self.renderer = Some(Renderer::with_context(&gpu.device, context, self.scene.fonts().clone()));
        }
        self.config = Some(config);
        let wl = self.role.wl_surface();
        match (&self.scale_objects, self.fractional) {
            (Some(objects), true) => {
                wl.set_buffer_scale(1);
                objects.viewport.set_destination(w as i32, h as i32);
            }
            _ => wl.set_buffer_scale(self.scale.round().max(1.0) as i32),
        }
        self.scene.set_viewport(Vec2::new(w as f32, h as f32), self.scale);
        self.dirty = true;
    }
    pub fn wants_frame(&self) -> bool {
        self.configured && !self.frame_pending && (self.dirty || self.scene.needs_frame())
    }

    pub fn draw(&mut self, gpu: &Gpu, qh: &QueueHandle<State>) {
        let frame = self.scene.frame();
        if !frame.changed && !self.dirty {
            return;
        }
        let (Some(renderer), Some(config)) = (self.renderer.as_mut(), self.config.as_ref()) else { return };
        let texture = match self.wgpu_surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                tracing::debug!("surface outdated; reconfiguring");
                self.wgpu_surface.configure(&gpu.device, config);
                self.dirty = true;
                return;
            }
            other => {
                tracing::warn!(status = ?std::mem::discriminant(&other), "could not acquire surface texture");
                return;
            }
        };
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        renderer.render(&gpu.device, &gpu.queue, &view, (config.width, config.height), frame.primitives, self.clear);
        let wl = self.role.wl_surface();
        wl.frame(qh, FrameCallbackData(wl.clone()));
        self.frame_pending = true;
        self.dirty = false;
        gpu.queue.present(texture);
    }
}
