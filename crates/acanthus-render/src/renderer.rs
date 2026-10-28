use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::rc::Rc;

use acanthus_core::{Affine2, Color, Rect};
use acanthus_scene::{
    Clip, Fonts, Glyph, ImageDraw, ImageId, ImageStore, LayerBegin, LayerFx, Orientation, Primitive, Quad, ShaderDraw,
    ShaderEffect,
};
use bytemuck::{Pod, Zeroable};

use crate::atlas::{GlyphAtlas, Lookup};
use crate::images::GpuImages;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Globals {
    viewport: [f32; 2],
    origin: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BlurParams {
    dir: [f32; 2],
    radius: f32,
    sigma: f32,
    tint: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct AdjustParams {
    color: [f32; 4],
    vals: [f32; 4],
    mask: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Placement {
    transform_matrix: [f32; 4],
    transform_translation: [f32; 4],
    shape_rect: [f32; 4],
    shape_matrix: [f32; 4],
    shape_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct QuadInstance {
    rect: [f32; 4],
    start: [f32; 4],
    end: [f32; 4],
    border_color: [f32; 4],
    params: [f32; 4],
    clip: [f32; 4],
    placement: Placement,
    radii: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GlyphInstance {
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
    clip: [f32; 4],
    params: [f32; 4],
    placement: Placement,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ImageInstance {
    rect: [f32; 4],
    uv: [f32; 4],
    clip: [f32; 4],
    params: [f32; 4],
    placement: Placement,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct EffectInstance {
    rect: [f32; 4],
    clip: [f32; 4],
    meta: [f32; 4],
    params: [[f32; 4]; 8],
    placement: Placement,
}

#[derive(Clone, Copy, PartialEq)]
enum Pass {
    Quads,
    Effect(ShaderEffect, [Option<usize>; 2]),
    Glyphs(usize),
    Image(ImageId, u32),
    Layer(usize),
    Shadow(usize),
}

enum Op {
    Blur(BlurParams),
    Adjust(AdjustParams),
}

struct EffectPass {
    src: usize,
    dst: usize,
    mask: Option<usize>,
    slot: usize,
    op: Op,
}

struct CachedLayer {
    textures: Vec<LayerTexture>,
    result: usize,
    shadow: Option<usize>,
}

struct Segment {
    origin: [i32; 2],
    size: (u32, u32),
    batches: Vec<(Pass, Range<u32>)>,
    fx: LayerFx,
    texture: usize,
    result: usize,
    shadow: Option<usize>,
    passes: Vec<EffectPass>,
    is_mask: bool,
    source: Option<u8>,
    sources: [Option<usize>; 2],
    mask: Option<usize>,
    hasher: DefaultHasher,
    hash: u64,
    cached: bool,
    owned: Vec<usize>,
}

impl Segment {
    fn new(origin: [i32; 2], size: (u32, u32), fx: LayerFx, is_mask: bool, source: Option<u8>) -> Self {
        Segment {
            origin,
            size,
            batches: Vec::new(),
            fx,
            texture: 0,
            result: 0,
            shadow: None,
            passes: Vec::new(),
            is_mask,
            source,
            sources: [None; 2],
            mask: None,
            hasher: DefaultHasher::new(),
            hash: 0,
            cached: false,
            owned: Vec::new(),
        }
    }

    fn finish_hash(&mut self) {
        let fx = &self.fx;
        let mut h = self.hasher.clone();
        self.origin.hash(&mut h);
        self.size.hash(&mut h);
        let shadow = fx.shadow.map(|s| (s.color.premultiplied(), s.blur, s.offset.x, s.offset.y));
        let floats =
            [fx.blur, fx.mask_threshold, fx.mask_spread, fx.brightness, fx.contrast, fx.saturation, fx.colorization];
        for f in floats.iter().chain(fx.colorization_color.premultiplied().iter()) {
            f.to_bits().hash(&mut h);
        }
        fx.masked.hash(&mut h);
        if let Some((c, b, x, y)) = shadow {
            for f in c.iter().chain([b, x, y].iter()) {
                f.to_bits().hash(&mut h);
            }
        }
        self.hash = h.finish();
    }

    fn feed(&mut self, tag: u8, bytes: &[u8]) {
        tag.hash(&mut self.hasher);
        self.hasher.write(bytes);
    }
}

struct LayerTexture {
    size: (u32, u32),
    view: wgpu::TextureView,
    group: wgpu::BindGroup,
}

struct UniformRing {
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    align: u64,
    capacity: u64,
    item: u64,
}

impl UniformRing {
    fn new(device: &wgpu::Device, layout: wgpu::BindGroupLayout, item: u64) -> Self {
        let align = (device.limits().min_uniform_buffer_offset_alignment as u64).max(item);
        let capacity = 8;
        let (buffer, group) = Self::alloc(device, &layout, align, capacity, item);
        UniformRing { buffer, group, layout, align, capacity, item }
    }

    fn alloc(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        align: u64,
        capacity: u64,
        item: u64,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("serp globals"),
            size: align * capacity,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("serp globals"),
            layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &buffer,
                    offset: 0,
                    size: std::num::NonZeroU64::new(item),
                }),
            }],
        });
        (buffer, group)
    }

    fn reserve(&mut self, device: &wgpu::Device, count: u64) {
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            (self.buffer, self.group) = Self::alloc(device, &self.layout, self.align, self.capacity, self.item);
        }
    }

    fn offset(&self, index: usize) -> u32 {
        (self.align * index as u64) as u32
    }
}

