use acanthus_core::{Color, Vec2};
use acanthus_render::{Gpu, Renderer, wgpu};
use acanthus_scene::{
    Anchors, Fonts, LayerEffect, Node, Scene, ShaderData, ShaderEffect, ShaderParams, TransformOrigin,
};

const W: u32 = 128;
const H: u32 = 64;

fn gpu() -> Option<Gpu> {
    match pollster::block_on(Gpu::new(Gpu::instance(), None)) {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

fn render(gpu: &Gpu, scene: &mut Scene, fonts: Fonts) -> Vec<u8> {
    render_frames(gpu, scene, fonts, 1).0
}

fn render_frames(gpu: &Gpu, scene: &mut Scene, fonts: Fonts, frames: usize) -> (Vec<u8>, usize) {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    let context = gpu.context(format, scene.images());
    let mut renderer = Renderer::with_context(&gpu.device, context, fonts);
    scene.set_viewport(Vec2::new(W as f32, H as f32), 1.0);
    for _ in 0..frames {
        let frame = scene.frame();
        renderer.render(&gpu.device, &gpu.queue, &view, (W, H), frame.primitives, Color::TRANSPARENT);
    }
    let reused = renderer.reused_layers();

    let row = (W * 4).next_multiple_of(256);
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: (row * H) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Default::default(),
            aspect: Default::default(),
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(H) },
        },
        wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
    );
    gpu.queue.submit(Some(encoder.finish()));
    buffer.map_async(wgpu::MapMode::Read, .., |r| r.unwrap());
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let data = buffer.get_mapped_range(..).unwrap();
    let mut out = Vec::with_capacity((W * H * 4) as usize);
    for y in 0..H {
        let start = (y * row) as usize;
        out.extend_from_slice(&data[start..start + (W * 4) as usize]);
    }
    (out, reused)
}

fn pixel(img: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * W + x) * 4) as usize;
    [img[i], img[i + 1], img[i + 2], img[i + 3]]
}

#[test]
fn renders_rounded_rect_border_and_text() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(
        root,
        Node::rect()
            .size(60.0, 40.0)
            .x(4.0)
            .y(4.0)
            .radius(12.0)
            .color(Color::hex(0xff0000))
            .border(3.0, Color::hex(0x0000ff)),
    );
    scene.add(
        root,
        Node::text("Hi").font_size(24.0).color(Color::WHITE).anchors(Anchors::default().right(4.0).v_center()),
    );
    let img = render(&gpu, &mut scene, fonts);

    assert_eq!(pixel(&img, 34, 24), [255, 0, 0, 255], "fill");
    assert_eq!(pixel(&img, 5, 5)[3], 0, "rounded corner is transparent");
    let border = pixel(&img, 5, 24);
    assert!(border[2] > 200 && border[0] < 50, "border {border:?}");
    assert_eq!(pixel(&img, 66, 2), [0, 0, 0, 0], "background");

    let text_pixels =
        (70..W).flat_map(|x| (0..H).map(move |y| (x, y))).filter(|&(x, y)| pixel(&img, x, y)[3] > 128).count();
    assert!(text_pixels > 20, "text pixels: {text_pixels}");
}

#[test]
fn surfaces_share_one_context_and_draw_the_same_pixels() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(root, Node::text("Hi").font_size(24.0).color(Color::WHITE));
    let first = render(&gpu, &mut scene, fonts.clone());
    let second = render(&gpu, &mut scene, fonts);
    assert_eq!(first, second);
    assert!(first.chunks(4).any(|p| p[3] > 128));
}

#[test]
fn clip_and_gradient() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let clipper = scene.add(root, Node::item().size(32.0, 32.0).clip(true));
    scene.add(
        clipper,
        Node::rect().size(128.0, 64.0).gradient(acanthus_scene::Gradient::horizontal(Color::BLACK, Color::WHITE)),
    );
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 40, 10)[3], 0, "clipped");
    let left = pixel(&img, 1, 10)[0];
    let right = pixel(&img, 30, 10)[0];
    assert!(right > left, "gradient {left} -> {right}");
}

#[test]
fn nested_rounded_clips_are_both_exact() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let outer = scene.add(root, Node::rect().size(60.0, 60.0).radius(30.0).clip(true));
    let inner = scene.add(outer, Node::item().size(60.0, 60.0).rotation(0.001).clip(true));
    scene.add(inner, Node::rect().size(60.0, 60.0).color(Color::hex(0xff0000)));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 30, 30), [255, 0, 0, 255], "inside both");
    assert_eq!(pixel(&img, 5, 5)[3], 0, "outside the outer circle");
    assert_eq!(pixel(&img, 54, 54)[3], 0, "outside the outer circle");
}

