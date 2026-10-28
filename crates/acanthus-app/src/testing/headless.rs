use std::path::Path;
use std::time::{Duration, Instant};

use acanthus_core::{Color, Vec2};
use acanthus_render::{Gpu, GpuError, Renderer, wgpu};
use acanthus_scene::Scene;
use serde_json::json;

use crate::testing::script::{Action, Script};

/// Length of one frame of the virtual clock in milliseconds: 60 frames per second.
pub const FRAME_MS: f64 = 1000.0 / 60.0;

/// Why a headless run could not happen.
#[derive(Debug, thiserror::Error)]
pub enum HeadlessError {
    /// No GPU adapter could be opened.
    #[error("no GPU adapter: {0}")]
    NoAdapter(#[from] GpuError),
    /// The picture could not be written.
    #[error("writing the picture failed: {0}")]
    Png(#[from] image::ImageError),
}

/// A picture of the surface in device pixels, four straight-alpha bytes per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// Width in device pixels.
    pub width: u32,
    /// Height in device pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub rgba: Vec<u8>,
}

impl Image {
    /// The pixel at `x`, `y` as `[r, g, b, a]`.
    ///
    /// # Panics
    /// When the point is outside the picture.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.width && y < self.height, "pixel ({x}, {y}) is outside {}x{}", self.width, self.height);
        let at = ((y * self.width + x) * 4) as usize;
        [self.rgba[at], self.rgba[at + 1], self.rgba[at + 2], self.rgba[at + 3]]
    }

    /// Number of pixels that are not fully transparent.
    pub fn painted(&self) -> usize {
        self.rgba.as_chunks::<4>().0.iter().filter(|pixel| pixel[3] > 0).count()
    }

    /// Writes the picture as a PNG file.
    pub fn save_png(&self, path: impl AsRef<Path>) -> Result<(), HeadlessError> {
        image::save_buffer(path, &self.rgba, self.width, self.height, image::ColorType::Rgba8)?;
        Ok(())
    }
}

/// What one frame of a run cost.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLog {
    /// Frame number from 0.
    pub index: u32,
    /// Virtual time of the frame in milliseconds.
    pub at_ms: f64,
    /// Real time spent on the frame (script, animation, layout, drawing), stalls included, in milliseconds.
    pub work_ms: f64,
    /// Whether the display list changed and was drawn again.
    pub drawn: bool,
}

/// The outcome of [`Headless::run`].
#[derive(Debug, Clone)]
pub struct Run {
    /// The surface after the last frame.
    pub image: Image,
    /// One entry per frame.
    pub frames: Vec<FrameLog>,
}

impl Run {
    /// The frame log as JSON: `{"frame_ms": 16.67, "frames": [{"index", "at_ms", "work_ms", "drawn"}, ...]}`.
    pub fn frame_log_json(&self) -> String {
        let frames: Vec<_> = self
            .frames
            .iter()
            .map(|f| json!({ "index": f.index, "at_ms": f.at_ms, "work_ms": f.work_ms, "drawn": f.drawn }))
            .collect();
        json!({ "frame_ms": FRAME_MS, "frames": frames }).to_string()
    }

    /// The largest real time any single frame took, in milliseconds.
    pub fn longest_frame_ms(&self) -> f64 {
        self.frames.iter().map(|f| f.work_ms).fold(0.0, f64::max)
    }

    /// Number of frames that drew.
    pub fn drawn_frames(&self) -> usize {
        self.frames.iter().filter(|f| f.drawn).count()
    }
}

/// Renders scenes offscreen on a fixed size and scale, driven by a virtual clock.
pub struct Headless {
    gpu: Gpu,
    size: Vec2,
    scale: f32,
}

impl Headless {
    /// A harness for surfaces of `width` x `height` logical pixels at output `scale`.
    pub fn new(width: u32, height: u32, scale: f32) -> Result<Headless, HeadlessError> {
        let gpu = pollster::block_on(Gpu::new(Gpu::instance(), None))?;
        Ok(Headless { gpu, size: Vec2::new(width as f32, height as f32), scale })
    }

