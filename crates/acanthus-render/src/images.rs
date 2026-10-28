use std::collections::HashMap;

use acanthus_scene::{ImageId, ImageStore};

pub struct GpuImages {
    store: ImageStore,
    layout: wgpu::BindGroupLayout,
    clamped: wgpu::Sampler,
    repeated: wgpu::Sampler,
    entries: HashMap<(ImageId, u32), wgpu::BindGroup>,
}

impl GpuImages {
    pub fn new(device: &wgpu::Device, store: ImageStore) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("acanthus image"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = |mode| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("acanthus image"),
                address_mode_u: mode,
                address_mode_v: mode,
                address_mode_w: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::MipmapFilterMode::Linear,
                ..Default::default()
            })
        };
        GpuImages {
            store,
            layout,
            clamped: sampler(wgpu::AddressMode::ClampToEdge),
            repeated: sampler(wgpu::AddressMode::Repeat),
            entries: HashMap::new(),
        }
    }

    pub fn store(&self) -> &ImageStore {
        &self.store
    }

    pub fn layout(&self) -> &wgpu::BindGroupLayout {
        &self.layout
    }

    pub fn bind_view(&self, device: &wgpu::Device, view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("acanthus layer"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamped) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.repeated) },
            ],
        })
    }

    pub fn collect_garbage(&mut self) {
        let store = &self.store;
        self.entries.retain(|(id, _), _| store.get(*id).is_some());
    }

    pub fn bind_group_cached(&self, id: ImageId, frame: u32) -> Option<&wgpu::BindGroup> {
        self.entries.get(&(id, frame))
    }

    pub fn bind_group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: ImageId,
        frame: u32,
    ) -> Option<&wgpu::BindGroup> {
        if !self.entries.contains_key(&(id, frame)) {
            let decoded = self.store.get(id)?;
            let size = wgpu::Extent3d { width: decoded.width, height: decoded.height, depth_or_array_layers: 1 };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("acanthus image"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                decoded.frame_rgba(frame as usize),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(decoded.width * 4),
                    rows_per_image: Some(decoded.height),
                },
                size,
            );
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("acanthus image"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamped) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.repeated) },
                ],
            });
            self.entries.insert((id, frame), group);
        }
        self.entries.get(&(id, frame))
    }
}
