use std::any::Any;
use std::borrow::Cow;

use acanthus_anim::{Animatable, Animated};
use acanthus_core::{Affine2, Color, Effect, Rect, Signal, Vec2};
use acanthus_image::{FillMode, ImageStatus};
use slotmap::{SlotMap, new_key_type};

use crate::flick::Flickable;
use crate::input::MouseArea;
use crate::keyboard::{CursorShape, KeyEvent, PointerEvent};
use crate::layout::{Align, Anchors, Constraints, Layout, Margins};
use crate::shader::{ShaderData, ShaderEffect, ShaderParams};
use crate::text::{Elide, HAlign, VAlign, WrapMode};

new_key_type! {
    pub struct NodeId;
}

pub trait IntoSignal<T> {
    fn into_signal(self) -> Signal<T>;
}

impl<T: 'static> IntoSignal<T> for Signal<T> {
    fn into_signal(self) -> Signal<T> {
        self
    }
}

impl<T: 'static> IntoSignal<T> for &Signal<T> {
    fn into_signal(self) -> Signal<T> {
        self.clone()
    }
}

impl<T: Animatable> IntoSignal<T> for &Animated<T> {
    fn into_signal(self) -> Signal<T> {
        self.signal().clone()
    }
}

macro_rules! value_into_signal {
    ($($t:ty),*) => {$(
        impl IntoSignal<$t> for $t {
            fn into_signal(self) -> Signal<$t> {
                Signal::new(self)
            }
        }
    )*};
}

value_into_signal!(
    f32,
    bool,
    u16,
    usize,
    Color,
    String,
    Layout,
    Anchors,
    Align,
    Option<Gradient>,
    Option<f32>,
    HAlign,
    VAlign,
    Elide,
    WrapMode,
    FillMode,
    Constraints,
    Option<Vec2>,
    TransformOrigin,
    ShaderParams,
    ShaderData
);

impl IntoSignal<Option<Vec2>> for Vec2 {
    fn into_signal(self) -> Signal<Option<Vec2>> {
        Signal::new(Some(self))
    }
}

impl IntoSignal<Option<f32>> for f32 {
    fn into_signal(self) -> Signal<Option<f32>> {
        Signal::new(Some(self))
    }
}

impl IntoSignal<String> for &str {
    fn into_signal(self) -> Signal<String> {
        Signal::new(self.to_owned())
    }
}

impl IntoSignal<LayerEffect> for LayerEffect {
    fn into_signal(self) -> Signal<LayerEffect> {
        Signal::new(self)
    }
}