struct DataBuffer {
    buffer: wgpu::Buffer,
    group: wgpu::BindGroup,
    layout: wgpu::BindGroupLayout,
    capacity: usize,
}

impl DataBuffer {
    fn new(device: &wgpu::Device, layout: wgpu::BindGroupLayout) -> Self {
        let capacity = 64;
        let (buffer, group) = Self::alloc(device, &layout, capacity);
        DataBuffer { buffer, group, layout, capacity }
    }

    fn alloc(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        capacity: usize,
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("serp effect data"),
            size: (capacity * 16) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("serp effect data"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
        });
        (buffer, group)
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[[f32; 4]]) {
        if data.len() > self.capacity {
            self.capacity = data.len().next_power_of_two();
            (self.buffer, self.group) = Self::alloc(device, &self.layout, self.capacity);
        }
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(data));
        }
    }
}

struct InstanceBuffer {
    buffer: wgpu::Buffer,
    capacity: usize,
    stride: usize,
    label: &'static str,
}

impl InstanceBuffer {
    fn new(device: &wgpu::Device, stride: usize, label: &'static str) -> Self {
        let capacity = 256;
        InstanceBuffer { buffer: Self::alloc(device, capacity * stride, label), capacity, stride, label }
    }

    fn alloc(device: &wgpu::Device, size: usize, label: &'static str) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: size as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, data: &[u8]) {
        let count = data.len() / self.stride;
        if count > self.capacity {
            self.capacity = count.next_power_of_two();
            self.buffer = Self::alloc(device, self.capacity * self.stride, self.label);
        }
        if !data.is_empty() {
            queue.write_buffer(&self.buffer, 0, data);
        }
    }
}

pub(crate) struct Shared {
    format: wgpu::TextureFormat,
    globals_layout: wgpu::BindGroupLayout,
    blur_layout: wgpu::BindGroupLayout,
    data_layout: wgpu::BindGroupLayout,
    blur_pipeline: wgpu::RenderPipeline,
    adjust_pipeline: wgpu::RenderPipeline,
    quad_pipeline: wgpu::RenderPipeline,
    effect_pipelines: Vec<wgpu::RenderPipeline>,
    glyph_pipeline: wgpu::RenderPipeline,
    image_pipeline: wgpu::RenderPipeline,
    blank: LayerTexture,
    images: GpuImages,
    atlas: GlyphAtlas,
}

#[derive(Clone)]
pub struct GpuContext(Rc<RefCell<Shared>>);

impl GpuContext {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, images: ImageStore) -> Self {
        GpuContext(Rc::new(RefCell::new(Shared::new(device, format, images))))
    }

    pub fn matches(&self, format: wgpu::TextureFormat, images: &ImageStore) -> bool {
        let shared = self.0.borrow();
        shared.format == format && shared.images.store().ptr_eq(images)
    }
}

pub struct Renderer {
    ctx: GpuContext,
    fonts: Fonts,
    globals: UniformRing,
    effect_params: UniformRing,
    textures: Vec<LayerTexture>,
    spare: Vec<LayerTexture>,
    cache: HashMap<u64, CachedLayer>,
    reused: usize,
    atlas_generation: u64,
    segments: Vec<Segment>,
    effect_buffer: InstanceBuffer,
    effect_instances: Vec<EffectInstance>,
    effect_data: Vec<[f32; 4]>,
    data_buffer: DataBuffer,
    quad_buffer: InstanceBuffer,
    glyph_buffer: InstanceBuffer,
    image_buffer: InstanceBuffer,
    image_instances: Vec<ImageInstance>,
    quads: Vec<QuadInstance>,
    glyphs: Vec<GlyphInstance>,
    pending: Vec<(u32, Option<u32>)>,
}