    /// Size of the pictures in device pixels.
    pub fn device_size(&self) -> (u32, u32) {
        ((self.size.x * self.scale).round() as u32, (self.size.y * self.scale).round() as u32)
    }

    /// Draws `scene` once and returns the picture.
    pub fn shot(&self, scene: &mut Scene) -> Image {
        self.run(scene, &Script::new(), 0).image
    }

    /// Replays `script` on `scene` for `duration_ms` of virtual time (at least until the last step) at 60 frames per second.
    ///
    /// Each frame applies the steps that are due, advances the animations by one frame, lays the scene out
    /// and draws it when the display list changed. Pointer and key events carry virtual timestamps, so the
    /// outcome does not depend on how fast the machine is; a `stall` step really sleeps and shows up in
    /// [`FrameLog::work_ms`].
    pub fn run(&self, scene: &mut Scene, script: &Script, duration_ms: u32) -> Run {
        let (width, height) = self.device_size();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let texture = self.gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("acanthus headless target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let context = self.gpu.context(format, scene.images());
        let mut renderer = Renderer::with_context(&self.gpu.device, context, scene.fonts().clone());
        scene.set_viewport(self.size, self.scale);

        let total_ms = duration_ms.max(script.end_ms()) as f64;
        let count = (total_ms / FRAME_MS).ceil() as u32 + 1;
        let steps = script.steps();
        let mut next_step = 0;
        let mut frames = Vec::with_capacity(count as usize);
        for index in 0..count {
            let started = Instant::now();
            let at_ms = index as f64 * FRAME_MS;
            while next_step < steps.len() && steps[next_step].at_ms as f64 <= at_ms {
                apply(scene, &steps[next_step].action, steps[next_step].at_ms as f32 / 1000.0);
                next_step += 1;
            }
            if index > 0 {
                acanthus_anim::tick((FRAME_MS / 1000.0) as f32);
            }
            let frame = scene.frame();
            let drawn = frame.changed || index == 0;
            if drawn {
                renderer.render(
                    &self.gpu.device,
                    &self.gpu.queue,
                    &view,
                    (width, height),
                    frame.primitives,
                    Color::TRANSPARENT,
                );
            }
            frames.push(FrameLog { index, at_ms, work_ms: started.elapsed().as_secs_f64() * 1000.0, drawn });
        }
        Run { image: self.read(&texture, width, height), frames }
    }

    fn read(&self, texture: &wgpu::Texture, width: u32, height: u32) -> Image {
        let row = (width * 4).next_multiple_of(256);
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("acanthus headless readback"),
            size: (row * height) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: Default::default(),
                aspect: Default::default(),
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.gpu.queue.submit(Some(encoder.finish()));
        buffer.map_async(wgpu::MapMode::Read, .., |result| result.unwrap());
        self.gpu.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.get_mapped_range(..).unwrap();
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            let start = (y * row) as usize;
            rgba.extend_from_slice(&data[start..start + (width * 4) as usize]);
        }
        Image { width, height, rgba }
    }
}