#[test]
fn scaled_text_is_as_crisp_as_text_of_the_scaled_size() {
    let Some(gpu) = gpu() else { return };
    let solid = |node: Node| {
        let fonts = Fonts::new();
        let mut scene = Scene::new(fonts.clone());
        let root = scene.root();
        scene.add(root, node.color(Color::WHITE));
        let img = render(&gpu, &mut scene, fonts);
        img.chunks(4).filter(|p| p[3] > 250).count()
    };
    let native = solid(Node::text("Hi").font_size(32.0));
    let scaled = solid(Node::text("Hi").font_size(16.0).scale(2.0).transform_origin(TransformOrigin::TopLeft));
    assert!(native > 50, "native {native}");
    assert!(scaled * 10 >= native * 9, "scaled {scaled} native {native}");
}

#[test]
fn draws_decoded_images() {
    let Some(gpu) = gpu() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("red.png");
    image::RgbaImage::from_pixel(8, 8, image::Rgba([255, 0, 0, 255])).save(&path).unwrap();
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(root, Node::image(path.to_str().unwrap()).size(32.0, 32.0));
    scene.set_viewport(Vec2::new(W as f32, H as f32), 1.0);
    scene.frame();
    let start = std::time::Instant::now();
    while !scene.images().poll() && start.elapsed() < std::time::Duration::from_secs(5) {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 10, 10), [255, 0, 0, 255], "inside the image");
    assert_eq!(pixel(&img, 40, 10)[3], 0, "outside the image");
}

#[test]
fn rotated_and_scaled_rects() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(root, Node::rect().x(44.0).y(27.0).size(40.0, 10.0).rotation(90.0).color(Color::hex(0xff0000)));
    scene.add(
        root,
        Node::rect()
            .x(10.0)
            .y(10.0)
            .size(10.0, 10.0)
            .scale(2.0)
            .transform_origin(TransformOrigin::TopLeft)
            .color(Color::hex(0x00ff00)),
    );
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 64, 15), [255, 0, 0, 255], "rotated bar covers the vertical");
    assert_eq!(pixel(&img, 64, 49), [255, 0, 0, 255], "rotated bar bottom");
    assert_eq!(pixel(&img, 48, 32)[3], 0, "unrotated extent is empty");
    assert_eq!(pixel(&img, 28, 28), [0, 255, 0, 255], "scaled rect grows from its top left");
    assert_eq!(pixel(&img, 31, 31)[3], 0, "scaled rect ends at twice its size");
}

#[test]
fn per_corner_radii_round_only_the_chosen_corners() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(
        root,
        Node::rect()
            .x(10.0)
            .y(10.0)
            .size(40.0, 40.0)
            .corner_radii(acanthus_core::Signal::new(Some([16.0, 0.0, 0.0, 0.0])))
            .color(Color::hex(0xff0000)),
    );
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 11, 11)[3], 0, "top left corner is cut");
    assert_eq!(pixel(&img, 48, 11), [255, 0, 0, 255], "top right stays square");
    assert_eq!(pixel(&img, 11, 48), [255, 0, 0, 255], "bottom left stays square");
    assert_eq!(pixel(&img, 48, 48), [255, 0, 0, 255], "bottom right stays square");
}

#[test]
fn rounded_and_rotated_clips() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let rounded =
        scene.add(root, Node::rect().x(4.0).y(4.0).size(40.0, 40.0).radius(16.0).color(Color::TRANSPARENT).clip(true));
    scene.add(rounded, Node::rect().x(-10.0).y(-10.0).size(128.0, 64.0).color(Color::hex(0xff0000)));
    let turned = scene.add(root, Node::item().x(80.0).y(16.0).size(32.0, 32.0).rotation(45.0).clip(true));
    scene.add(turned, Node::rect().x(-40.0).y(-40.0).size(128.0, 128.0).color(Color::hex(0x0000ff)));
    let img = render(&gpu, &mut scene, fonts);

    assert_eq!(pixel(&img, 24, 24), [255, 0, 0, 255], "inside the rounded clip");
    assert_eq!(pixel(&img, 5, 24), [255, 0, 0, 255], "edge middle is inside");
    assert_eq!(pixel(&img, 6, 6)[3], 0, "rounded corner is clipped");
    assert_eq!(pixel(&img, 50, 10)[3], 0, "outside the clip");

    assert_eq!(pixel(&img, 96, 32), [0, 0, 255, 255], "centre of the diamond");
    assert_eq!(pixel(&img, 96, 12), [0, 0, 255, 255], "top tip of the diamond");
    assert_eq!(pixel(&img, 82, 18)[3], 0, "unrotated corner is clipped");
}

