use acanthus_core::{Affine2, Color, Rect, Vec2};
use cosmic_text::CacheKey;
use slotmap::{SecondaryMap, SlotMap};

use crate::layout::ImageEntry;
use crate::node::{Kind, LayerEffect, Node, NodeId, Orientation, paint_order};
use crate::shader::{ShaderData, ShaderEffect, ShaderParams};
use crate::text::ShapedText;
use acanthus_image::{ImageId, place};

/// A rounded-rectangle clip applied on top of a rectangular one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeClip {
    /// The clipping rectangle in the node's own space.
    pub rect: Rect,
    /// Corner radius.
    pub radius: f32,
    /// Maps device-space points back into the rectangle's space.
    pub inverse: Affine2,
}

/// The clip of a primitive: a device-space rectangle and an optional rounded shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    /// The clipping rectangle.
    pub rect: Rect,
    /// A rounded shape that further restricts the visible area.
    pub shape: Option<ShapeClip>,
}

impl Clip {
    /// A clip of a plain rectangle.
    pub fn rect(rect: Rect) -> Self {
        Clip { rect, shape: None }
    }

    /// True when nothing can be visible.
    pub fn is_empty(&self) -> bool {
        self.rect.is_empty()
    }
}

/// A filled rectangle, optionally gradient, rounded and bordered.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// The rectangle in node space.
    pub rect: Rect,
    /// Fill colour, or the gradient start.
    pub start: Color,
    /// The gradient end colour.
    pub end: Color,
    /// Gradient direction, or `None` for a solid fill.
    pub orientation: Option<Orientation>,
    /// Uniform corner radius.
    pub radius: f32,
    /// Per-corner radii (top left, top right, bottom right, bottom left); used when `radius` is zero.
    pub radii: [f32; 4],
    /// Border width, zero for none.
    pub border_width: f32,
    /// Border colour.
    pub border_color: Color,
    /// Clip to apply.
    pub clip: Clip,
    /// Transform from node space to device space.
    pub transform: Affine2,
}

/// A positioned glyph from the glyph atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// Glyph identity in the font cache.
    pub key: CacheKey,
    /// Horizontal pixel position of the glyph origin.
    pub x: i32,
    /// Vertical pixel position of the glyph origin.
    pub y: i32,
    /// Text colour.
    pub color: Color,
    /// Clip to apply.
    pub clip: Clip,
    /// Transform from node space to device space.
    pub transform: Affine2,
}

/// A textured rectangle showing a frame of a stored image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDraw {
    /// The image.
    pub id: ImageId,
    /// Frame index for animated images.
    pub frame: u32,
    /// Destination rectangle in node space.
    pub rect: Rect,
    /// Source rectangle in texture coordinates as left, top, right, bottom.
    pub uv: [f32; 4],
    /// True when the texture tiles over the rectangle.
    pub repeat: bool,
    /// True when the texture is sampled from a mipmap chain.
    pub mipmap: bool,
    /// Opacity from 0 to 1.
    pub opacity: f32,
    /// Clip to apply.
    pub clip: Clip,
    /// Transform from node space to device space.
    pub transform: Affine2,
}

/// A rectangle painted by a shader effect.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderDraw {
    /// The effect.
    pub effect: ShaderEffect,
    /// Its parameter slots.
    pub params: ShaderParams,
    /// Its array data.
    pub data: ShaderData,
    /// Destination rectangle in node space.
    pub rect: Rect,
    /// Size of the node the effect belongs to, which can differ from `rect`.
    pub item_size: Vec2,
    /// Which of the two source layers the effect samples.
    pub sources: [bool; 2],
    /// Opacity from 0 to 1.
    pub opacity: f32,
    /// Clip to apply.
    pub clip: Clip,
    /// Transform from node space to device space.
    pub transform: Affine2,
}

/// A drop shadow applied to a layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerShadowFx {
    /// Shadow colour.
    pub color: Color,
    /// Blur radius.
    pub blur: f32,
    /// Offset from the layer.
    pub offset: Vec2,
}