impl Shared {
    fn new(device: &wgpu::Device, format: wgpu::TextureFormat, images: ImageStore) -> Self {
        let images = GpuImages::new(device, images);
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("serp globals"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<Globals>() as u64),
                },
                count: None,
            }],
        });
        let blur_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("serp blur"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: true,
                    min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<AdjustParams>() as u64),
                },
                count: None,
            }],
        });
        let atlas_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("serp glyph atlas"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("serp quad"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let data_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("serp effect data"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(16),
                },
                count: None,
            }],
        });
        let effect_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("serp effect"),
            bind_group_layouts: &[
                Some(&globals_layout),
                Some(images.layout()),
                Some(images.layout()),
                Some(&data_layout),
            ],
            immediate_size: 0,
        });
        let blank = make_layer(device, format, &images, (1, 1));
        let glyph_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("serp glyph"),
            bind_group_layouts: &[Some(&globals_layout), Some(&atlas_layout)],
            immediate_size: 0,
        });

        let image_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("serp image"),
            bind_group_layouts: &[Some(&globals_layout), Some(images.layout())],
            immediate_size: 0,
        });
        let image_shader = shader(device, "serp image", include_str!("shaders/image.wgsl"));
        let image_attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4
        ];
        let image_pipeline = pipeline(
            device,
            "serp image",
            &image_layout,
            &image_shader,
            format,
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<ImageInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &image_attrs,
            },
        );

        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("serp blur"),
            bind_group_layouts: &[Some(&blur_layout), Some(images.layout())],
            immediate_size: 0,
        });
        let blur_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("serp blur"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blur.wgsl").into()),
        });
        let blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("serp blur"),
            layout: Some(&blur_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blur_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blur_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let adjust_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("serp adjust"),
            bind_group_layouts: &[Some(&blur_layout), Some(images.layout()), Some(images.layout())],
            immediate_size: 0,
        });
        let adjust_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("serp adjust"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/adjust.wgsl").into()),
        });
        let adjust_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("serp adjust"),
            layout: Some(&adjust_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &adjust_module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &adjust_module,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let quad_shader = shader(device, "serp quad", include_str!("shaders/quad.wgsl"));
        let glyph_shader = shader(device, "serp glyph", include_str!("shaders/glyph.wgsl"));

        let quad_attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
            6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4
        ];
        let glyph_attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4,
            5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4
        ];

        let quad_pipeline = pipeline(
            device,
            "serp quad",
            &quad_layout,
            &quad_shader,
            format,
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<QuadInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &quad_attrs,
            },
        );
        let effect_attrs = wgpu::vertex_attr_array![
            0 => Float32x4, 1 => Float32x4, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4,
            6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4, 10 => Float32x4, 11 => Float32x4,
            12 => Float32x4, 13 => Float32x4, 14 => Float32x4, 15 => Float32x4
        ];
        let builtin = ShaderEffect::ALL.iter().map(|effect| effect_source(*effect));
        let app = acanthus_scene::app_effects().iter().map(|effect| (effect.label, effect.wgsl));
        let effect_pipelines = builtin
            .chain(app)
            .map(|(label, body)| {
                let source = format!("{}\n{}", include_str!("shaders/effect_base.wgsl"), body);
                let module = shader(device, label, &source);
                pipeline(
                    device,
                    label,
                    &effect_layout,
                    &module,
                    format,
                    wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<EffectInstance>() as u64,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &effect_attrs,
                    },
                )
            })
            .collect();
        let glyph_pipeline = pipeline(
            device,
            "serp glyph",
            &glyph_layout,
            &glyph_shader,
            format,
            wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GlyphInstance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &glyph_attrs,
            },
        );

        Shared {
            format,
            globals_layout,
            blur_layout,
            data_layout,
            adjust_pipeline,
            blur_pipeline,
            quad_pipeline,
            effect_pipelines,
            glyph_pipeline,
            image_pipeline,
            blank,
            images,
            atlas: GlyphAtlas::new(&atlas_layout),
        }
    }
}