impl IntoSignal<Option<Gradient>> for Gradient {
    fn into_signal(self) -> Signal<Option<Gradient>> {
        Signal::new(Some(self))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformOrigin {
    TopLeft,
    Top,
    TopRight,
    Left,
    #[default]
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl TransformOrigin {
    pub fn point(self, rect: Rect) -> Vec2 {
        let (fx, fy) = match self {
            TransformOrigin::TopLeft => (0.0, 0.0),
            TransformOrigin::Top => (0.5, 0.0),
            TransformOrigin::TopRight => (1.0, 0.0),
            TransformOrigin::Left => (0.0, 0.5),
            TransformOrigin::Center => (0.5, 0.5),
            TransformOrigin::Right => (1.0, 0.5),
            TransformOrigin::BottomLeft => (0.0, 1.0),
            TransformOrigin::Bottom => (0.5, 1.0),
            TransformOrigin::BottomRight => (1.0, 1.0),
        };
        Vec2::new(rect.x + rect.w * fx, rect.y + rect.h * fy)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Orientation {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerShadow {
    pub color: Color,
    pub opacity: f32,
    pub blur: f32,
    pub offset: Vec2,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerEffect {
    pub blur: f32,
    pub blur_max: f32,
    pub shadow: Option<LayerShadow>,
    pub mask: Option<NodeId>,
    pub mask_threshold: f32,
    pub mask_spread: f32,
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub colorization: f32,
    pub colorization_color: Color,
}

impl Default for LayerEffect {
    fn default() -> Self {
        LayerEffect {
            blur: 0.0,
            blur_max: 32.0,
            shadow: None,
            mask: None,
            mask_threshold: 0.0,
            mask_spread: 0.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 0.0,
            colorization: 0.0,
            colorization_color: Color::hex(0xff0000),
        }
    }
}

impl LayerEffect {
    pub fn blur(blur: f32) -> Self {
        LayerEffect { blur, ..Default::default() }
    }

    pub fn shadow(color: Color, opacity: f32, blur: f32, offset: Vec2) -> Self {
        LayerEffect { shadow: Some(LayerShadow { color, opacity, blur, offset }), ..Default::default() }
    }

    pub fn mask(source: NodeId) -> Self {
        LayerEffect { mask: Some(source), ..Default::default() }
    }

    pub fn with_mask(mut self, source: NodeId, threshold: f32, spread: f32) -> Self {
        self.mask = Some(source);
        self.mask_threshold = threshold;
        self.mask_spread = spread;
        self
    }

    pub fn adjust(brightness: f32, contrast: f32, saturation: f32) -> Self {
        LayerEffect { brightness, contrast, saturation, ..Default::default() }
    }

    pub fn colorize(color: Color, amount: f32) -> Self {
        LayerEffect { colorization: amount, colorization_color: color, ..Default::default() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gradient {
    pub start: Color,
    pub end: Color,
    pub orientation: Orientation,
}

impl Gradient {
    pub fn vertical(start: Color, end: Color) -> Self {
        Gradient { start, end, orientation: Orientation::Vertical }
    }

    pub fn horizontal(start: Color, end: Color) -> Self {
        Gradient { start, end, orientation: Orientation::Horizontal }
    }
}

pub struct RectStyle {
    pub color: Signal<Color>,
    pub gradient: Signal<Option<Gradient>>,
    pub radius: Signal<f32>,
    pub corner_radii: Signal<Option<[f32; 4]>>,
    pub border_width: Signal<f32>,
    pub border_color: Signal<Color>,
}

pub struct TextStyle {
    pub text: Signal<String>,
    pub color: Signal<Color>,
    pub size: Signal<f32>,
    pub weight: Signal<u16>,
    pub axes: u8,
    pub styled: bool,
    pub family: Signal<String>,
    pub h_align: Signal<HAlign>,
    pub v_align: Signal<VAlign>,
    pub elide: Signal<Elide>,
    pub wrap: Signal<WrapMode>,
    pub max_lines: Signal<usize>,
    pub line_height: Signal<Option<f32>>,
    pub letter_spacing: Signal<f32>,
}

pub struct ImageStyle {
    pub source: Signal<String>,
    pub fill_mode: Signal<FillMode>,
    pub source_size: Signal<Option<Vec2>>,
    pub status: Signal<ImageStatus>,
    pub animated: Signal<bool>,
}

pub struct ShaderStyle {
    pub effect: ShaderEffect,
    pub params: Signal<ShaderParams>,
    pub data: Signal<ShaderData>,
    pub sources: [Option<NodeId>; 2],
}

pub enum Kind {
    Item,
    Rect(RectStyle),
    Text(TextStyle),
    Image(ImageStyle),
    Shader(ShaderStyle),
}

pub(crate) type KeyHandler = Box<dyn FnMut(&KeyEvent) -> bool>;

pub(crate) type PointerHandler = Box<dyn FnMut(PointerEvent)>;

pub struct Node {
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    pub kind: Kind,
    pub x: Signal<f32>,
    pub y: Signal<f32>,
    pub width: Option<Signal<f32>>,
    pub height: Option<Signal<f32>>,
    pub opacity: Signal<f32>,
    pub visible: Signal<bool>,
    pub clip: Signal<bool>,
    pub smooth: bool,
    pub layer: Signal<bool>,
    pub laid_out_hidden: bool,
    pub layer_effect: Signal<LayerEffect>,
    pub z: Signal<f32>,
    pub scale: Signal<f32>,
    pub rotation: Signal<f32>,
    pub transform_origin: Signal<TransformOrigin>,
    pub anchors: Signal<Anchors>,
    pub layout: Signal<Layout>,
    pub constraints: Signal<Constraints>,
    pub implicit_size: Signal<Vec2>,
    pub bounds: Signal<Rect>,
    pub align: Signal<Align>,
    pub fill_width: Signal<bool>,
    pub fill_height: Signal<bool>,
    pub mouse: Option<MouseArea>,
    pub flick: Option<Flickable>,
    pub cursor: Option<CursorShape>,
    pub focusable: bool,
    pub active_focus: Signal<bool>,
    pub(crate) key: Option<KeyHandler>,
    pub(crate) pointer: Option<PointerHandler>,
    pub(crate) owned: Vec<Box<dyn Any>>,
}

impl Node {
    fn with_kind(kind: Kind) -> Self {
        Node {
            parent: None,
            children: Vec::new(),
            kind,
            x: Signal::new(0.0),
            y: Signal::new(0.0),
            width: None,
            height: None,
            opacity: Signal::new(1.0),
            visible: Signal::new(true),
            clip: Signal::new(false),
            smooth: false,
            layer: Signal::new(false),
            laid_out_hidden: false,
            layer_effect: Signal::new(LayerEffect::default()),
            z: Signal::new(0.0),
            scale: Signal::new(1.0),
            rotation: Signal::new(0.0),
            transform_origin: Signal::new(TransformOrigin::Center),
            anchors: Signal::new(Anchors::default()),
            layout: Signal::new(Layout::Free),
            constraints: Signal::new(Constraints::default()),
            implicit_size: Signal::new(Vec2::ZERO),
            bounds: Signal::new(Rect::default()),
            align: Signal::new(Align::Center),
            fill_width: Signal::new(false),
            fill_height: Signal::new(false),
            mouse: None,
            flick: None,
            cursor: None,
            focusable: false,
            active_focus: Signal::new(false),
            key: None,
            pointer: None,
            owned: Vec::new(),
        }
    }

    pub fn item() -> Self {
        Node::with_kind(Kind::Item)
    }

    pub fn rect() -> Self {
        Node::with_kind(Kind::Rect(RectStyle {
            color: Signal::new(Color::WHITE),
            gradient: Signal::new(None),
            radius: Signal::new(0.0),
            corner_radii: Signal::new(None),
            border_width: Signal::new(0.0),
            border_color: Signal::new(Color::TRANSPARENT),
        }))
    }

    pub fn text(text: impl IntoSignal<String>) -> Self {
        Node::with_kind(Kind::Text(TextStyle {
            text: text.into_signal(),
            color: Signal::new(Color::BLACK),
            size: Signal::new(14.0),
            weight: Signal::new(400),
            axes: 0,
            styled: false,
            family: Signal::new(String::new()),
            h_align: Signal::new(HAlign::Left),
            v_align: Signal::new(VAlign::Top),
            elide: Signal::new(Elide::None),
            wrap: Signal::new(WrapMode::NoWrap),
            max_lines: Signal::new(0),
            line_height: Signal::new(None),
            letter_spacing: Signal::new(0.0),
        }))
    }

    pub fn image(source: impl IntoSignal<String>) -> Self {
        Node::with_kind(Kind::Image(ImageStyle {
            source: source.into_signal(),
            fill_mode: Signal::new(FillMode::Stretch),
            source_size: Signal::new(None),
            status: Signal::new(ImageStatus::Null),
            animated: Signal::new(false),
        }))
    }

    pub fn shader(effect: ShaderEffect, params: impl IntoSignal<ShaderParams>) -> Self {
        Node::with_kind(Kind::Shader(ShaderStyle {
            effect,
            params: params.into_signal(),
            data: Signal::new(ShaderData::default()),
            sources: [None; 2],
        }))
    }

    pub fn shader_data(mut self, data: impl IntoSignal<ShaderData>) -> Self {
        if let Kind::Shader(style) = &mut self.kind {
            style.data = data.into_signal();
        }
        self
    }

    pub fn shader_sources(mut self, sources: [Option<NodeId>; 2]) -> Self {
        if let Kind::Shader(style) = &mut self.kind {
            style.sources = sources;
        }
        self
    }

    pub fn row(spacing: f32) -> Self {
        Node::item().layout(Layout::row(spacing))
    }

    pub fn column(spacing: f32) -> Self {
        Node::item().layout(Layout::column(spacing))
    }

    pub fn x(mut self, v: impl IntoSignal<f32>) -> Self {
        self.x = v.into_signal();
        self
    }

    pub fn y(mut self, v: impl IntoSignal<f32>) -> Self {
        self.y = v.into_signal();
        self
    }

    pub fn width(mut self, v: impl IntoSignal<f32>) -> Self {
        self.width = Some(v.into_signal());
        self
    }

    pub fn height(mut self, v: impl IntoSignal<f32>) -> Self {
        self.height = Some(v.into_signal());
        self
    }

    pub fn size(self, w: impl IntoSignal<f32>, h: impl IntoSignal<f32>) -> Self {
        self.width(w).height(h)
    }

    pub fn opacity(mut self, v: impl IntoSignal<f32>) -> Self {
        self.opacity = v.into_signal();
        self
    }

    pub fn visible(mut self, v: impl IntoSignal<bool>) -> Self {
        self.visible = v.into_signal();
        self
    }

    pub fn clip(mut self, v: impl IntoSignal<bool>) -> Self {
        self.clip = v.into_signal();
        self
    }

    pub fn smooth(mut self, v: bool) -> Self {
        self.smooth = v;
        self
    }

    pub fn mask_source(mut self) -> Self {
        self.visible = Signal::new(false);
        self.laid_out_hidden = true;
        self
    }

    pub fn layer(mut self, v: impl IntoSignal<bool>) -> Self {
        self.layer = v.into_signal();
        self
    }

    pub fn layer_effect(mut self, v: impl IntoSignal<LayerEffect>) -> Self {
        self.layer_effect = v.into_signal();
        self.layer = Signal::new(true);
        self
    }

    pub fn z(mut self, v: impl IntoSignal<f32>) -> Self {
        self.z = v.into_signal();
        self
    }

    pub fn scale(mut self, v: impl IntoSignal<f32>) -> Self {
        self.scale = v.into_signal();
        self
    }

    pub fn rotation(mut self, v: impl IntoSignal<f32>) -> Self {
        self.rotation = v.into_signal();
        self
    }

    pub fn transform_origin(mut self, v: impl IntoSignal<TransformOrigin>) -> Self {
        self.transform_origin = v.into_signal();
        self
    }

    pub(crate) fn local_transform(&self, rect: Rect, tracked: bool) -> Option<Affine2> {
        let scale = read(&self.scale, tracked);
        let rotation = read(&self.rotation, tracked);
        if scale == 1.0 && rotation == 0.0 {
            return None;
        }
        let origin = read(&self.transform_origin, tracked).point(rect);
        Some(
            Affine2::from_translation(origin)
                * Affine2::from_angle(rotation.to_radians())
                * Affine2::from_scale(Vec2::splat(scale))
                * Affine2::from_translation(-origin),
        )
    }

    pub fn anchors(mut self, v: impl IntoSignal<Anchors>) -> Self {
        self.anchors = v.into_signal();
        self
    }

    pub fn layout(mut self, v: impl IntoSignal<Layout>) -> Self {
        self.layout = v.into_signal();
        self
    }

    pub fn constraints(mut self, v: impl IntoSignal<Constraints>) -> Self {
        self.constraints = v.into_signal();
        self
    }

    pub fn implicit_size(&self) -> Signal<Vec2> {
        self.implicit_size.clone()
    }

    pub fn bounds(&self) -> Signal<Rect> {
        self.bounds.clone()
    }

    fn constrain(self, f: impl FnOnce(&mut Constraints)) -> Self {
        self.constraints.update(f);
        self
    }

    pub fn preferred(self, w: f32, h: f32) -> Self {
        self.constrain(|c| {
            c.preferred_width = Some(w);
            c.preferred_height = Some(h);
        })
    }

    pub fn preferred_width(self, w: f32) -> Self {
        self.constrain(|c| c.preferred_width = Some(w))
    }

    pub fn preferred_height(self, h: f32) -> Self {
        self.constrain(|c| c.preferred_height = Some(h))
    }

    pub fn min_size(self, w: f32, h: f32) -> Self {
        self.constrain(|c| {
            c.min_width = w;
            c.min_height = h;
        })
    }

    pub fn max_size(self, w: f32, h: f32) -> Self {
        self.constrain(|c| {
            c.max_width = w;
            c.max_height = h;
        })
    }

    pub fn margins(self, v: Margins) -> Self {
        self.constrain(|c| c.margins = v)
    }

    pub fn span(self, columns: usize, rows: usize) -> Self {
        self.constrain(|c| {
            c.column_span = columns.max(1);
            c.row_span = rows.max(1);
        })
    }

    pub fn align(mut self, v: impl IntoSignal<Align>) -> Self {
        self.align = v.into_signal();
        self
    }

    pub fn fill_width(mut self) -> Self {
        self.fill_width = Signal::new(true);
        self
    }

    pub fn fill_height(mut self) -> Self {
        self.fill_height = Signal::new(true);
        self
    }

    pub fn mouse(mut self, area: MouseArea) -> Self {
        self.mouse = Some(area);
        self
    }

    pub fn cursor(mut self, shape: CursorShape) -> Self {
        self.cursor = Some(shape);
        self
    }

    pub fn focusable(mut self, v: bool) -> Self {
        self.focusable = v;
        self
    }

    pub fn active_focus(&self) -> Signal<bool> {
        self.active_focus.clone()
    }

    pub fn on_key(mut self, f: impl FnMut(&KeyEvent) -> bool + 'static) -> Self {
        self.key = Some(Box::new(f));
        self
    }

    pub fn on_pointer(mut self, f: impl FnMut(PointerEvent) + 'static) -> Self {
        self.pointer = Some(Box::new(f));
        self
    }

    pub fn bind(self, effect: Effect) -> Self {
        self.own(effect)
    }

    pub fn own(mut self, value: impl Any) -> Self {
        self.owned.push(Box::new(value));
        self
    }

    pub fn keep(&mut self, value: impl Any) {
        self.owned.push(Box::new(value));
    }

    pub fn color(mut self, v: impl IntoSignal<Color>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => s.color = v.into_signal(),
            Kind::Text(s) => s.color = v.into_signal(),
            Kind::Item | Kind::Image(_) | Kind::Shader(_) => {
                tracing::warn!("color() has no effect on an Item, Image or Shader")
            }
        }
        self
    }

    pub fn gradient(mut self, v: impl IntoSignal<Option<Gradient>>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => s.gradient = v.into_signal(),
            _ => tracing::warn!("gradient() only applies to Rect"),
        }
        self
    }

    pub fn radius(mut self, v: impl IntoSignal<f32>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => s.radius = v.into_signal(),
            _ => tracing::warn!("radius() only applies to Rect"),
        }
        self
    }

    pub fn corner_radii(mut self, v: impl IntoSignal<Option<[f32; 4]>>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => s.corner_radii = v.into_signal(),
            _ => tracing::warn!("corner_radii() only applies to Rect"),
        }
        self
    }

    pub fn border(mut self, width: impl IntoSignal<f32>, color: impl IntoSignal<Color>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => {
                s.border_width = width.into_signal();
                s.border_color = color.into_signal();
            }
            _ => tracing::warn!("border() only applies to Rect"),
        }
        self
    }

    pub fn font_size(mut self, v: impl IntoSignal<f32>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.size = v.into_signal(),
            _ => tracing::warn!("font_size() only applies to Text"),
        }
        self
    }

    pub fn font_weight(mut self, v: impl IntoSignal<u16>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.weight = v.into_signal(),
            _ => tracing::warn!("font_weight() only applies to Text"),
        }
        self
    }

    pub fn styled(mut self, v: bool) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.styled = v,
            _ => tracing::warn!("styled() only applies to Text"),
        }
        self
    }

    pub fn font_axes(mut self, axes: &[(&str, f32)]) -> Self {
        match &mut self.kind {
            Kind::Text(s) => {
                let tags: Vec<_> = axes
                    .iter()
                    .filter_map(|(tag, value)| Some((<[u8; 4]>::try_from(tag.as_bytes()).ok()?, *value)))
                    .collect();
                s.axes = crate::axes::register(&tags);
            }
            _ => tracing::warn!("font_axes() only applies to Text"),
        }
        self
    }

    pub fn font_family(mut self, v: impl IntoSignal<String>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.family = v.into_signal(),
            _ => tracing::warn!("font_family() only applies to Text"),
        }
        self
    }