/// Post-processing applied to a rendered layer: blur, shadow, mask and colour adjustments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerFx {
    /// Gaussian blur radius, zero for none.
    pub blur: f32,
    /// Drop shadow.
    pub shadow: Option<LayerShadowFx>,
    /// True when the layer is thresholded into a mask.
    pub masked: bool,
    /// Alpha threshold of the mask.
    pub mask_threshold: f32,
    /// Softness of the mask edge.
    pub mask_spread: f32,
    /// Brightness adjustment, zero for none.
    pub brightness: f32,
    /// Contrast adjustment, zero for none.
    pub contrast: f32,
    /// Saturation adjustment, zero for none.
    pub saturation: f32,
    /// Strength of the colour overlay, zero for none.
    pub colorization: f32,
    /// Colour of the overlay.
    pub colorization_color: Color,
}

impl Default for LayerFx {
    fn default() -> Self {
        LayerFx {
            blur: 0.0,
            shadow: None,
            masked: false,
            mask_threshold: 0.0,
            mask_spread: 0.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            colorization: 0.0,
            colorization_color: Color::TRANSPARENT,
        }
    }
}

impl LayerFx {
    /// True when any colour adjustment is active.
    pub fn adjusts(&self) -> bool {
        self.brightness != 0.0 || self.contrast != 0.0 || self.saturation != 0.0 || self.colorization > 0.0
    }

    /// True when the layer needs no post-processing.
    pub fn is_none(&self) -> bool {
        self.blur <= 0.0 && self.shadow.is_none() && !self.masked && !self.adjusts()
    }

    fn pad(&self) -> f32 {
        let shadow = self.shadow.map_or(0.0, |s| s.blur + s.offset.x.abs().max(s.offset.y.abs()));
        (self.blur.max(shadow)).ceil() + 1.0
    }
}

/// Start of an offscreen layer; everything up to the matching `LayerEnd` is rendered into it and composited.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerBegin {
    /// Bounds of the layer in device space.
    pub bounds: Rect,
    /// Opacity used when compositing.
    pub opacity: f32,
    /// Clip used when compositing.
    pub clip: Clip,
    /// Post-processing.
    pub fx: LayerFx,
    /// True when the layer is used as a mask for a source layer.
    pub is_mask: bool,
    /// Index of the source slot the layer fills, or `None` for a plain composite.
    pub source: Option<u8>,
}

/// One draw command of a display list, in paint order.
#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    /// A rectangle.
    Quad(Quad),
    /// A glyph.
    Glyph(Glyph),
    /// An image.
    Image(ImageDraw),
    /// A shader effect.
    Shader(ShaderDraw),
    /// Starts an offscreen layer.
    LayerBegin(LayerBegin),
    /// Ends the innermost layer.
    LayerEnd,
}

pub(crate) struct DisplayCx<'a> {
    pub nodes: &'a SlotMap<NodeId, Node>,
    pub geometry: &'a SecondaryMap<NodeId, Rect>,
    pub texts: &'a SecondaryMap<NodeId, ShapedText>,
    pub images: &'a SecondaryMap<NodeId, ImageEntry>,
    pub scale: f32,
    pub out: &'a mut Vec<Primitive>,
    pub extent: Option<Rect>,
    pub forced: Option<NodeId>,
    pub smooth: bool,
    pub snap: bool,
}