#[test]
fn rotated_text_still_draws() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(root, Node::text("Hi").font_size(24.0).color(Color::WHITE).x(80.0).y(16.0).rotation(180.0));
    let img = render(&gpu, &mut scene, fonts);
    let text_pixels =
        (64..W).flat_map(|x| (0..H).map(move |y| (x, y))).filter(|&(x, y)| pixel(&img, x, y)[3] > 128).count();
    assert!(text_pixels > 20, "text pixels: {text_pixels}");
}

fn overlapping_pair(scene: &mut Scene, layered: bool) {
    let root = scene.root();
    let group = scene.add(root, Node::item().x(20.0).y(10.0).size(60.0, 40.0).opacity(0.5).layer(layered));
    scene.add(group, Node::rect().size(40.0, 40.0).color(Color::hex(0xff0000)));
    scene.add(group, Node::rect().x(20.0).size(40.0, 40.0).color(Color::hex(0x0000ff)));
}

#[test]
fn layer_gives_true_group_opacity() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();

    let mut plain = Scene::new(fonts.clone());
    overlapping_pair(&mut plain, false);
    let img = render(&gpu, &mut plain, fonts.clone());
    let blended = pixel(&img, 50, 30);
    assert!(blended[0] > 20, "red shows through the blue when not layered: {blended:?}");

    let mut layered = Scene::new(fonts.clone());
    overlapping_pair(&mut layered, true);
    let img = render(&gpu, &mut layered, fonts);
    let overlap = pixel(&img, 50, 30);
    assert_eq!(overlap[0], 0, "red is fully hidden under the blue inside the layer: {overlap:?}");
    assert!((overlap[2] as i32 - 128).abs() <= 2 && (overlap[3] as i32 - 128).abs() <= 2, "group alpha: {overlap:?}");
    let red = pixel(&img, 30, 30);
    assert!((red[0] as i32 - 128).abs() <= 2 && red[2] == 0, "red only part: {red:?}");
    assert_eq!(pixel(&img, 10, 30)[3], 0, "outside the layer bounds");
    assert_eq!(pixel(&img, 90, 30)[3], 0, "outside the layer bounds on the right");
}

#[test]
fn layer_respects_clip_and_nesting() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let clipper = scene.add(root, Node::item().x(8.0).y(8.0).size(48.0, 32.0).clip(true));
    let outer = scene.add(clipper, Node::item().x(-4.0).y(-4.0).size(80.0, 50.0).opacity(0.5).layer(true));
    let inner = scene.add(outer, Node::item().x(10.0).y(10.0).size(30.0, 30.0).opacity(0.5).layer(true));
    scene.add(inner, Node::rect().size(30.0, 30.0).color(Color::hex(0x00ff00)));
    let img = render(&gpu, &mut scene, fonts);
    let inside = pixel(&img, 24, 24);
    assert!((inside[1] as i32 - 64).abs() <= 3, "two nested half opacities give a quarter: {inside:?}");
    assert_eq!(pixel(&img, 60, 24)[3], 0, "clipped on the right");
    assert_eq!(pixel(&img, 24, 44)[3], 0, "clipped at the bottom");
}

#[test]
fn layer_blur_softens_edges_and_keeps_the_core() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let group = scene.add(root, Node::item().x(40.0).y(14.0).size(36.0, 36.0).layer_effect(LayerEffect::blur(0.25)));
    scene.add(group, Node::rect().size(36.0, 36.0).color(Color::WHITE));
    let img = render(&gpu, &mut scene, fonts);
    let core = pixel(&img, 58, 32);
    assert!(core[3] >= 250, "core stays opaque: {core:?}");
    let halo = pixel(&img, 38, 32);
    assert!(halo[3] > 10 && halo[3] < 200, "blur spreads outside the original edge: {halo:?}");
    let edge_inside = pixel(&img, 41, 32);
    assert!(edge_inside[3] < 250 && edge_inside[3] > 60, "edge is soft inside too: {edge_inside:?}");
    assert_eq!(pixel(&img, 10, 32)[3], 0, "far away stays empty");
}

