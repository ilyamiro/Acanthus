use std::f32::consts::{FRAC_PI_2, TAU};

use acanthus_core::{Color, Rect, Vec2};
use resvg::tiny_skia;

use crate::decode::Decoded;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Segment {
    Move(Vec2),
    Line(Vec2),
    Quad(Vec2, Vec2),
    Cubic(Vec2, Vec2, Vec2),
    Close,
}

/// An outline of lines and curves in logical units, built like a Qt `Context2D` path.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Path(Vec<Segment>);

impl Path {
    /// An empty path.
    pub fn new() -> Self {
        Path::default()
    }

    /// Starts a new subpath at `p`.
    pub fn move_to(mut self, p: Vec2) -> Self {
        self.0.push(Segment::Move(p));
        self
    }

    /// A straight line to `p`.
    pub fn line_to(mut self, p: Vec2) -> Self {
        self.0.push(Segment::Line(p));
        self
    }

    /// A quadratic Bézier curve through control point `c` to `p`.
    pub fn quad_to(mut self, c: Vec2, p: Vec2) -> Self {
        self.0.push(Segment::Quad(c, p));
        self
    }

    /// A cubic Bézier curve through control points `c1` and `c2` to `p`.
    pub fn cubic_to(mut self, c1: Vec2, c2: Vec2, p: Vec2) -> Self {
        self.0.push(Segment::Cubic(c1, c2, p));
        self
    }

    /// Closes the current subpath with a line back to its start.
    pub fn close(mut self) -> Self {
        self.0.push(Segment::Close);
        self
    }

    /// Adds a closed rectangle as its own subpath.
    pub fn rect(self, r: Rect) -> Self {
        self.move_to(Vec2::new(r.x, r.y))
            .line_to(Vec2::new(r.x + r.w, r.y))
            .line_to(Vec2::new(r.x + r.w, r.y + r.h))
            .line_to(Vec2::new(r.x, r.y + r.h))
            .close()
    }

    /// An arc of the circle at `center` from angle `start` to `end` in radians, clockwise on screen unless `anticlockwise`, joined to the current point by a line as in the HTML canvas `arc`.
    pub fn arc(mut self, center: Vec2, radius: f32, start: f32, end: f32, anticlockwise: bool) -> Self {
        let sweep = match anticlockwise {
            false if end - start >= TAU => TAU,
            false => (end - start).rem_euclid(TAU),
            true if start - end >= TAU => -TAU,
            true => -(start - end).rem_euclid(TAU),
        };
        let point = |a: f32| center + Vec2::new(a.cos(), a.sin()) * radius;
        let tangent = |a: f32| Vec2::new(-a.sin(), a.cos()) * radius;
        let first = point(start);
        self.0.push(if self.0.is_empty() { Segment::Move(first) } else { Segment::Line(first) });
        let pieces = (sweep.abs() / FRAC_PI_2).ceil().max(1.0) as usize;
        let step = sweep / pieces as f32;
        let k = 4.0 / 3.0 * (step / 4.0).tan();
        for i in 0..pieces {
            let a0 = start + step * i as f32;
            let a1 = a0 + step;
            self.0.push(Segment::Cubic(point(a0) + tangent(a0) * k, point(a1) - tangent(a1) * k, point(a1)));
        }
        self
    }

    fn build(&self) -> Option<tiny_skia::Path> {
        let mut b = tiny_skia::PathBuilder::new();
        for segment in &self.0 {
            match *segment {
                Segment::Move(p) => b.move_to(p.x, p.y),
                Segment::Line(p) => b.line_to(p.x, p.y),
                Segment::Quad(c, p) => b.quad_to(c.x, c.y, p.x, p.y),
                Segment::Cubic(c1, c2, p) => b.cubic_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y),
                Segment::Close => b.close(),
            }
        }
        b.finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Command {
    Fill(Path, Color),
    Stroke(Path, Color, f32),
}

/// The drawing of a canvas: fills and strokes recorded in logical units over an area of `size`, rasterised at the device scale.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Painting {
    /// The area painted, in logical units.
    pub size: Vec2,
    commands: Vec<Command>,
}

impl Painting {
    /// An empty painting of `size`.
    pub fn new(size: Vec2) -> Self {
        Painting { size, commands: Vec::new() }
    }