impl Renderer {
    pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat, fonts: Fonts, images: ImageStore) -> Self {
        Renderer::with_context(device, GpuContext::new(device, format, images), fonts)
    }

    pub fn with_context(device: &wgpu::Device, ctx: GpuContext, fonts: Fonts) -> Self {
        let (globals_layout, blur_layout, data_layout, generation) = {
            let shared = ctx.0.borrow();
            (
                shared.globals_layout.clone(),
                shared.blur_layout.clone(),
                shared.data_layout.clone(),
                shared.atlas.generation(),
            )
        };
        Renderer {
            ctx,
            fonts,
            globals: UniformRing::new(device, globals_layout, std::mem::size_of::<Globals>() as u64),
            effect_params: UniformRing::new(device, blur_layout, std::mem::size_of::<AdjustParams>() as u64),
            atlas_generation: generation,
            segments: Vec::new(),
            textures: Vec::new(),
            spare: Vec::new(),
            cache: HashMap::new(),
            reused: 0,
            effect_buffer: InstanceBuffer::new(device, std::mem::size_of::<EffectInstance>(), "serp effects"),
            effect_instances: Vec::new(),
            effect_data: Vec::new(),
            data_buffer: DataBuffer::new(device, data_layout),
            quad_buffer: InstanceBuffer::new(device, std::mem::size_of::<QuadInstance>(), "serp quads"),
            glyph_buffer: InstanceBuffer::new(device, std::mem::size_of::<GlyphInstance>(), "serp glyphs"),
            image_buffer: InstanceBuffer::new(device, std::mem::size_of::<ImageInstance>(), "serp images"),
            image_instances: Vec::new(),
            quads: Vec::new(),
            glyphs: Vec::new(),
            pending: Vec::new(),
        }
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::TextureView,
        size: (u32, u32),
        primitives: &[Primitive],
        clear: Color,
    ) {
        {
            let mut shared = self.ctx.0.borrow_mut();
            shared.images.collect_garbage();
            shared.atlas.begin_frame();
        }
        if !self.prepare(device, queue, primitives, size) {
            tracing::debug!("glyph atlas full; rebuilding");
            self.ctx.0.borrow_mut().atlas.clear();
            self.prepare(device, queue, primitives, size);
        }
        let generation = self.ctx.0.borrow().atlas.generation();
        if generation != self.atlas_generation {
            self.atlas_generation = generation;
            self.cache.clear();
        }
        self.plan_textures(device);
        let ctx = self.ctx.clone();
        let shared = ctx.0.borrow();

        self.globals.reserve(device, self.segments.len() as u64);
        for (index, segment) in self.segments.iter().enumerate() {
            let globals = Globals {
                viewport: [segment.size.0 as f32, segment.size.1 as f32],
                origin: [segment.origin[0] as f32, segment.origin[1] as f32],
            };
            queue.write_buffer(&self.globals.buffer, self.globals.offset(index) as u64, bytemuck::bytes_of(&globals));
        }
        let effect_passes = self.segments.iter().map(|s| s.passes.len() as u64).sum();
        self.effect_params.reserve(device, effect_passes);
        for pass in self.segments.iter().flat_map(|s| &s.passes) {
            let offset = self.effect_params.offset(pass.slot) as u64;
            match &pass.op {
                Op::Blur(p) => queue.write_buffer(&self.effect_params.buffer, offset, bytemuck::bytes_of(p)),
                Op::Adjust(p) => queue.write_buffer(&self.effect_params.buffer, offset, bytemuck::bytes_of(p)),
            }
        }
        self.quad_buffer.upload(device, queue, bytemuck::cast_slice(&self.quads));
        self.effect_buffer.upload(device, queue, bytemuck::cast_slice(&self.effect_instances));
        self.data_buffer.upload(device, queue, &self.effect_data);
        self.glyph_buffer.upload(device, queue, bytemuck::cast_slice(&self.glyphs));
        self.image_buffer.upload(device, queue, bytemuck::cast_slice(&self.image_instances));

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("serp frame") });
        for index in (0..self.segments.len()).rev() {
            if self.segments[index].cached {
                continue;
            }
            let (view, color) = if index == 0 {
                (target, clear)
            } else {
                (&self.textures[self.segments[index].texture].view, Color::TRANSPARENT)
            };
            let mut pass = begin_pass(&mut encoder, view, color);
            pass.set_bind_group(0, &self.globals.group, &[self.globals.offset(index)]);
            for (kind, range) in &self.segments[index].batches {
                match kind {
                    Pass::Quads => {
                        pass.set_pipeline(&shared.quad_pipeline);
                        pass.set_vertex_buffer(0, self.quad_buffer.buffer.slice(..));
                    }
                    Pass::Effect(effect, sources) => {
                        pass.set_pipeline(&shared.effect_pipelines[effect.index()]);
                        for (slot, source) in sources.iter().enumerate() {
                            let group = match source {
                                Some(i) => &self.textures[self.segments[*i].result].group,
                                None => &shared.blank.group,
                            };
                            pass.set_bind_group(slot as u32 + 1, group, &[]);
                        }
                        pass.set_bind_group(3, &self.data_buffer.group, &[]);
                        pass.set_vertex_buffer(0, self.effect_buffer.buffer.slice(..));
                    }
                    Pass::Glyphs(page) => {
                        pass.set_pipeline(&shared.glyph_pipeline);
                        pass.set_bind_group(1, &shared.atlas.page(*page).bind_group, &[]);
                        pass.set_vertex_buffer(0, self.glyph_buffer.buffer.slice(..));
                    }
                    Pass::Image(id, frame) => {
                        let Some(group) = shared.images.bind_group_cached(*id, *frame) else { continue };
                        pass.set_pipeline(&shared.image_pipeline);
                        pass.set_bind_group(1, group, &[]);
                        pass.set_vertex_buffer(0, self.image_buffer.buffer.slice(..));
                    }
                    Pass::Layer(segment) => {
                        pass.set_pipeline(&shared.image_pipeline);
                        pass.set_bind_group(1, &self.textures[self.segments[*segment].result].group, &[]);
                        pass.set_vertex_buffer(0, self.image_buffer.buffer.slice(..));
                    }
                    Pass::Shadow(segment) => {
                        let Some(texture) = self.segments[*segment].shadow else { continue };
                        pass.set_pipeline(&shared.image_pipeline);
                        pass.set_bind_group(1, &self.textures[texture].group, &[]);
                        pass.set_vertex_buffer(0, self.image_buffer.buffer.slice(..));
                    }
                }
                pass.draw(0..4, range.clone());
            }
            drop(pass);
            for effect in &self.segments[index].passes {
                let mut pass = begin_pass(&mut encoder, &self.textures[effect.dst].view, Color::TRANSPARENT);
                pass.set_bind_group(0, &self.effect_params.group, &[self.effect_params.offset(effect.slot)]);
                pass.set_bind_group(1, &self.textures[effect.src].group, &[]);
                match &effect.op {
                    Op::Blur(_) => pass.set_pipeline(&shared.blur_pipeline),
                    Op::Adjust(_) => {
                        pass.set_pipeline(&shared.adjust_pipeline);
                        let mask = effect.mask.unwrap_or(effect.src);
                        pass.set_bind_group(2, &self.textures[mask].group, &[]);
                    }
                }
                pass.draw(0..4, 0..1);
            }
        }
        queue.submit(Some(encoder.finish()));
        self.store_cache();
    }

    pub fn reused_layers(&self) -> usize {
        self.reused
    }

    fn store_cache(&mut self) {
        let mut all: Vec<Option<LayerTexture>> = self.textures.drain(..).map(Some).collect();
        self.cache.clear();
        for segment in self.segments.iter().skip(1) {
            let position = |texture: usize| segment.owned.iter().position(|&i| i == texture).unwrap_or(0);
            let textures: Vec<LayerTexture> = segment.owned.iter().filter_map(|&i| all[i].take()).collect();
            if textures.len() != segment.owned.len() {
                continue;
            }
            let entry =
                CachedLayer { textures, result: position(segment.result), shadow: segment.shadow.map(position) };
            self.cache.insert(segment.hash, entry);
        }
    }

    fn take_texture(&mut self, device: &wgpu::Device, size: (u32, u32), owner: usize) -> usize {
        let texture = match self.spare.iter().position(|t| t.size == size) {
            Some(i) => self.spare.swap_remove(i),
            None => self.create_layer(device, size),
        };
        self.textures.push(texture);
        let index = self.textures.len() - 1;
        self.segments[owner].owned.push(index);
        index
    }

    fn plan_textures(&mut self, device: &wgpu::Device) {
        let mut cache = std::mem::take(&mut self.cache);
        for index in 1..self.segments.len() {
            let segment = &mut self.segments[index];
            segment.owned.clear();
            segment.passes.clear();
            segment.shadow = None;
            if let Some(entry) = cache.remove(&segment.hash) {
                let base = self.textures.len();
                segment.owned.extend(base..base + entry.textures.len());
                segment.texture = base;
                segment.result = base + entry.result;
                segment.shadow = entry.shadow.map(|s| base + s);
                segment.cached = true;
                self.textures.extend(entry.textures);
            } else {
                segment.cached = false;
            }
        }
        self.reused = self.segments.iter().filter(|s| s.cached).count();
        self.spare = cache.into_values().flat_map(|c| c.textures).collect();
        let mut slot = 0;
        for index in 1..self.segments.len() {
            if self.segments[index].cached {
                continue;
            }
            let size = self.segments[index].size;
            let texture = self.take_texture(device, size, index);
            self.segments[index].texture = texture;
            self.segments[index].result = texture;
        }
        for index in 1..self.segments.len() {
            if self.segments[index].cached {
                continue;
            }
            let fx = self.segments[index].fx;
            let size = self.segments[index].size;
            let mut current = self.segments[index].texture;
            if let Some(mask) = self.segments[index].mask {
                let mask_texture = self.segments[mask].result;
                let target = self.take_texture(device, size, index);
                let params = AdjustParams {
                    color: [0.0; 4],
                    vals: [0.0; 4],
                    mask: [1.0, fx.mask_threshold, fx.mask_spread, 0.0],
                };
                self.segments[index].passes.push(effect_pass(
                    current,
                    target,
                    Some(mask_texture),
                    &mut slot,
                    Op::Adjust(params),
                ));
                current = target;
            }
            let masked = current;
            if fx.blur > 0.0 {
                let middle = self.take_texture(device, size, index);
                let result = self.take_texture(device, size, index);
                let segment = &mut self.segments[index];
                segment.passes.push(effect_pass(current, middle, None, &mut slot, blur_op([1.0, 0.0], fx.blur, None)));
                segment.passes.push(effect_pass(middle, result, None, &mut slot, blur_op([0.0, 1.0], fx.blur, None)));
                current = result;
            }
            if fx.adjusts() {
                let target = self.take_texture(device, size, index);
                let color = fx.colorization_color.premultiplied();
                let params = AdjustParams {
                    color: [color[0], color[1], color[2], color[3]],
                    vals: [fx.brightness, fx.contrast, fx.saturation, fx.colorization],
                    mask: [0.0; 4],
                };
                self.segments[index].passes.push(effect_pass(current, target, None, &mut slot, Op::Adjust(params)));
                current = target;
            }
            if let Some(shadow) = fx.shadow {
                let middle = self.take_texture(device, size, index);
                let result = self.take_texture(device, size, index);
                let tint = shadow.color.premultiplied();
                let segment = &mut self.segments[index];
                segment.passes.push(effect_pass(
                    masked,
                    middle,
                    None,
                    &mut slot,
                    blur_op([1.0, 0.0], shadow.blur, None),
                ));
                segment.passes.push(effect_pass(
                    middle,
                    result,
                    None,
                    &mut slot,
                    blur_op([0.0, 1.0], shadow.blur, Some(tint)),
                ));
                segment.shadow = Some(result);
            }
            self.segments[index].result = current;
        }
        self.spare.clear();
    }

    fn create_layer(&self, device: &wgpu::Device, size: (u32, u32)) -> LayerTexture {
        {
            let shared = self.ctx.0.borrow();
            make_layer(device, shared.format, &shared.images, size)
        }
    }

    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        primitives: &[Primitive],
        size: (u32, u32),
    ) -> bool {
        let ctx = self.ctx.clone();
        let mut shared = ctx.0.borrow_mut();
        self.quads.clear();
        self.effect_instances.clear();
        self.effect_data.clear();
        self.image_instances.clear();
        self.glyphs.clear();
        self.segments.clear();
        self.segments.push(Segment::new([0, 0], size, LayerFx::default(), false, None));
        let mut open = vec![0usize];
        let mut fonts = self.fonts.system();
        for prim in primitives {
            let current = *open.last().unwrap();
            let pass = match prim {
                Primitive::Quad(q) => {
                    let instance = quad_instance(q);
                    self.segments[current].feed(0, bytemuck::bytes_of(&instance));
                    self.quads.push(instance);
                    Pass::Quads
                }
                Primitive::Shader(d) => {
                    let instance = effect_instance(d, self.effect_data.len());
                    self.segments[current].feed(2, bytemuck::bytes_of(&instance));
                    if !d.data.is_empty() {
                        self.segments[current].feed(9, bytemuck::cast_slice(&d.data.0));
                        self.effect_data.extend_from_slice(&d.data.0);
                    }
                    d.effect.hash(&mut self.segments[current].hasher);
                    let mut sources = [None; 2];
                    for (slot, used) in d.sources.iter().enumerate() {
                        if *used && let Some(index) = self.segments[current].sources[slot].take() {
                            let hash = self.segments[index].hash;
                            self.segments[current].feed(8, &hash.to_le_bytes());
                            sources[slot] = Some(index);
                        }
                    }
                    self.effect_instances.push(instance);
                    Pass::Effect(d.effect, sources)
                }
                Primitive::Image(d) => {
                    if shared.images.bind_group(device, queue, d.id, d.frame).is_none() {
                        continue;
                    }
                    let instance = image_instance(d);
                    self.segments[current].feed(1, bytemuck::bytes_of(&instance));
                    d.id.hash(&mut self.segments[current].hasher);
                    d.frame.hash(&mut self.segments[current].hasher);
                    self.image_instances.push(instance);
                    Pass::Image(d.id, d.frame)
                }
                Primitive::Glyph(g) => match shared.atlas.get(device, queue, &mut fonts, g.key) {
                    Lookup::Hit(entry) => {
                        let instance = glyph_instance(g, &entry);
                        self.segments[current].feed(3, bytemuck::bytes_of(&instance));
                        self.glyphs.push(instance);
                        Pass::Glyphs(entry.page)
                    }
                    Lookup::Empty => continue,
                    Lookup::Full => return false,
                },
                Primitive::LayerBegin(begin) => {
                    let index = self.segments.len();
                    self.segments.push(Segment::new(
                        [begin.bounds.x as i32, begin.bounds.y as i32],
                        (begin.bounds.w as u32, begin.bounds.h as u32),
                        begin.fx,
                        begin.is_mask,
                        begin.source,
                    ));
                    let shadow = begin.fx.shadow.map(|shadow| {
                        let offset = acanthus_core::Vec2::new(shadow.offset.x, shadow.offset.y);
                        let mut instance = layer_instance(begin, begin.clip);
                        instance.rect = rect4(&begin.bounds.translate(offset));
                        self.image_instances.push(instance);
                        self.image_instances.len() as u32 - 1
                    });
                    let content_clip = if begin.fx.is_none() { Clip::rect(begin.bounds) } else { begin.clip };
                    self.image_instances.push(layer_instance(begin, content_clip));
                    self.pending.push((self.image_instances.len() as u32 - 1, shadow));
                    open.push(index);
                    continue;
                }
                Primitive::LayerEnd => {
                    let index = open.pop().unwrap_or(0);
                    if index == 0 {
                        open.push(0);
                        continue;
                    }
                    let (content, shadow) = self.pending.pop().unwrap();
                    self.segments[index].finish_hash();
                    let hash = self.segments[index].hash;
                    let parent = *open.last().unwrap();
                    if let Some(slot) = self.segments[index].source {
                        self.segments[parent].sources[slot as usize] = Some(index);
                        continue;
                    }
                    if self.segments[index].is_mask {
                        self.segments[parent].mask = Some(index);
                        self.segments[parent].feed(4, &hash.to_le_bytes());
                        continue;
                    }
                    let instance = self.image_instances[content as usize];
                    self.segments[parent].feed(5, &hash.to_le_bytes());
                    self.segments[parent].feed(6, bytemuck::bytes_of(&instance));
                    if let Some(shadow) = shadow {
                        let instance = self.image_instances[shadow as usize];
                        self.segments[parent].feed(7, bytemuck::bytes_of(&instance));
                    }
                    let batches = &mut self.segments[parent].batches;
                    if let Some(shadow) = shadow {
                        batches.push((Pass::Shadow(index), shadow..shadow + 1));
                    }
                    batches.push((Pass::Layer(index), content..content + 1));
                    continue;
                }
            };
            let index = match pass {
                Pass::Quads => self.quads.len() as u32 - 1,
                Pass::Effect(..) => self.effect_instances.len() as u32 - 1,
                Pass::Glyphs(_) => self.glyphs.len() as u32 - 1,
                Pass::Image(..) | Pass::Layer(_) | Pass::Shadow(_) => self.image_instances.len() as u32 - 1,
            };
            let batches = &mut self.segments[current].batches;
            match batches.last_mut() {
                Some((last, range)) if *last == pass && !matches!(pass, Pass::Image(..)) => range.end = index + 1,
                _ => batches.push((pass, index..index + 1)),
            }
        }
        true
    }
}