impl DisplayCx<'_> {
    pub fn build(&mut self, root: NodeId, viewport: Rect) {
        self.out.clear();
        self.extent = None;
        self.walk(root, 1.0, Clip::rect(viewport.scale(self.scale)), Affine2::IDENTITY);
    }

    fn walk(&mut self, id: NodeId, parent_opacity: f32, clip: Clip, parent: Affine2) {
        let node = &self.nodes[id];
        if !node.layer.get() {
            return self.draw(id, parent_opacity, true, clip, parent);
        }
        if !node.visible.get() {
            return;
        }
        let opacity = parent_opacity * node.opacity.get().clamp(0.0, 1.0);
        if opacity <= 0.0 || clip.is_empty() {
            return;
        }
        let begin = self.out.len();
        self.out.push(Primitive::LayerEnd);
        let outer = self.extent.take();
        self.draw(id, 1.0, false, clip, parent);
        let content = self.extent.take();
        let effect = node.layer_effect.get();
        let mask = effect.mask.filter(|m| self.nodes.contains_key(*m) && *m != id);
        let fx = self.layer_fx(effect, mask.is_some());
        let pad = if fx.is_none() { 0.0 } else { fx.pad() };
        let bounds = content
            .map(|c| pixel_aligned(c.intersect(&clip.rect).inset(-pad).intersect(&clip.rect)))
            .filter(|b| !b.is_empty());
        match bounds {
            Some(bounds) => {
                let composite = if fx.is_none() { Clip::rect(bounds) } else { clip };
                self.out[begin] = Primitive::LayerBegin(LayerBegin {
                    bounds,
                    opacity,
                    clip: composite,
                    fx,
                    is_mask: false,
                    source: None,
                });
                if let Some(mask) = mask {
                    self.out.push(Primitive::LayerBegin(LayerBegin {
                        bounds,
                        opacity: 1.0,
                        clip: Clip::rect(bounds),
                        fx: LayerFx::default(),
                        is_mask: true,
                        source: None,
                    }));
                    let world = self.nodes[mask].parent.map_or(Affine2::IDENTITY, |p| self.world(p));
                    self.forced = Some(mask);
                    self.draw(mask, 1.0, true, clip, world);
                    self.forced = None;
                    self.extent = None;
                    self.out.push(Primitive::LayerEnd);
                }
                self.out.push(Primitive::LayerEnd);
                self.extent = Some(union(outer, bounds));
            }
            None => {
                self.out.truncate(begin);
                self.extent = outer;
            }
        }
    }

    fn emit_source(&mut self, source: NodeId, slot: u8) -> bool {
        let Some(rect) = self.geometry.get(source).copied() else { return false };
        let bounds = pixel_aligned(rect.scale(self.scale));
        if bounds.is_empty() {
            return false;
        }
        let clip = Clip::rect(bounds);
        self.out.push(Primitive::LayerBegin(LayerBegin {
            bounds,
            opacity: 1.0,
            clip,
            fx: LayerFx::default(),
            is_mask: false,
            source: Some(slot),
        }));
        let outer = self.extent.take();
        self.forced = Some(source);
        self.draw(source, 1.0, true, clip, Affine2::IDENTITY);
        self.forced = None;
        self.extent = outer;
        self.out.push(Primitive::LayerEnd);
        true
    }

    fn world(&self, id: NodeId) -> Affine2 {
        let node = &self.nodes[id];
        let parent = node.parent.map_or(Affine2::IDENTITY, |p| self.world(p));
        match self.geometry.get(id).and_then(|rect| node.local_transform(*rect, true)) {
            Some(local) => parent * local,
            None => parent,
        }
    }

    fn layer_fx(&self, effect: LayerEffect, masked: bool) -> LayerFx {
        let reach = effect.blur_max.max(0.0) * self.scale;
        LayerFx {
            masked,
            mask_threshold: effect.mask_threshold.clamp(0.0, 1.0),
            mask_spread: effect.mask_spread.clamp(0.0, 1.0),
            brightness: effect.brightness.clamp(-1.0, 1.0),
            contrast: effect.contrast.clamp(-1.0, 1.0),
            saturation: effect.saturation.clamp(-1.0, 1.0),
            colorization: effect.colorization.clamp(0.0, 1.0),
            colorization_color: effect.colorization_color,
            blur: (effect.blur.clamp(0.0, 1.0) * reach).round(),
            shadow: effect.shadow.map(|s| LayerShadowFx {
                color: s.color.fade(s.opacity.clamp(0.0, 1.0)),
                blur: (s.blur.clamp(0.0, 1.0) * reach).round(),
                offset: Vec2::new((s.offset.x * self.scale).round(), (s.offset.y * self.scale).round()),
            }),
        }
    }

    fn cover(&mut self, rect: Rect) {
        self.extent = Some(union(self.extent, rect.inset(-2.0)));
    }

    fn draw(&mut self, id: NodeId, parent_opacity: f32, own_opacity: bool, clip: Clip, parent: Affine2) {
        let node = &self.nodes[id];
        if !node.visible.get() && self.forced != Some(id) {
            return;
        }
        let opacity = if own_opacity { parent_opacity * node.opacity.get().clamp(0.0, 1.0) } else { parent_opacity };
        if opacity <= 0.0 || clip.is_empty() {
            return;
        }
        let Some(rect) = self.geometry.get(id).copied() else { return };
        let phys = if self.snap { snapped(rect.scale(self.scale)) } else { rect.scale(self.scale) };
        let world = match node.local_transform(rect, true) {
            Some(local) => parent * local,
            None => parent,
        };
        if world.matrix2.determinant() == 0.0 {
            return;
        }
        let transform = Affine2::from_mat2_translation(world.matrix2, world.translation * self.scale);

        match &node.kind {
            Kind::Item => {}
            Kind::Rect(style) => {
                let gradient = style.gradient.get();
                let color = style.color.get();
                let border_width = style.border_width.get().max(0.0);
                let border_color = style.border_color.get();
                let (start, end, orientation) = match gradient {
                    Some(g) => (g.start, g.end, Some(g.orientation)),
                    None => (color, color, None),
                };
                let visible_fill = start.a > 0.0 || end.a > 0.0;
                let visible_border = border_width > 0.0 && border_color.a > 0.0;
                if (visible_fill || visible_border) && !phys.is_empty() {
                    let limit = rect.w.min(rect.h) / 2.0;
                    let radius = style.radius.get().max(0.0).min(limit);
                    let radii = style.corner_radii.get().map_or([radius; 4], |r| r.map(|v| v.max(0.0).min(limit)));
                    self.cover(transformed_bounds(transform, phys));
                    self.out.push(Primitive::Quad(Quad {
                        rect: phys,
                        start: start.fade(opacity),
                        end: end.fade(opacity),
                        orientation,
                        radius: radius * self.scale,
                        radii: radii.map(|v| v * self.scale),
                        border_width: if self.snap && border_width > 0.0 {
                            (border_width * self.scale).round().max(1.0)
                        } else {
                            border_width * self.scale
                        },
                        border_color: border_color.fade(opacity),
                        clip,
                        transform,
                    }));
                }
            }
            Kind::Image(style) => {
                if let Some(entry) = self.images.get(id)
                    && let Some(image) = entry.ready
                    && !phys.is_empty()
                {
                    let placement = place(style.fill_mode.get(), entry.size, rect);
                    self.cover(transformed_bounds(transform, placement.rect.scale(self.scale)));
                    self.out.push(Primitive::Image(ImageDraw {
                        id: image,
                        frame: entry.clock.as_ref().map_or(0, |c| c.frame.get()),
                        rect: placement.rect.scale(self.scale),
                        uv: placement.uv,
                        repeat: placement.repeat,
                        mipmap: style.mipmap.get(),
                        opacity,
                        clip,
                        transform,
                    }));
                }
            }
            Kind::Shader(style) => {
                if !phys.is_empty() {
                    self.cover(transformed_bounds(transform, phys));
                    let mut sources = [false; 2];
                    for (slot, source) in style.sources.iter().enumerate() {
                        if let Some(source) = source.filter(|s| *s != id && self.nodes.contains_key(*s)) {
                            sources[slot] = self.emit_source(source, slot as u8);
                        }
                    }
                    self.out.push(Primitive::Shader(ShaderDraw {
                        effect: style.effect,
                        params: style.params.get(),
                        data: style.data.get(),
                        rect: phys,
                        item_size: Vec2::new(rect.w, rect.h),
                        sources,
                        opacity,
                        clip,
                        transform,
                    }));
                }
            }
            Kind::Text(style) => {
                if let Some(shaped) = self.texts.get(id) {
                    let color = style.color.get().fade(opacity);
                    let h_align = style.h_align.get();
                    let reach = Rect::new(
                        phys.x,
                        phys.y,
                        phys.w.max(shaped.size.x * self.scale),
                        phys.h.max(shaped.size.y * self.scale),
                    );
                    self.cover(transformed_bounds(transform, reach));
                    let dy = (style.v_align.get().offset(rect.h - shaped.size.y) * self.scale).round();
                    let (origin_y, transform) = if transform == Affine2::IDENTITY && !self.smooth {
                        (phys.y, transform)
                    } else {
                        let whole = phys.y.floor();
                        (whole, transform * Affine2::from_translation(Vec2::new(0.0, phys.y - whole)))
                    };
                    let k = raster_scale(transform);
                    let raster = self.scale * k;
                    let transform =
                        if k == 1.0 { transform } else { transform * Affine2::from_scale(Vec2::splat(1.0 / k)) };
                    for run in shaped.buffer.layout_runs().take(shaped.lines) {
                        let line_y = (run.line_y * raster).round() as i32;
                        let dx = (h_align.offset(rect.w - run.line_w) * self.scale).round();
                        for g in run.glyphs {
                            let p = g.physical(((phys.x + dx) * k, (origin_y + dy) * k), raster);
                            self.out.push(Primitive::Glyph(Glyph {
                                key: p.cache_key,
                                x: p.x,
                                y: p.y + line_y,
                                color,
                                clip,
                                transform,
                            }));
                        }
                    }
                }
            }
        }

        let was_smooth = self.smooth;
        self.smooth |= node.smooth;
        let child_clip = if node.clip.get() { self.clip_children(node, rect, phys, transform, clip) } else { clip };
        if !child_clip.is_empty() {
            let begin = self.out.len();
            let nested = clip.shape.is_some() && child_clip.shape != clip.shape;
            let outer = nested.then(|| {
                self.out.push(Primitive::LayerEnd);
                self.extent.take()
            });
            for &child in paint_order(self.nodes, &node.children, true).iter() {
                self.walk(child, opacity, child_clip, world);
            }
            if let Some(outer) = outer {
                self.close_clip_layer(begin, outer, clip, child_clip.rect);
            }
        }
        self.smooth = was_smooth;
    }

    fn close_clip_layer(&mut self, begin: usize, outer: Option<Rect>, clip: Clip, area: Rect) {
        match self.extent.take().map(|c| pixel_aligned(c.intersect(&area))).filter(|b| !b.is_empty()) {
            Some(bounds) => {
                self.out[begin] = Primitive::LayerBegin(LayerBegin {
                    bounds,
                    opacity: 1.0,
                    clip,
                    fx: LayerFx::default(),
                    is_mask: false,
                    source: None,
                });
                self.out.push(Primitive::LayerEnd);
                self.extent = Some(union(outer, bounds));
            }
            None => {
                self.out.truncate(begin);
                self.extent = outer;
            }
        }
    }

    fn clip_children(&self, node: &Node, rect: Rect, phys: Rect, transform: Affine2, clip: Clip) -> Clip {
        let radius = match &node.kind {
            Kind::Rect(style) => style.radius.get().max(0.0).min(rect.w.min(rect.h) / 2.0) * self.scale,
            _ => 0.0,
        };
        let bounds = transformed_bounds(transform, phys);
        let axis_aligned = transform.matrix2.x_axis.y == 0.0 && transform.matrix2.y_axis.x == 0.0;
        let shape = if axis_aligned && radius <= 0.0 {
            clip.shape
        } else {
            Some(ShapeClip { rect: phys, radius, inverse: transform.inverse() })
        };
        Clip { rect: clip.rect.intersect(&bounds), shape }
    }
}