    /// Fills `path` with `color` using the non-zero winding rule.
    pub fn fill(&mut self, path: &Path, color: Color) {
        self.commands.push(Command::Fill(path.clone(), color));
    }

    /// Strokes `path` with `color` at `width`, with butt caps and miter joins as Qt's `Context2D` defaults.
    pub fn stroke(&mut self, path: &Path, color: Color, width: f32) {
        self.commands.push(Command::Stroke(path.clone(), color, width));
    }

    /// Renders the painting at `scale` device pixels per unit into premultiplied RGBA, `None` when the area is empty.
    pub fn rasterize(&self, scale: f32) -> Option<Decoded> {
        let width = (self.size.x * scale).ceil() as u32;
        let height = (self.size.y * scale).ceil() as u32;
        let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
        let transform = tiny_skia::Transform::from_scale(scale, scale);
        for command in &self.commands {
            let (path, color) = match command {
                Command::Fill(path, color) | Command::Stroke(path, color, _) => (path, color),
            };
            let (Some(path), Some(color)) =
                (path.build(), tiny_skia::Color::from_rgba(color.r, color.g, color.b, color.a))
            else {
                continue;
            };
            let mut paint = tiny_skia::Paint::default();
            paint.set_color(color);
            match command {
                Command::Fill(..) => pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None),
                Command::Stroke(_, _, width) => {
                    let stroke = tiny_skia::Stroke { width: *width, miter_limit: 10.0, ..Default::default() };
                    pixmap.stroke_path(&path, &paint, &stroke, transform, None);
                }
            }
        }
        Some(Decoded { width, height, rgba: pixmap.take(), animation: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(image: &Decoded, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * image.width + x) * 4) as usize;
        image.rgba[i..i + 4].try_into().unwrap()
    }

    #[test]
    fn fills_are_rasterised_at_the_device_scale() {
        let mut painting = Painting::new(Vec2::new(10.0, 5.0));
        painting.fill(&Path::new().rect(Rect::new(0.0, 0.0, 5.0, 5.0)), Color::rgba(1.0, 0.0, 0.0, 1.0));
        let image = painting.rasterize(2.0).unwrap();
        assert_eq!((image.width, image.height), (20, 10));
        assert_eq!(pixel(&image, 4, 4), [255, 0, 0, 255]);
        assert_eq!(pixel(&image, 15, 4), [0, 0, 0, 0]);
    }

    #[test]
    fn strokes_cover_only_the_outline() {
        let mut painting = Painting::new(Vec2::new(20.0, 20.0));
        let path = Path::new().move_to(Vec2::new(0.0, 10.0)).line_to(Vec2::new(20.0, 10.0));
        painting.stroke(&path, Color::rgba(0.0, 0.0, 1.0, 0.5), 4.0);
        let image = painting.rasterize(1.0).unwrap();
        assert_eq!(pixel(&image, 10, 10), [0, 0, 128, 128]);
        assert_eq!(pixel(&image, 10, 3), [0, 0, 0, 0]);
    }

    #[test]
    fn a_full_arc_is_a_circle_and_angles_follow_the_html_canvas() {
        let mut painting = Painting::new(Vec2::new(20.0, 20.0));
        painting.fill(&Path::new().arc(Vec2::new(10.0, 10.0), 8.0, 0.0, TAU, false), Color::WHITE);
        let image = painting.rasterize(1.0).unwrap();
        assert_eq!(pixel(&image, 10, 10), [255; 4]);
        assert_eq!(pixel(&image, 1, 1), [0; 4]);

        let mut half = Painting::new(Vec2::new(20.0, 20.0));
        half.fill(&Path::new().arc(Vec2::new(10.0, 10.0), 8.0, 0.0, std::f32::consts::PI, false), Color::WHITE);
        let image = half.rasterize(1.0).unwrap();
        assert_eq!(pixel(&image, 10, 14), [255; 4]);
        assert_eq!(pixel(&image, 10, 6), [0; 4]);
    }

    #[test]
    fn an_empty_area_has_no_image() {
        assert!(Painting::new(Vec2::new(0.0, 10.0)).rasterize(1.0).is_none());
    }
}
