use std::cell::RefCell;

use acanthus_scene::ImageStore;

use crate::renderer::GpuContext;

/// Why the GPU could not be set up.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    /// No adapter satisfies the request.
    #[error("no compatible GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    /// The device could not be opened.
    #[error("failed to open GPU device: {0}")]
    Device(#[from] wgpu::RequestDeviceError),
    /// The window surface could not be created.
    #[error("failed to create surface: {0}")]
    Surface(#[from] wgpu::CreateSurfaceError),
}

/// The GPU connection of a process: instance, adapter, device and queue, plus the render contexts shared between renderers.
pub struct Gpu {
    /// The wgpu instance.
    pub instance: wgpu::Instance,
    /// The selected adapter.
    pub adapter: wgpu::Adapter,
    /// The opened device.
    pub device: wgpu::Device,
    /// The command queue.
    pub queue: wgpu::Queue,
    contexts: RefCell<Vec<GpuContext>>,
}

impl Gpu {
    /// Creates an instance on Vulkan, or on the backends named by `WGPU_BACKEND`.
    pub fn instance() -> wgpu::Instance {
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
        if std::env::var_os("WGPU_BACKEND").is_none() {
            desc.backends = wgpu::Backends::VULKAN;
        }
        wgpu::Instance::new(desc)
    }

    /// Picks a low-power adapter compatible with `surface` when one is given and opens a device on it.
    pub async fn new(instance: wgpu::Instance, surface: Option<&wgpu::Surface<'_>>) -> Result<Self, GpuError> {
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                compatible_surface: surface,
                ..Default::default()
            })
            .await?;
        let info = adapter.get_info();
        tracing::info!(name = %info.name, backend = ?info.backend, "selected GPU adapter");
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("acanthus device"),
                required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await?;
        Ok(Gpu { instance, adapter, device, queue, contexts: RefCell::new(Vec::new()) })
    }

    /// The shared render context for `format` and `images`, created on first use and reused by later renderers.
    pub fn context(&self, format: wgpu::TextureFormat, images: &ImageStore) -> GpuContext {
        let mut contexts = self.contexts.borrow_mut();
        if let Some(found) = contexts.iter().find(|c| c.matches(format, images)) {
            return found.clone();
        }
        let context = GpuContext::new(&self.device, format, images.clone());
        contexts.push(context.clone());
        context
    }

    /// A surface configuration of the given size, preferring an 8-bit RGBA or BGRA format and premultiplied alpha when the compositor supports it.
    pub fn surface_config(&self, surface: &wgpu::Surface<'_>, width: u32, height: u32) -> wgpu::SurfaceConfiguration {
        let caps = surface.get_capabilities(&self.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Rgba8Unorm))
            .unwrap_or(caps.formats[0]);
        let alpha_mode = [wgpu::CompositeAlphaMode::PreMultiplied, wgpu::CompositeAlphaMode::Auto]
            .into_iter()
            .find(|m| caps.alpha_modes.contains(m))
            .unwrap_or(caps.alpha_modes[0]);
        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        };
        wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            color_space: wgpu::SurfaceColorSpace::Auto,
            width: width.max(1),
            height: height.max(1),
            present_mode,
            desired_maximum_frame_latency: 2,
            alpha_mode,
            view_formats: vec![],
        }
    }
}
