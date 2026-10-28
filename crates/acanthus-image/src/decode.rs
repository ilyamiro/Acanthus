use std::fmt;
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

use image::codecs::gif::GifDecoder;
use image::imageops::FilterType;
use image::{AnimationDecoder, DynamicImage};

const MAX_FRAMES: usize = 512;
const MIN_DELAY_MS: u32 = 20;
const DEFAULT_DELAY_MS: u32 = 100;

#[derive(Debug, PartialEq, Eq)]
/// Frames and per-frame delays of an animated image.
pub struct Animation {
    /// RGBA8 pixels of every frame.
    pub frames: Vec<Vec<u8>>,
    /// Display time of every frame in milliseconds.
    pub delays_ms: Vec<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A decoded image as straight RGBA8 pixels.
pub struct Decoded {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Pixels of the first frame.
    pub rgba: Vec<u8>,
    /// Present for animated images (GIF, animated WebP).
    pub animation: Option<Arc<Animation>>,
}

impl Decoded {
    /// Number of frames, 1 for still images.
    pub fn frame_count(&self) -> usize {
        self.animation.as_ref().map_or(1, |a| a.frames.len())
    }

    /// Pixels of frame `frame`, clamped to the last frame.
    pub fn frame_rgba(&self, frame: usize) -> &[u8] {
        match &self.animation {
            Some(a) if !a.frames.is_empty() => &a.frames[frame.min(a.frames.len() - 1)],
            _ => &self.rgba,
        }
    }

    /// Delay of frame `frame` in milliseconds, 100 when unknown.
    pub fn delay_ms(&self, frame: usize) -> u32 {
        self.animation.as_ref().and_then(|a| a.delays_ms.get(frame).copied()).unwrap_or(DEFAULT_DELAY_MS)
    }
}

#[derive(Debug)]
/// Why an image could not be decoded.
pub enum DecodeError {
    /// The file could not be read.
    Io(std::io::Error),
    /// The raster decoder rejected the data.
    Image(image::ImageError),
    /// The SVG could not be parsed or rendered.
    Svg(String),
    /// The image has zero pixels.
    Empty,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Io(e) => write!(f, "{e}"),
            DecodeError::Image(e) => write!(f, "{e}"),
            DecodeError::Svg(e) => write!(f, "{e}"),
            DecodeError::Empty => write!(f, "image has no pixels"),
        }
    }
}

impl std::error::Error for DecodeError {}

pub fn is_svg(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("svg") || e.eq_ignore_ascii_case("svgz"))
}

/// Whether the path has an `svg` or `svgz` extension.
pub fn decode_file(path: &Path, target: Option<(u32, u32)>) -> Result<Decoded, DecodeError> {
    let bytes = std::fs::read(path).map_err(DecodeError::Io)?;
    if is_svg(path) { decode_svg(&bytes, target) } else { decode_raster(&bytes, target) }
}

/// Reads and decodes a file, choosing SVG or raster by extension. `target` is the size SVGs are rasterised at.
pub fn decode_raster(bytes: &[u8], target: Option<(u32, u32)>) -> Result<Decoded, DecodeError> {
    if bytes.starts_with(b"GIF8")
        && let Some(decoded) = decode_gif(bytes, target)?
    {
        return Ok(decoded);
    }
    let img = image::load_from_memory(bytes).map_err(DecodeError::Image)?;
    let (width, height, rgba) = finish(img, target)?;
    Ok(Decoded { width, height, rgba, animation: None })
}

fn finish(mut img: DynamicImage, target: Option<(u32, u32)>) -> Result<(u32, u32, Vec<u8>), DecodeError> {
    if let Some((tw, th)) = target
        && tw > 0
        && th > 0
        && (img.width() > tw || img.height() > th)
    {
        img = img.resize(tw, th, FilterType::Triangle);
    }
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err(DecodeError::Empty);
    }
    let mut data = rgba.into_raw();
    premultiply(&mut data);
    Ok((width, height, data))
}

fn decode_gif(bytes: &[u8], target: Option<(u32, u32)>) -> Result<Option<Decoded>, DecodeError> {
    let decoder = GifDecoder::new(Cursor::new(bytes)).map_err(DecodeError::Image)?;
    let mut frames = Vec::new();
    let mut delays_ms = Vec::new();
    let mut size = (0, 0);
    for frame in decoder.into_frames().take(MAX_FRAMES) {
        let frame = frame.map_err(DecodeError::Image)?;
        let (n, d) = frame.delay().numer_denom_ms();
        let delay = n / d.max(1);
        delays_ms.push(if delay < MIN_DELAY_MS { DEFAULT_DELAY_MS } else { delay });
        let (width, height, rgba) = finish(DynamicImage::ImageRgba8(frame.into_buffer()), target)?;
        size = (width, height);
        frames.push(rgba);
    }
    if frames.len() < 2 {
        return Ok(None);
    }
    let rgba = frames[0].clone();
    Ok(Some(Decoded {
        width: size.0,
        height: size.1,
        rgba,
        animation: Some(Arc::new(Animation { frames, delays_ms })),
    }))
}