fn apply(scene: &mut Scene, action: &Action, time: f32) {
    match action {
        Action::Move(position) => scene.pointer_motion_at(*position, time),
        Action::Down(button) => scene.pointer_button_at(*button, true, time),
        Action::Up(button) => scene.pointer_button_at(*button, false, time),
        Action::Click(button) => {
            scene.pointer_button_at(*button, true, time);
            scene.pointer_button_at(*button, false, time);
        }
        Action::Leave => scene.pointer_leave(),
        Action::Scroll(delta) => scene.scroll(*delta),
        Action::Key(event) => {
            scene.key_event(event);
        }
        Action::Stall(ms) => std::thread::sleep(Duration::from_millis(u64::from(*ms))),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use acanthus_anim::{Animated, Easing, Motion};
    use acanthus_core::Signal;
    use acanthus_scene::{Fonts, Key, MouseArea, Node};

    use super::*;

    fn harness() -> Option<Headless> {
        match Headless::new(64, 32, 1.0) {
            Ok(headless) => Some(headless),
            Err(e) => {
                eprintln!("skipping headless test: {e}");
                None
            }
        }
    }

    #[test]
    fn a_shot_shows_the_scene_at_the_requested_scale() {
        let Some(headless) = Headless::new(40, 20, 2.0).ok() else { return };
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        scene.add(root, Node::rect().size(10.0, 10.0).color(Color::hex(0xff0000)));
        let image = headless.shot(&mut scene);
        assert_eq!((image.width, image.height), (80, 40));
        assert_eq!(image.pixel(10, 10), [255, 0, 0, 255]);
        assert_eq!(image.pixel(30, 10)[3], 0);
        assert!(image.painted() >= 400);
    }

    #[test]
    fn png_files_round_trip() {
        let Some(headless) = harness() else { return };
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        scene.add(root, Node::rect().size(64.0, 32.0).color(Color::hex(0x336699)));
        let image = headless.shot(&mut scene);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        image.save_png(&path).unwrap();
        let back = image::open(&path).unwrap().to_rgba8();
        assert_eq!(back.as_raw(), &image.rgba);
    }

    #[test]
    fn scripted_pointer_and_keys_reach_the_nodes_on_the_virtual_clock() {
        let Some(headless) = harness() else { return };
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let clicks = Rc::new(Cell::new(0));
        let counter = clicks.clone();
        let area = MouseArea::default().on_click(move |_| counter.set(counter.get() + 1));
        let escapes = Rc::new(Cell::new(0));
        let seen = escapes.clone();
        let button = scene.add(
            root,
            Node::rect().size(32.0, 32.0).color(Color::WHITE).mouse(area).focusable(true).on_key(move |e| {
                if e.key == Key::Escape {
                    seen.set(seen.get() + 1);
                }
                true
            }),
        );
        let script = Script::parse("10,10@100 click@200 key:Escape@300 50,10@400").unwrap();
        let run = headless.run(&mut scene, &script, 0);
        assert_eq!(clicks.get(), 1);
        assert_eq!(escapes.get(), 1);
        assert!(scene.get(button).is_some());
        assert_eq!(run.frames.len(), (400.0 / FRAME_MS).ceil() as usize + 1);
    }

    #[test]
    fn animations_advance_one_frame_at_a_time_and_finish_deterministically() {
        let Some(headless) = harness() else { return };
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let width = Signal::new(8.0f32);
        let animated = Animated::new(&width, Motion::tween(200, Easing::Linear));
        scene.add(root, Node::rect().width(animated.signal()).height(8.0).color(Color::WHITE));
        width.set(48.0);
        let run = headless.run(&mut scene, &Script::new(), 400);
        assert_eq!(run.image.pixel(47, 4), [255, 255, 255, 255]);
        assert_eq!(run.image.pixel(50, 4)[3], 0);
        let midway = run.frames.iter().position(|f| f.at_ms >= 100.0).unwrap();
        assert!(run.frames[midway].drawn);
        assert!(run.drawn_frames() > 5 && run.drawn_frames() < run.frames.len());
    }

    #[test]
    fn a_stall_step_shows_in_the_frame_log() {
        let Some(headless) = harness() else { return };
        let mut scene = Scene::new(Fonts::new());
        let script = Script::parse("stall:120@50").unwrap();
        let run = headless.run(&mut scene, &script, 100);
        assert!(run.longest_frame_ms() >= 120.0);
        let log: serde_json::Value = serde_json::from_str(&run.frame_log_json()).unwrap();
        assert_eq!(log["frames"].as_array().unwrap().len(), run.frames.len());
        assert!(log["frames"][0]["drawn"].as_bool().unwrap());
    }
}