fn rect4(r: &Rect) -> [f32; 4] {
    [r.x, r.y, r.w, r.h]
}

fn matrix4(t: &Affine2) -> [f32; 4] {
    [t.matrix2.x_axis.x, t.matrix2.x_axis.y, t.matrix2.y_axis.x, t.matrix2.y_axis.y]
}

fn placement(transform: &Affine2, clip: &Clip) -> Placement {
    let scale = transform.matrix2.determinant().abs().sqrt();
    let (shape_rect, shape_matrix, shape_params) = match &clip.shape {
        Some(shape) => {
            let inverse = &shape.inverse;
            let shape_scale = 1.0 / inverse.matrix2.determinant().abs().sqrt().max(f32::EPSILON);
            (
                rect4(&shape.rect),
                matrix4(inverse),
                [inverse.translation.x, inverse.translation.y, shape.radius, shape_scale],
            )
        }
        None => ([0.0; 4], [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, -1.0, 1.0]),
    };
    Placement {
        transform_matrix: matrix4(transform),
        transform_translation: [transform.translation.x, transform.translation.y, scale, 0.0],
        shape_rect,
        shape_matrix,
        shape_params,
    }
}

fn shader(device: &wgpu::Device, label: &str, source: &str) -> wgpu::ShaderModule {
    let source = format!("{}\n{}", include_str!("shaders/common.wgsl"), source);
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

fn quad_instance(q: &Quad) -> QuadInstance {
    let mode = match q.orientation {
        None => 0.0,
        Some(Orientation::Vertical) => 1.0,
        Some(Orientation::Horizontal) => 2.0,
    };
    QuadInstance {
        rect: rect4(&q.rect),
        start: q.start.premultiplied(),
        end: q.end.premultiplied(),
        border_color: q.border_color.premultiplied(),
        params: [q.radius, q.border_width, mode, 0.0],
        clip: rect4(&q.clip.rect),
        placement: placement(&q.transform, &q.clip),
        radii: q.radii,
    }
}

fn effect_source(effect: ShaderEffect) -> (&'static str, &'static str) {
    match effect {
        ShaderEffect::GridPattern => ("serp grid pattern", include_str!("shaders/effects/grid_pattern.wgsl")),
        ShaderEffect::DashedEllipse => ("serp dashed ellipse", include_str!("shaders/effects/dashed_ellipse.wgsl")),
        ShaderEffect::Vignette => ("serp vignette", include_str!("shaders/effects/vignette.wgsl")),
        ShaderEffect::CornerCutout => ("serp corner cutout", include_str!("shaders/effects/corner_cutout.wgsl")),
        ShaderEffect::RoundedMask => ("serp rounded mask", include_str!("shaders/effects/rounded_mask.wgsl")),
        ShaderEffect::MarqueeScroll => ("serp marquee scroll", include_str!("shaders/effects/marquee_scroll.wgsl")),
        ShaderEffect::WallpaperBlend => ("serp wallpaper blend", include_str!("shaders/effects/wallpaper_blend.wgsl")),
        ShaderEffect::Ghost => ("serp ghost", include_str!("shaders/effects/ghost.wgsl")),
        ShaderEffect::BorderCharge => ("serp border_charge", include_str!("shaders/effects/border_charge.wgsl")),
        ShaderEffect::WidgetOutline => ("serp widget_outline", include_str!("shaders/effects/widget_outline.wgsl")),
        ShaderEffect::SeekbarWave => ("serp seekbar_wave", include_str!("shaders/effects/seekbar_wave.wgsl")),
        ShaderEffect::CurtainWipe => ("serp curtain_wipe", include_str!("shaders/effects/curtain_wipe.wgsl")),
        ShaderEffect::ScreenWipe => ("serp screen_wipe", include_str!("shaders/effects/screen_wipe.wgsl")),
        ShaderEffect::FluidWave => ("serp fluid_wave", include_str!("shaders/effects/fluid_wave.wgsl")),
        ShaderEffect::CircularWaveGauge => {
            ("serp circular_wave_gauge", include_str!("shaders/effects/circular_wave_gauge.wgsl"))
        }
        ShaderEffect::CircularWaveGaugeSmooth => {
            ("serp circular_wave_gauge_smooth", include_str!("shaders/effects/circular_wave_gauge_smooth.wgsl"))
        }
        ShaderEffect::VisualizerBars => ("serp visualizer_bars", include_str!("shaders/effects/visualizer_bars.wgsl")),
        ShaderEffect::VisualizerWave => ("serp visualizer_wave", include_str!("shaders/effects/visualizer_wave.wgsl")),
        ShaderEffect::VisualizerRingWave => {
            ("serp visualizer_ring_wave", include_str!("shaders/effects/visualizer_ring_wave.wgsl"))
        }
        ShaderEffect::App(_) => ("", ""),
    }
}

fn make_layer(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    images: &GpuImages,
    size: (u32, u32),
) -> LayerTexture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("serp layer"),
        size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let group = images.bind_view(device, &view);
    LayerTexture { size, view, group }
}