/// Rasterises SVG data, scaled to `target` when given.
pub fn decode_svg(bytes: &[u8], target: Option<(u32, u32)>) -> Result<Decoded, DecodeError> {
    let options = resvg::usvg::Options::default();
    let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| DecodeError::Svg(e.to_string()))?;
    let size = tree.size();
    let (sw, sh) = (size.width(), size.height());
    if sw <= 0.0 || sh <= 0.0 {
        return Err(DecodeError::Empty);
    }
    let scale = match target {
        Some((tw, th)) if tw > 0 && th > 0 => (tw as f32 / sw).min(th as f32 / sh),
        _ => 1.0,
    };
    let width = ((sw * scale).round() as u32).max(1);
    let height = ((sh * scale).round() as u32).max(1);
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).ok_or(DecodeError::Empty)?;
    let transform = resvg::tiny_skia::Transform::from_scale(width as f32 / sw, height as f32 / sh);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    Ok(Decoded { width, height, rgba: pixmap.take(), animation: None })
}

fn premultiply(data: &mut [u8]) {
    for px in data.as_chunks_mut::<4>().0 {
        let a = px[3] as u32;
        if a == 255 {
            continue;
        }
        for c in &mut px[..3] {
            *c = ((*c as u32 * a + 127) / 255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#ff0000"/></svg>"##;

    #[test]
    fn svg_renders_at_the_requested_size_keeping_aspect() {
        let d = decode_svg(SVG.as_bytes(), Some((40, 40))).unwrap();
        assert_eq!((d.width, d.height), (40, 20));
        assert_eq!(&d.rgba[0..4], &[255, 0, 0, 255]);
        let natural = decode_svg(SVG.as_bytes(), None).unwrap();
        assert_eq!((natural.width, natural.height), (20, 10));
    }

    #[test]
    fn raster_decodes_premultiplied_and_downscales() {
        let mut img = image::RgbaImage::new(8, 4);
        for px in img.pixels_mut() {
            *px = image::Rgba([200, 100, 50, 128]);
        }
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let full = decode_raster(&png, None).unwrap();
        assert_eq!((full.width, full.height), (8, 4));
        assert_eq!(&full.rgba[0..4], &[100, 50, 25, 128]);
        let small = decode_raster(&png, Some((4, 4))).unwrap();
        assert_eq!((small.width, small.height), (4, 2));
    }

    fn two_frame_gif(delays: [u16; 2]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut out);
            for (i, delay) in delays.iter().enumerate() {
                let color = if i == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] };
                let frame = image::Frame::from_parts(
                    image::RgbaImage::from_pixel(4, 2, image::Rgba(color)),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(*delay as u32, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        out
    }

    #[test]
    fn gif_keeps_every_frame_and_its_delay() {
        let d = decode_raster(&two_frame_gif([40, 250]), None).unwrap();
        assert_eq!((d.width, d.height, d.frame_count()), (4, 2, 2));
        assert_eq!(&d.frame_rgba(0)[0..4], &[255, 0, 0, 255]);
        assert_eq!(&d.frame_rgba(1)[0..4], &[0, 0, 255, 255]);
        assert_eq!(&d.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!((d.delay_ms(0), d.delay_ms(1)), (40, 250));
        assert_eq!(d.frame_rgba(9), d.frame_rgba(1));
    }

    #[test]
    fn gif_frames_scale_with_the_target_and_tiny_delays_become_100_ms() {
        let d = decode_raster(&two_frame_gif([0, 10]), Some((2, 2))).unwrap();
        assert_eq!((d.width, d.height), (2, 1));
        assert_eq!(d.frame_rgba(1).len(), 2 * 4);
        assert_eq!((d.delay_ms(0), d.delay_ms(1)), (100, 100));
    }

    #[test]
    fn still_images_report_one_frame() {
        let d = decode_svg(SVG.as_bytes(), None).unwrap();
        assert_eq!(d.frame_count(), 1);
        assert_eq!(d.frame_rgba(3), &d.rgba[..]);
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(decode_raster(b"nope", None).is_err());
        assert!(decode_svg(b"nope", None).is_err());
    }
}