#[test]
fn layer_shadow_is_offset_tinted_and_under_the_content() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let effect = LayerEffect::shadow(Color::hex(0x0000ff), 1.0, 0.1, Vec2::new(14.0, 10.0));
    let group = scene.add(root, Node::item().x(20.0).y(8.0).size(24.0, 24.0).layer_effect(effect));
    scene.add(group, Node::rect().size(24.0, 24.0).color(Color::hex(0xff0000)));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 30, 20), [255, 0, 0, 255], "content stays on top");
    let shadow = pixel(&img, 52, 38);
    assert!(shadow[2] > 200 && shadow[0] < 30 && shadow[3] > 200, "shadow tinted blue below right: {shadow:?}");
    assert_eq!(pixel(&img, 8, 8)[3], 0, "no shadow to the top left");
    assert_eq!(pixel(&img, 100, 50)[3], 0, "no shadow far away");
}

#[test]
fn layer_mask_cuts_the_content() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let mask =
        scene.add(root, Node::rect().x(20.0).y(10.0).size(40.0, 40.0).radius(20.0).color(Color::WHITE).mask_source());
    let group = scene.add(root, Node::item().x(20.0).y(10.0).size(40.0, 40.0).layer_effect(LayerEffect::mask(mask)));
    scene.add(group, Node::rect().size(40.0, 40.0).color(Color::hex(0x00ff00)));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 40, 30), [0, 255, 0, 255], "inside the mask");
    assert_eq!(pixel(&img, 21, 11)[3], 0, "corner outside the round mask");
    assert_eq!(pixel(&img, 90, 30)[3], 0, "the invisible mask source draws nothing itself");
}

#[test]
fn layer_colour_adjustments() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let grey =
        scene.add(root, Node::item().x(2.0).y(2.0).size(20.0, 20.0).layer_effect(LayerEffect::adjust(0.0, 0.0, -1.0)));
    scene.add(grey, Node::rect().size(20.0, 20.0).color(Color::hex(0xff0000)));
    let tinted = scene.add(
        root,
        Node::item().x(30.0).y(2.0).size(20.0, 20.0).layer_effect(LayerEffect::colorize(Color::hex(0x0000ff), 1.0)),
    );
    scene.add(tinted, Node::rect().size(20.0, 20.0).color(Color::WHITE));
    let bright =
        scene.add(root, Node::item().x(60.0).y(2.0).size(20.0, 20.0).layer_effect(LayerEffect::adjust(0.5, 0.0, 0.0)));
    scene.add(bright, Node::rect().size(20.0, 20.0).color(Color::hex(0x000000)));
    let img = render(&gpu, &mut scene, fonts);
    let g = pixel(&img, 12, 12);
    assert!(
        (g[0] as i32 - g[1] as i32).abs() <= 2 && (g[1] as i32 - g[2] as i32).abs() <= 2 && g[0] > 40,
        "grey {g:?}"
    );
    let t = pixel(&img, 40, 12);
    assert!(t[2] > 240 && t[0] < 15 && t[1] < 15, "blue tint {t:?}");
    let b = pixel(&img, 70, 12);
    assert!((b[0] as i32 - 128).abs() <= 2 && b[3] == 255, "black lifted to mid grey: {b:?}");
}

#[test]
fn unchanged_layers_are_reused_between_frames() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let group = scene.add(root, Node::item().x(10.0).y(10.0).size(30.0, 30.0).layer_effect(LayerEffect::blur(0.25)));
    scene.add(group, Node::rect().size(30.0, 30.0).color(Color::WHITE));
    let (once, reused_first) = render_frames(&gpu, &mut scene, fonts.clone(), 1);
    assert_eq!(reused_first, 0);
    let (twice, reused) = render_frames(&gpu, &mut scene, fonts, 2);
    assert_eq!(reused, 1, "the second frame reuses the layer texture");
    assert_eq!(once, twice, "reused output is identical");
}

#[test]
fn grid_pattern_shader_draws_lines() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let params = ShaderParams::grid_pattern(16.0, 2.0, Color::WHITE);
    scene.add(root, Node::shader(ShaderEffect::GridPattern, params).size(64.0, 64.0));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 15, 8), [255, 255, 255, 255], "on a vertical line");
    assert_eq!(pixel(&img, 8, 8)[3], 0, "between lines");
    assert_eq!(pixel(&img, 90, 8)[3], 0, "outside the item");
}