fn effect_instance(d: &ShaderDraw, data_offset: usize) -> EffectInstance {
    EffectInstance {
        rect: rect4(&d.rect),
        clip: rect4(&d.clip.rect),
        meta: [d.opacity, d.item_size.x, d.item_size.y, data_offset as f32],
        params: d.params.0,
        placement: placement(&d.transform, &d.clip),
    }
}

fn image_instance(d: &ImageDraw) -> ImageInstance {
    ImageInstance {
        rect: rect4(&d.rect),
        uv: d.uv,
        clip: rect4(&d.clip.rect),
        params: [d.opacity, if d.repeat { 1.0 } else { 0.0 }, 0.0, 0.0],
        placement: placement(&d.transform, &d.clip),
    }
}

fn layer_instance(begin: &LayerBegin, clip: Clip) -> ImageInstance {
    ImageInstance {
        rect: rect4(&begin.bounds),
        uv: [0.0, 0.0, 1.0, 1.0],
        clip: rect4(&clip.rect),
        params: [begin.opacity, 0.0, 0.0, 0.0],
        placement: placement(&Affine2::IDENTITY, &clip),
    }
}

fn blur_op(dir: [f32; 2], radius: f32, tint: Option<[f32; 4]>) -> Op {
    Op::Blur(BlurParams { dir, radius, sigma: (radius / 2.0).max(0.5), tint: tint.unwrap_or([0.0, 0.0, 0.0, -1.0]) })
}

