use acanthus_core::{Rect, Vec2};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
/// How an image fills its destination rectangle, like QML `Image.fillMode`.
pub enum FillMode {
    #[default]
    /// Stretch to the destination, ignoring the aspect ratio.
    Stretch,
    /// Scale uniformly until the whole image is visible.
    PreserveAspectFit,
    /// Scale uniformly until the destination is covered, cropping the overflow.
    PreserveAspectCrop,
    /// Repeat at the source size.
    Tile,
    /// Keep the source size, centred and cropped to the destination.
    Pad,
}

#[derive(Debug, Clone, Copy, PartialEq)]
/// Where and how an image is drawn.
pub struct Placement {
    /// Rectangle to draw into.
    pub rect: Rect,
    /// Texture coordinates as left, top, right, bottom.
    pub uv: [f32; 4],
    /// Whether the texture sampler repeats.
    pub repeat: bool,
}

/// Computes the [`Placement`] of an image of size `source` in `dest` for the given fill mode.
pub fn place(mode: FillMode, source: Vec2, dest: Rect) -> Placement {
    let full = [0.0, 0.0, 1.0, 1.0];
    if source.x <= 0.0 || source.y <= 0.0 || dest.w <= 0.0 || dest.h <= 0.0 {
        return Placement { rect: dest, uv: full, repeat: false };
    }
    match mode {
        FillMode::Stretch => Placement { rect: dest, uv: full, repeat: false },
        FillMode::PreserveAspectFit => {
            let scale = (dest.w / source.x).min(dest.h / source.y);
            let (w, h) = (source.x * scale, source.y * scale);
            let rect = Rect::new(dest.x + (dest.w - w) / 2.0, dest.y + (dest.h - h) / 2.0, w, h);
            Placement { rect, uv: full, repeat: false }
        }
        FillMode::PreserveAspectCrop => {
            let scale = (dest.w / source.x).max(dest.h / source.y);
            let visible_w = dest.w / (source.x * scale);
            let visible_h = dest.h / (source.y * scale);
            let u0 = (1.0 - visible_w) / 2.0;
            let v0 = (1.0 - visible_h) / 2.0;
            Placement { rect: dest, uv: [u0, v0, u0 + visible_w, v0 + visible_h], repeat: false }
        }
        FillMode::Pad => {
            let (w, h) = (source.x.min(dest.w), source.y.min(dest.h));
            let rect = Rect::new(dest.x + (dest.w - w) / 2.0, dest.y + (dest.h - h) / 2.0, w, h);
            let (u0, v0) = ((source.x - w) / 2.0 / source.x, (source.y - h) / 2.0 / source.y);
            Placement { rect, uv: [u0, v0, u0 + w / source.x, v0 + h / source.y], repeat: false }
        }
        FillMode::Tile => Placement { rect: dest, uv: [0.0, 0.0, dest.w / source.x, dest.h / source.y], repeat: true },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterboxes_and_crop_zooms() {
        let dest = Rect::new(10.0, 10.0, 100.0, 100.0);
        let fit = place(FillMode::PreserveAspectFit, Vec2::new(200.0, 100.0), dest);
        assert_eq!(fit.rect, Rect::new(10.0, 35.0, 100.0, 50.0));
        let crop = place(FillMode::PreserveAspectCrop, Vec2::new(200.0, 100.0), dest);
        assert_eq!(crop.rect, dest);
        assert_eq!(crop.uv, [0.25, 0.0, 0.75, 1.0]);
    }

    #[test]
    fn pad_keeps_the_source_size_centred_and_crops_the_overflow() {
        let small = place(FillMode::Pad, Vec2::new(20.0, 10.0), Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(small.rect, Rect::new(40.0, 20.0, 20.0, 10.0));
        assert_eq!(small.uv, [0.0, 0.0, 1.0, 1.0]);
        let large = place(FillMode::Pad, Vec2::new(200.0, 10.0), Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(large.rect, Rect::new(0.0, 20.0, 100.0, 10.0));
        assert_eq!(large.uv, [0.25, 0.0, 0.75, 1.0]);
    }

    #[test]
    fn tile_repeats_at_natural_size() {
        let p = place(FillMode::Tile, Vec2::new(10.0, 20.0), Rect::new(0.0, 0.0, 30.0, 40.0));
        assert_eq!(p.uv, [0.0, 0.0, 3.0, 2.0]);
        assert!(p.repeat);
    }
}