#[test]
fn vignette_shader_darkens_the_corners() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let params = ShaderParams::vignette(Color::hex(0x000000), 0.0, 0.0, 0.0);
    scene.add(root, Node::shader(ShaderEffect::Vignette, params).size(128.0, 64.0));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 64, 32)[3], 0, "centre is clear");
    let corner = pixel(&img, 1, 1);
    assert!((40..80).contains(&corner[3]), "corner alpha {corner:?}");
}

#[test]
fn corner_cutout_shader_leaves_a_quarter_circle_clear() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let params = ShaderParams::corner_cutout(0.0, Color::WHITE);
    scene.add(root, Node::shader(ShaderEffect::CornerCutout, params).x(10.0).y(10.0).size(20.0, 20.0));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 11, 11), [255, 255, 255, 255], "the fillet fills outside the circle");
    assert_eq!(pixel(&img, 25, 25)[3], 0, "the circle itself is cut out");
}

#[test]
fn dashed_ellipse_shader_draws_a_ring() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let params = ShaderParams::dashed_ellipse(Vec2::new(20.0, 20.0), Vec2::new(10.0, 10.0), 2.0, Color::WHITE);
    scene.add(root, Node::shader(ShaderEffect::DashedEllipse, params).size(64.0, 64.0));
    let img = render(&gpu, &mut scene, fonts);
    assert!(pixel(&img, 52, 32)[3] >= 250, "first dash starts at angle zero");
    assert_eq!(pixel(&img, 32, 32)[3], 0, "centre is clear");
}

#[test]
fn rounded_mask_shader_samples_its_source() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let source = scene.add(root, Node::rect().x(0.0).y(0.0).size(40.0, 40.0).color(Color::hex(0x00ff00)).mask_source());
    let params = ShaderParams::rounded_mask([20.0; 4], 0.0);
    scene.add(
        root,
        Node::shader(ShaderEffect::RoundedMask, params)
            .x(30.0)
            .y(10.0)
            .size(40.0, 40.0)
            .shader_sources([Some(source), None]),
    );
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 50, 30), [0, 255, 0, 255], "centre shows the source");
    assert_eq!(pixel(&img, 31, 11)[3], 0, "corner is rounded away");
    assert_eq!(pixel(&img, 10, 30)[3], 0, "the source itself is not drawn");
}

#[test]
fn marquee_scroll_shader_offsets_the_source() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let source = scene.add(root, Node::item().size(40.0, 20.0).mask_source());
    scene.add(source, Node::rect().size(20.0, 20.0).color(Color::hex(0xff0000)));
    scene.add(source, Node::rect().x(20.0).size(20.0, 20.0).color(Color::hex(0x0000ff)));
    let params = ShaderParams::marquee_scroll(Vec2::new(40.0, 20.0), 20.0);
    scene.add(
        root,
        Node::shader(ShaderEffect::MarqueeScroll, params)
            .x(50.0)
            .y(0.0)
            .size(20.0, 20.0)
            .shader_sources([Some(source), None]),
    );
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 55, 10), [0, 0, 255, 255], "scrolled to the second half");
}

#[test]
fn wallpaper_blend_shader_mixes_two_sources() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let old = scene.add(root, Node::rect().size(40.0, 40.0).color(Color::hex(0xff0000)).mask_source());
    let new = scene.add(root, Node::rect().size(40.0, 40.0).color(Color::hex(0x0000ff)).mask_source());
    let params = ShaderParams::wallpaper_blend(0.5, 0.0, Vec2::new(0.5, 0.5), [0.0; 4]);
    scene.add(
        root,
        Node::shader(ShaderEffect::WallpaperBlend, params)
            .x(50.0)
            .size(40.0, 40.0)
            .shader_sources([Some(old), Some(new)]),
    );
    let img = render(&gpu, &mut scene, fonts);
    let p = pixel(&img, 70, 20);
    assert!((p[0] as i32 - 127).abs() <= 2 && (p[2] as i32 - 128).abs() <= 2 && p[1] == 0, "{p:?}");
}