pub(crate) fn transformed_bounds(transform: Affine2, rect: Rect) -> Rect {
    if transform == Affine2::IDENTITY {
        return rect;
    }
    let corners = [
        Vec2::new(rect.x, rect.y),
        Vec2::new(rect.right(), rect.y),
        Vec2::new(rect.x, rect.bottom()),
        Vec2::new(rect.right(), rect.bottom()),
    ]
    .map(|p| transform.transform_point2(p));
    let min = corners.iter().fold(Vec2::splat(f32::INFINITY), |a, p| a.min(*p));
    let max = corners.iter().fold(Vec2::splat(f32::NEG_INFINITY), |a, p| a.max(*p));
    Rect::new(min.x, min.y, max.x - min.x, max.y - min.y)
}

fn raster_scale(transform: Affine2) -> f32 {
    let k = transform.matrix2.determinant().abs().sqrt();
    if !k.is_normal() {
        return 1.0;
    }
    2f32.powf((k.log2() * 4.0).round().clamp(-8.0, 8.0) / 4.0)
}

fn union(a: Option<Rect>, b: Rect) -> Rect {
    match a {
        None => b,
        Some(a) => {
            let x = a.x.min(b.x);
            let y = a.y.min(b.y);
            Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y)
        }
    }
}

fn snapped(rect: Rect) -> Rect {
    let x = rect.x.round();
    let y = rect.y.round();
    Rect::new(x, y, rect.right().round() - x, rect.bottom().round() - y)
}

fn pixel_aligned(rect: Rect) -> Rect {
    let x = rect.x.floor();
    let y = rect.y.floor();
    Rect::new(x, y, rect.right().ceil() - x, rect.bottom().ceil() - y)
}
