use std::collections::HashMap;

use acanthus_scene::{ImageId, ImageStore};

pub struct GpuImages {
    store: ImageStore,
    layout: wgpu::BindGroupLayout,
    clamped: wgpu::Sampler,
    repeated: wgpu::Sampler,
    entries: HashMap<(ImageId, u32), (wgpu::BindGroup, bool)>,
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
        self.entries.get(&(id, frame)).map(|(group, _)| group)
    }

    pub fn bind_group(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: ImageId,
        frame: u32,
        mipmap: bool,
    ) -> Option<&wgpu::BindGroup> {
        if self.entries.get(&(id, frame)).is_none_or(|(_, mipped)| mipmap && !mipped) {
            let decoded = self.store.get(id)?;
            let size = wgpu::Extent3d { width: decoded.width, height: decoded.height, depth_or_array_layers: 1 };
            let levels = if mipmap {
                mip_chain(decoded.width, decoded.height, decoded.frame_rgba(frame as usize))
            } else {
                Vec::new()
            };
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("acanthus image"),
                size,
                mip_level_count: levels.len() as u32 + 1,
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
            for (level, (width, height, rgba)) in levels.iter().enumerate() {
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &texture,
                        mip_level: level as u32 + 1,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    rgba,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(*height),
                    },
                    wgpu::Extent3d { width: *width, height: *height, depth_or_array_layers: 1 },
                );
            }
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
            self.entries.insert((id, frame), (group, mipmap));
        }
        self.bind_group_cached(id, frame)
    }
}

fn mip_chain(width: u32, height: u32, rgba: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let mut levels: Vec<(u32, u32, Vec<u8>)> = Vec::new();
    let (mut w, mut h) = (width, height);
    while w > 1 || h > 1 {
        let src = levels.last().map_or(rgba, |(_, _, data)| data.as_slice());
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            let rows = [(2 * y).min(h - 1), (2 * y + 1).min(h - 1)];
            for x in 0..nw {
                let columns = [(2 * x).min(w - 1), (2 * x + 1).min(w - 1)];
                for c in 0..4 {
                    let sum: u32 = rows
                        .iter()
                        .flat_map(|row| {
                            columns.iter().map(move |column| src[((row * w + column) * 4 + c) as usize] as u32)
                        })
                        .sum();
                    next[((y * nw + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        levels.push((nw, nh, next));
        (w, h) = (nw, nh);
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mip_chain_halves_down_to_one_pixel_averaging_each_block() {
        let rgba: Vec<u8> = [[0u8, 0, 0, 0], [255, 255, 255, 255], [255, 255, 255, 255], [0, 0, 0, 0]]
            .into_iter()
            .cycle()
            .take(4 * 2)
            .flatten()
            .collect();
        let levels = mip_chain(4, 2, &rgba);
        let sizes: Vec<(u32, u32)> = levels.iter().map(|(w, h, _)| (*w, *h)).collect();
        assert_eq!(sizes, vec![(2, 1), (1, 1)]);
        assert_eq!(levels[0].2[..4], [128; 4]);
        assert_eq!(levels[1].2, vec![128; 4]);
    }
}