#[test]
fn ghost_shader_draws_body_eyes_and_scalloped_feet() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let (body, pupil) = (Color::hex(0xff0000), Color::hex(0x0000ff));
    scene.add(
        root,
        Node::shader(ShaderEffect::Ghost, ShaderParams::ghost(body, pupil, Vec2::new(0.0, 0.0))).size(60.0, 66.0),
    );
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 30, 45), [255, 0, 0, 255], "body");
    assert_eq!(pixel(&img, 1, 1)[3], 0, "outside the round head");
    let eye = pixel(&img, 19, 28);
    assert!(eye[2] > 200 && eye[0] < 60, "pupil {eye:?}");
    let white = pixel(&img, 12, 28);
    assert!(white[0] > 240 && white[1] > 240 && white[2] > 240, "eye white {white:?}");
    assert_eq!(pixel(&img, 20, 63)[3], 255, "foot");
    assert_eq!(pixel(&img, 30, 63)[3], 0, "scallop between the feet");
}

#[test]
fn widget_outline_shader_traces_the_rounded_rect() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let params = ShaderParams::widget_outline(Color::WHITE, 8.0, 2.0, 2.0, 0.0, 0.1, 0.0);
    scene.add(root, Node::shader(ShaderEffect::WidgetOutline, params).size(64.0, 64.0));
    let img = render(&gpu, &mut scene, fonts);
    assert!(pixel(&img, 32, 2)[3] >= 250, "top edge");
    assert!(pixel(&img, 61, 32)[3] >= 250, "right edge");
    assert_eq!(pixel(&img, 32, 32)[3], 0, "interior");
}

#[test]
fn border_charge_shader_draws_the_stroke_up_to_the_progress() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let half = 2.0 * (64.0 - 6.0 - 16.0) + 2.0 * (64.0 - 6.0 - 16.0) + 2.0 * std::f32::consts::PI * 8.0;
    let params = ShaderParams::border_charge(8.0, 3.5, 6.0, half * 0.5);
    scene.add(root, Node::shader(ShaderEffect::BorderCharge, params).size(64.0, 64.0));
    let img = render(&gpu, &mut scene, fonts);
    assert!(pixel(&img, 3, 32)[3] >= 250, "left edge is drawn first");
    assert!(pixel(&img, 32, 3)[3] >= 250, "top edge is drawn");
    assert_eq!(pixel(&img, 32, 60)[3], 0, "bottom edge is not reached yet");
    assert_eq!(pixel(&img, 32, 32)[3], 0, "interior");
}

#[test]
fn seekbar_wave_shader_draws_track_and_progress() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let params = ShaderParams::seekbar_wave(
        Color::WHITE,
        [2.0, 20.0, 50.0, 4.0],
        [0.0, 0.5, 20.0, 0.0],
        [[1.0, 1.0, 10.0, 30.0], [1.0, 0.5, 10.0, 30.0]],
        [[1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
    );
    scene.add(root, Node::shader(ShaderEffect::SeekbarWave, params).size(100.0, 40.0));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 25, 20)[3], 255, "progress line");
    let track = pixel(&img, 75, 20)[3] as i32;
    assert!((track - 128).abs() <= 2, "track alpha {track}");
    assert_eq!(pixel(&img, 25, 5)[3], 0, "no wave at zero amplitude");
}

#[test]
fn wipe_shaders_cover_the_item_when_fully_revealed() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let colors =
        [Color::hex(0x101010), Color::hex(0x202020), Color::hex(0x303030), Color::hex(0x404040), Color::hex(0xff0000)];
    scene.add(
        root,
        Node::shader(ShaderEffect::CurtainWipe, ShaderParams::curtain_wipe(1.0, 28.0, colors)).size(40.0, 40.0),
    );
    scene.add(
        root,
        Node::shader(ShaderEffect::ScreenWipe, ShaderParams::screen_wipe(1.0, 0.0, 45.0, colors))
            .x(50.0)
            .size(40.0, 40.0),
    );
    scene.add(
        root,
        Node::shader(ShaderEffect::CurtainWipe, ShaderParams::curtain_wipe(0.0, 28.0, colors))
            .x(100.0)
            .size(40.0, 40.0),
    );
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 20, 20), [255, 0, 0, 255]);
    assert_eq!(pixel(&img, 70, 20), [255, 0, 0, 255]);
    assert_eq!(pixel(&img, 120, 20)[3], 0, "nothing before the reveal starts");
}