    pub fn h_align(mut self, v: impl IntoSignal<HAlign>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.h_align = v.into_signal(),
            _ => tracing::warn!("h_align() only applies to Text"),
        }
        self
    }

    pub fn v_align(mut self, v: impl IntoSignal<VAlign>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.v_align = v.into_signal(),
            _ => tracing::warn!("v_align() only applies to Text"),
        }
        self
    }

    pub fn elide(mut self, v: impl IntoSignal<Elide>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.elide = v.into_signal(),
            _ => tracing::warn!("elide() only applies to Text"),
        }
        self
    }

    pub fn wrap_mode(mut self, v: impl IntoSignal<WrapMode>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.wrap = v.into_signal(),
            _ => tracing::warn!("wrap_mode() only applies to Text"),
        }
        self
    }

    pub fn max_lines(mut self, v: impl IntoSignal<usize>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.max_lines = v.into_signal(),
            _ => tracing::warn!("max_lines() only applies to Text"),
        }
        self
    }

    pub fn line_height(mut self, v: impl IntoSignal<Option<f32>>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.line_height = v.into_signal(),
            _ => tracing::warn!("line_height() only applies to Text"),
        }
        self
    }

    pub fn letter_spacing(mut self, v: impl IntoSignal<f32>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.letter_spacing = v.into_signal(),
            _ => tracing::warn!("letter_spacing() only applies to Text"),
        }
        self
    }

    pub fn fill_mode(mut self, v: impl IntoSignal<FillMode>) -> Self {
        match &mut self.kind {
            Kind::Image(s) => s.fill_mode = v.into_signal(),
            _ => tracing::warn!("fill_mode() only applies to Image"),
        }
        self
    }

    pub fn source_size(mut self, v: impl IntoSignal<Option<Vec2>>) -> Self {
        match &mut self.kind {
            Kind::Image(s) => s.source_size = v.into_signal(),
            _ => tracing::warn!("source_size() only applies to Image"),
        }
        self
    }

    pub fn animated(mut self, v: impl IntoSignal<bool>) -> Self {
        match &mut self.kind {
            Kind::Image(s) => s.animated = v.into_signal(),
            _ => tracing::warn!("animated() only applies to Image"),
        }
        self
    }

    pub fn image_status(&self) -> Option<Signal<ImageStatus>> {
        match &self.kind {
            Kind::Image(s) => Some(s.status.clone()),
            _ => None,
        }
    }

    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn children(&self) -> &[NodeId] {
        &self.children
    }
}

fn read<T: Clone + 'static>(signal: &Signal<T>, tracked: bool) -> T {
    if tracked { signal.get() } else { signal.get_untracked() }
}

pub(crate) fn paint_order<'a>(
    nodes: &SlotMap<NodeId, Node>,
    children: &'a [NodeId],
    tracked: bool,
) -> Cow<'a, [NodeId]> {
    let Some(first) = children.first() else { return Cow::Borrowed(children) };
    let first_z = read(&nodes[*first].z, tracked);
    let mut uniform = true;
    for child in &children[1..] {
        uniform &= read(&nodes[*child].z, tracked) == first_z;
    }
    if uniform {
        return Cow::Borrowed(children);
    }
    let mut order: Vec<(f32, NodeId)> = children.iter().map(|c| (read(&nodes[*c].z, false), *c)).collect();
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    Cow::Owned(order.into_iter().map(|(_, c)| c).collect())
}