fn effect_pass(src: usize, dst: usize, mask: Option<usize>, slot: &mut usize, op: Op) -> EffectPass {
    *slot += 1;
    EffectPass { src, dst, mask, slot: *slot - 1, op }
}

fn begin_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    view: &'a wgpu::TextureView,
    clear: Color,
) -> wgpu::RenderPass<'a> {
    let [r, g, b, a] = clear.premultiplied();
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("serp frame"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color { r: r as f64, g: g as f64, b: b as f64, a: a as f64 }),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}

fn glyph_instance(g: &Glyph, e: &crate::atlas::AtlasGlyph) -> GlyphInstance {
    GlyphInstance {
        rect: [(g.x + e.left) as f32, (g.y - e.top) as f32, e.width as f32, e.height as f32],
        uv: [e.x as f32, e.y as f32, e.width as f32, e.height as f32],
        color: g.color.premultiplied(),
        clip: rect4(&g.clip.rect),
        params: [
            if e.color { 1.0 } else { 0.0 },
            if g.transform == Affine2::IDENTITY { 0.0 } else { 1.0 },
            e.page as f32,
            0.0,
        ],
        placement: placement(&g.transform, &g.clip),
    }
}

fn pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    format: wgpu::TextureFormat,
    instances: wgpu::VertexBufferLayout<'_>,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[Some(instances)],
        },
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleStrip, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    })
}