#[test]
fn fluid_wave_shader_fills_horizontally() {
    let Some(gpu) = gpu() else { return };
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    let params = ShaderParams::fluid_wave(0.0, 0.5, [0.0, 0.0], false, 1.0, Color::WHITE, Color::WHITE);
    scene.add(root, Node::shader(ShaderEffect::FluidWave, params).size(64.0, 64.0));
    let img = render(&gpu, &mut scene, fonts);
    assert_eq!(pixel(&img, 16, 32)[3], 255, "filled side");
    assert_eq!(pixel(&img, 48, 32)[3], 0, "empty side");
}

fn render_one(effect: ShaderEffect, params: ShaderParams) -> Option<Vec<u8>> {
    let gpu = gpu()?;
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(root, Node::shader(effect, params).size(64.0, 64.0));
    Some(render(&gpu, &mut scene, fonts))
}

#[test]
fn circular_wave_gauge_shaders_draw_track_and_progress() {
    let plain = ShaderParams::circular_wave_gauge(0.5, 4.0, false, Color::WHITE, [0.0, 28.0, 0.0, 0.0]);
    let smooth = ShaderParams::circular_wave_gauge_smooth(
        0.5,
        4.0,
        false,
        Color::WHITE,
        [0.0, 28.0, 0.0, 0.0],
        Color::hex(0x404040),
    );
    for (effect, params) in [(ShaderEffect::CircularWaveGauge, plain), (ShaderEffect::CircularWaveGaugeSmooth, smooth)]
    {
        let Some(img) = render_one(effect, params) else { return };
        assert_eq!(pixel(&img, 60, 32)[3], 255, "{effect:?} progress on the right half");
        assert!(pixel(&img, 3, 32)[3] > 20, "{effect:?} track on the empty half");
        assert_eq!(pixel(&img, 32, 32)[3], 0, "{effect:?} hollow centre");
    }
}

fn render_one_data(effect: ShaderEffect, params: ShaderParams, data: ShaderData) -> Option<Vec<u8>> {
    let gpu = gpu()?;
    let fonts = Fonts::new();
    let mut scene = Scene::new(fonts.clone());
    let root = scene.root();
    scene.add(root, Node::shader(effect, params).shader_data(data).size(64.0, 64.0));
    Some(render(&gpu, &mut scene, fonts))
}

#[test]
fn visualizer_bars_grow_from_the_base_by_level() {
    let params = ShaderParams::visualizer_bars(
        Color::WHITE,
        4,
        false,
        2.0,
        0.0,
        [0.0, 60.0],
        0.0,
        [1.0, 0.0],
        false,
        [0.0, 0.0],
        0.0,
    );
    let Some(img) =
        render_one_data(ShaderEffect::VisualizerBars, params, ShaderData::from_floats(&[1.0, 0.5, 0.0, 1.0]))
    else {
        return;
    };
    assert_eq!(pixel(&img, 8, 40)[3], 255, "full bar");
    assert_eq!(pixel(&img, 8, 10)[3], 255, "full bar reaches the top");
    assert_eq!(pixel(&img, 24, 50)[3], 255, "half bar near the base");
    assert_eq!(pixel(&img, 24, 10)[3], 0, "half bar stops short");
    assert_eq!(pixel(&img, 40, 50)[3], 0, "silent bar is empty");
    assert_eq!(pixel(&img, 56, 40)[3], 255, "fourth bar");
}

#[test]
fn visualizer_wave_fills_under_the_curve() {
    let params = ShaderParams::visualizer_wave(Color::WHITE, 4, false, 0.0, 32.0);
    let Some(img) = render_one_data(ShaderEffect::VisualizerWave, params, ShaderData::from_floats(&[1.0; 4])) else {
        return;
    };
    assert_eq!(pixel(&img, 32, 10)[3], 255, "inside the wave");
    assert_eq!(pixel(&img, 32, 50)[3], 0, "beyond the wave");
}

#[test]
fn visualizer_ring_wave_draws_outside_the_ring() {
    let params = ShaderParams::visualizer_ring_wave(Color::WHITE, 8, [0.0, 20.0], [1.0, 0.0], 10.0, 0.0);
    let Some(img) = render_one_data(ShaderEffect::VisualizerRingWave, params, ShaderData::from_floats(&[1.0; 8]))
    else {
        return;
    };
    assert_eq!(pixel(&img, 32, 17)[3], 255, "between the ring and the tips");
    assert_eq!(pixel(&img, 32, 32)[3], 0, "hollow centre");
    assert_eq!(pixel(&img, 1, 1)[3], 0, "corner is empty");
}
