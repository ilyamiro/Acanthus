use std::any::Any;
use std::borrow::Cow;

use acanthus_anim::{Animatable, Animated};
use acanthus_core::{Affine2, Color, Effect, Rect, Signal, Vec2};
use acanthus_image::{FillMode, ImageStatus, MemorySlot, Painting};
use slotmap::{SlotMap, new_key_type};
use std::cell::RefCell;

use crate::flick::Flickable;
use crate::input::MouseArea;
use crate::keyboard::{CursorShape, KeyEvent, PointerEvent};
use crate::layout::{Align, Anchors, Constraints, Layout, Margins};
use crate::shader::{ShaderData, ShaderEffect, ShaderParams};
use crate::text::{Capitalization, Elide, HAlign, VAlign, WrapMode};

new_key_type! {
    /// Identifies a node of a [`Scene`]; stays valid until the node is removed.
    pub struct NodeId;
}

/// Conversion into a [`Signal`]: plain values become constant signals, signals pass through, so builder methods accept either.
pub trait IntoSignal<T> {
    /// Performs the conversion.
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
    Capitalization,
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

/// The point a node's scale and rotation pivot around.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransformOrigin {
    /// The top left corner.
    TopLeft,
    /// The middle of the top edge.
    Top,
    /// The top right corner.
    TopRight,
    /// The middle of the left edge.
    Left,
    /// The centre.
    #[default]
    Center,
    /// The middle of the right edge.
    Right,
    /// The bottom left corner.
    BottomLeft,
    /// The middle of the bottom edge.
    Bottom,
    /// The bottom right corner.
    BottomRight,
}

impl TransformOrigin {
    /// The pivot point inside `rect`.
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

/// The axis a gradient or list runs along.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Orientation {
    /// Top to bottom.
    #[default]
    Vertical,
    /// Left to right.
    Horizontal,
}

/// A drop shadow of a layer effect.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerShadow {
    /// Shadow colour.
    pub color: Color,
    /// Shadow opacity from 0 to 1.
    pub opacity: f32,
    /// Blur radius.
    pub blur: f32,
    /// Offset from the node.
    pub offset: Vec2,
}

/// Post-processing of a node rendered as a layer: blur, shadow, mask and colour adjustments.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayerEffect {
    /// Blur radius, zero for none.
    pub blur: f32,
    /// Upper bound for `blur` so animated values stay cheap.
    pub blur_max: f32,
    /// Drop shadow.
    pub shadow: Option<LayerShadow>,
    /// Node whose rendering is used as an alpha mask.
    pub mask: Option<NodeId>,
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
    /// A blur of the given radius.
    pub fn blur(blur: f32) -> Self {
        LayerEffect { blur, ..Default::default() }
    }

    /// A drop shadow.
    pub fn shadow(color: Color, opacity: f32, blur: f32, offset: Vec2) -> Self {
        LayerEffect { shadow: Some(LayerShadow { color, opacity, blur, offset }), ..Default::default() }
    }

    /// A mask taken from the rendering of `source`.
    pub fn mask(source: NodeId) -> Self {
        LayerEffect { mask: Some(source), ..Default::default() }
    }

    /// Adds a mask from `source` with the given alpha `threshold` and edge `spread`.
    pub fn with_mask(mut self, source: NodeId, threshold: f32, spread: f32) -> Self {
        self.mask = Some(source);
        self.mask_threshold = threshold;
        self.mask_spread = spread;
        self
    }

    /// Brightness, contrast and saturation adjustments.
    pub fn adjust(brightness: f32, contrast: f32, saturation: f32) -> Self {
        LayerEffect { brightness, contrast, saturation, ..Default::default() }
    }

    /// A colour overlay of `color` at strength `amount`.
    pub fn colorize(color: Color, amount: f32) -> Self {
        LayerEffect { colorization: amount, colorization_color: color, ..Default::default() }
    }
}

/// A two-colour gradient fill.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gradient {
    /// The colour at the start.
    pub start: Color,
    /// The colour at the end.
    pub end: Color,
    /// The direction the gradient runs along.
    pub orientation: Orientation,
}

impl Gradient {
    /// A top to bottom gradient.
    pub fn vertical(start: Color, end: Color) -> Self {
        Gradient { start, end, orientation: Orientation::Vertical }
    }

    /// A left to right gradient.
    pub fn horizontal(start: Color, end: Color) -> Self {
        Gradient { start, end, orientation: Orientation::Horizontal }
    }
}

/// Visual properties of a rectangle node.
pub struct RectStyle {
    /// Fill colour.
    pub color: Signal<Color>,
    /// Gradient replacing the fill colour, if any.
    pub gradient: Signal<Option<Gradient>>,
    /// Uniform corner radius.
    pub radius: Signal<f32>,
    /// Per-corner radii overriding `radius`.
    pub corner_radii: Signal<Option<[f32; 4]>>,
    /// Border width.
    pub border_width: Signal<f32>,
    /// Border colour.
    pub border_color: Signal<Color>,
}

/// Properties of a text node.
pub struct TextStyle {
    /// The string shown.
    pub text: Signal<String>,
    /// Text colour.
    pub color: Signal<Color>,
    /// Font size in logical pixels.
    pub size: Signal<f32>,
    /// Font weight from 100 to 900.
    pub weight: Signal<u16>,
    /// Registered variable-font axis profile, zero for none.
    pub axes: u8,
    /// True to request the bold or italic style variants of the family.
    pub styled: bool,
    /// Font family, empty for the default.
    pub family: Signal<String>,
    /// Horizontal alignment.
    pub h_align: Signal<HAlign>,
    /// False until [`Node::h_align`] is called; while false, right-to-left text aligns right as in QML.
    pub h_align_explicit: bool,
    /// Vertical alignment.
    pub v_align: Signal<VAlign>,
    /// Eliding of truncated text.
    pub elide: Signal<Elide>,
    /// Line wrapping.
    pub wrap: Signal<WrapMode>,
    /// Largest number of lines, zero for unlimited.
    pub max_lines: Signal<usize>,
    /// Line height in logical pixels, or `None` for the font's own.
    pub line_height: Signal<Option<f32>>,
    /// Extra spacing after every glyph.
    pub letter_spacing: Signal<f32>,
    /// Letter case applied when shaping.
    pub capitalization: Signal<Capitalization>,
}

/// Properties of an image node.
pub struct ImageStyle {
    /// Path or URL of the image.
    pub source: Signal<String>,
    /// How the image fits the node.
    pub fill_mode: Signal<FillMode>,
    /// Natural size of the image once known, or an override.
    pub source_size: Signal<Option<Vec2>>,
    /// Loading state.
    pub status: Signal<ImageStatus>,
    /// True to play animated images.
    pub animated: Signal<bool>,
    /// True to sample from a mipmap chain, which keeps strong downscales smooth (Qt's `Image.mipmap`).
    pub mipmap: Signal<bool>,
    pub(crate) canvas: Option<Canvas>,
}

pub(crate) struct Canvas {
    painting: Signal<Painting>,
    slot: MemorySlot,
    painted: RefCell<Option<(Painting, f32, String)>>,
}

impl Canvas {
    pub(crate) fn source(&self, scale: f32) -> String {
        self.painting.with(|painting| {
            let mut painted = self.painted.borrow_mut();
            if let Some((last, last_scale, source)) = &*painted
                && last == painting
                && *last_scale == scale
            {
                return source.clone();
            }
            let source = painting
                .rasterize(scale)
                .map(|image| self.slot.set(image.width, image.height, image.rgba))
                .unwrap_or_default();
            *painted = Some((painting.clone(), scale, source.clone()));
            source
        })
    }
}

/// Properties of a shader node.
pub struct ShaderStyle {
    /// The effect.
    pub effect: ShaderEffect,
    /// Its parameter slots.
    pub params: Signal<ShaderParams>,
    /// Its array data.
    pub data: Signal<ShaderData>,
    /// Nodes whose rendering the effect samples as source layers.
    pub sources: [Option<NodeId>; 2],
}

/// What a node draws.
pub enum Kind {
    /// Nothing; a plain container.
    Item,
    /// A rectangle.
    Rect(RectStyle),
    /// Text.
    Text(TextStyle),
    /// An image.
    Image(ImageStyle),
    /// A shader effect.
    Shader(ShaderStyle),
}

pub(crate) type KeyHandler = Box<dyn FnMut(&KeyEvent) -> bool>;

pub(crate) type PointerHandler = Box<dyn FnMut(PointerEvent)>;

/// A node of the scene tree under construction: its kind, geometry, appearance, layout and input properties. Built with chained methods and added with [`Scene::add`](crate::Scene::add).
pub struct Node {
    pub(crate) parent: Option<NodeId>,
    pub(crate) children: Vec<NodeId>,
    /// What the node draws.
    pub kind: Kind,
    /// Horizontal position in the parent.
    pub x: Signal<f32>,
    /// Vertical position in the parent.
    pub y: Signal<f32>,
    /// Explicit width, or `None` to use the implicit size.
    pub width: Option<Signal<f32>>,
    /// Explicit height, or `None` to use the implicit size.
    pub height: Option<Signal<f32>>,
    /// Opacity from 0 to 1, multiplied down the tree.
    pub opacity: Signal<f32>,
    /// Whether the node and its children are drawn.
    pub visible: Signal<bool>,
    /// Whether children are clipped to the node's bounds.
    pub clip: Signal<bool>,
    /// Whether the clip edge is anti-aliased.
    pub smooth: bool,
    /// Whether the node is rendered into its own layer.
    pub layer: Signal<bool>,
    /// True when the node is invisible but still takes part in layout.
    pub laid_out_hidden: bool,
    /// Post-processing of the layer.
    pub layer_effect: Signal<LayerEffect>,
    /// Paint order among siblings; higher is on top.
    pub z: Signal<f32>,
    /// Scale factor around the transform origin.
    pub scale: Signal<f32>,
    /// Rotation in degrees around the transform origin.
    pub rotation: Signal<f32>,
    /// Pivot of scale and rotation.
    pub transform_origin: Signal<TransformOrigin>,
    /// Anchors to the parent or siblings.
    pub anchors: Signal<Anchors>,
    /// How the children are arranged.
    pub layout: Signal<Layout>,
    /// Size hints for the parent's layout.
    pub constraints: Signal<Constraints>,
    /// Size the node's content asks for.
    pub implicit_size: Signal<Vec2>,
    /// The rectangle the node ended up with after layout.
    pub bounds: Signal<Rect>,
    /// Alignment inside the layout cell.
    pub align: Signal<Align>,
    /// True to fill the cell's width.
    pub fill_width: Signal<bool>,
    /// True to fill the cell's height.
    pub fill_height: Signal<bool>,
    /// Pointer handling.
    pub mouse: Option<MouseArea>,
    /// Scrolling state, for a flickable viewport.
    pub flick: Option<Flickable>,
    /// Cursor shape requested while hovered.
    pub cursor: Option<CursorShape>,
    /// True when the node can take keyboard focus.
    pub focusable: bool,
    /// True while the node has active focus.
    pub active_focus: Signal<bool>,
    pub(crate) key: Option<KeyHandler>,
    pub(crate) pointer: Option<PointerHandler>,
    pub(crate) drop: Option<crate::drag::DropArea>,
    pub(crate) drag_source: Option<crate::drag::DragSource>,
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
            drop: None,
            drag_source: None,
            owned: Vec::new(),
        }
    }

    /// An empty container node.
    pub fn item() -> Self {
        Node::with_kind(Kind::Item)
    }

    /// A rectangle node, white by default.
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

    /// A text node showing `text`; 14 px, weight 400, black and left aligned (right for right-to-left text) by default.
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
            h_align_explicit: false,
            v_align: Signal::new(VAlign::Top),
            elide: Signal::new(Elide::None),
            wrap: Signal::new(WrapMode::NoWrap),
            max_lines: Signal::new(0),
            line_height: Signal::new(None),
            letter_spacing: Signal::new(0.0),
            capitalization: Signal::new(Capitalization::MixedCase),
        }))
    }

    /// An image node loading `source`.
    pub fn image(source: impl IntoSignal<String>) -> Self {
        Node::with_kind(Kind::Image(ImageStyle {
            source: source.into_signal(),
            fill_mode: Signal::new(FillMode::Stretch),
            source_size: Signal::new(None),
            status: Signal::new(ImageStatus::Null),
            animated: Signal::new(false),
            mipmap: Signal::new(false),
            canvas: None,
        }))
    }

    /// A node showing what `paint` records, like Qt's `Canvas` with its default image target: `paint` runs again when the node is resized or a signal it reads changes, and the painting is rasterised at the device scale. Its implicit size is zero.
    pub fn canvas(mut paint: impl FnMut(&mut Painting) + 'static) -> Self {
        let mut node = Node::image(String::new());
        let bounds = node.bounds.clone();
        let painting = Signal::new(Painting::default());
        let effect = painting.bind(move || {
            let rect = bounds.get();
            let mut painting = Painting::new(Vec2::new(rect.w, rect.h));
            paint(&mut painting);
            painting
        });
        if let Kind::Image(style) = &mut node.kind {
            style.canvas = Some(Canvas { painting, slot: MemorySlot::new(), painted: RefCell::new(None) });
        }
        node.own(effect)
    }

    /// A node drawing `effect` with `params`.
    pub fn shader(effect: ShaderEffect, params: impl IntoSignal<ShaderParams>) -> Self {
        Node::with_kind(Kind::Shader(ShaderStyle {
            effect,
            params: params.into_signal(),
            data: Signal::new(ShaderData::default()),
            sources: [None; 2],
        }))
    }

    /// Sets the array data of a shader node.
    pub fn shader_data(mut self, data: impl IntoSignal<ShaderData>) -> Self {
        if let Kind::Shader(style) = &mut self.kind {
            style.data = data.into_signal();
        }
        self
    }

    /// Sets the nodes a shader node samples as source layers.
    pub fn shader_sources(mut self, sources: [Option<NodeId>; 2]) -> Self {
        if let Kind::Shader(style) = &mut self.kind {
            style.sources = sources;
        }
        self
    }

    /// A container laying its children out in a row.
    pub fn row(spacing: f32) -> Self {
        Node::item().layout(Layout::row(spacing))
    }

    /// A container laying its children out in a column.
    pub fn column(spacing: f32) -> Self {
        Node::item().layout(Layout::column(spacing))
    }

    /// Sets the horizontal position.
    pub fn x(mut self, v: impl IntoSignal<f32>) -> Self {
        self.x = v.into_signal();
        self
    }

    /// Sets the vertical position.
    pub fn y(mut self, v: impl IntoSignal<f32>) -> Self {
        self.y = v.into_signal();
        self
    }

    /// Sets the width.
    pub fn width(mut self, v: impl IntoSignal<f32>) -> Self {
        self.width = Some(v.into_signal());
        self
    }

    /// Sets the height.
    pub fn height(mut self, v: impl IntoSignal<f32>) -> Self {
        self.height = Some(v.into_signal());
        self
    }

    /// Sets width and height.
    pub fn size(self, w: impl IntoSignal<f32>, h: impl IntoSignal<f32>) -> Self {
        self.width(w).height(h)
    }

    /// Sets the opacity.
    pub fn opacity(mut self, v: impl IntoSignal<f32>) -> Self {
        self.opacity = v.into_signal();
        self
    }

    /// Sets whether the node is drawn.
    pub fn visible(mut self, v: impl IntoSignal<bool>) -> Self {
        self.visible = v.into_signal();
        self
    }

    /// Sets whether children are clipped to the node.
    pub fn clip(mut self, v: impl IntoSignal<bool>) -> Self {
        self.clip = v.into_signal();
        self
    }

    /// Sets whether the clip edge is anti-aliased.
    pub fn smooth(mut self, v: bool) -> Self {
        self.smooth = v;
        self
    }

    /// Marks the node as a mask source: it is rendered for another node's mask and not shown itself.
    pub fn mask_source(mut self) -> Self {
        self.visible = Signal::new(false);
        self.laid_out_hidden = true;
        self
    }

    /// Sets whether the node is rendered into its own layer.
    pub fn layer(mut self, v: impl IntoSignal<bool>) -> Self {
        self.layer = v.into_signal();
        self
    }

    /// Sets the layer effect and turns layer rendering on.
    pub fn layer_effect(mut self, v: impl IntoSignal<LayerEffect>) -> Self {
        self.layer_effect = v.into_signal();
        self.layer = Signal::new(true);
        self
    }

    /// Sets the paint order among siblings.
    pub fn z(mut self, v: impl IntoSignal<f32>) -> Self {
        self.z = v.into_signal();
        self
    }

    /// Sets the scale factor.
    pub fn scale(mut self, v: impl IntoSignal<f32>) -> Self {
        self.scale = v.into_signal();
        self
    }

    /// Sets the rotation in degrees.
    pub fn rotation(mut self, v: impl IntoSignal<f32>) -> Self {
        self.rotation = v.into_signal();
        self
    }

    /// Sets the pivot of scale and rotation.
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

    /// Sets the anchors.
    pub fn anchors(mut self, v: impl IntoSignal<Anchors>) -> Self {
        self.anchors = v.into_signal();
        self
    }

    /// Sets the child layout.
    pub fn layout(mut self, v: impl IntoSignal<Layout>) -> Self {
        self.layout = v.into_signal();
        self
    }

    /// Sets the layout constraints.
    pub fn constraints(mut self, v: impl IntoSignal<Constraints>) -> Self {
        self.constraints = v.into_signal();
        self
    }

    /// The implicit size signal, written by the scene when content is measured.
    pub fn implicit_size(&self) -> Signal<Vec2> {
        self.implicit_size.clone()
    }

    /// The bounds signal, written by the scene after layout.
    pub fn bounds(&self) -> Signal<Rect> {
        self.bounds.clone()
    }

    fn constrain(self, f: impl FnOnce(&mut Constraints)) -> Self {
        self.constraints.update(f);
        self
    }

    /// Sets the preferred width and height.
    pub fn preferred(self, w: f32, h: f32) -> Self {
        self.constrain(|c| {
            c.preferred_width = Some(w);
            c.preferred_height = Some(h);
        })
    }

    /// Sets the preferred width.
    pub fn preferred_width(self, w: f32) -> Self {
        self.constrain(|c| c.preferred_width = Some(w))
    }

    /// Sets the preferred height.
    pub fn preferred_height(self, h: f32) -> Self {
        self.constrain(|c| c.preferred_height = Some(h))
    }

    /// Sets the smallest width and height.
    pub fn min_size(self, w: f32, h: f32) -> Self {
        self.constrain(|c| {
            c.min_width = w;
            c.min_height = h;
        })
    }

    /// Sets the largest width and height.
    pub fn max_size(self, w: f32, h: f32) -> Self {
        self.constrain(|c| {
            c.max_width = w;
            c.max_height = h;
        })
    }

    /// Sets the margins.
    pub fn margins(self, v: Margins) -> Self {
        self.constrain(|c| c.margins = v)
    }

    /// Sets how many grid columns and rows the node spans, at least one each.
    pub fn span(self, columns: usize, rows: usize) -> Self {
        self.constrain(|c| {
            c.column_span = columns.max(1);
            c.row_span = rows.max(1);
        })
    }

    /// Sets the alignment inside the layout cell.
    pub fn align(mut self, v: impl IntoSignal<Align>) -> Self {
        self.align = v.into_signal();
        self
    }

    /// Makes the node fill the width of its cell.
    pub fn fill_width(mut self) -> Self {
        self.fill_width = Signal::new(true);
        self
    }

    /// Makes the node fill the height of its cell.
    pub fn fill_height(mut self) -> Self {
        self.fill_height = Signal::new(true);
        self
    }

    /// Attaches pointer handling.
    pub fn mouse(mut self, area: MouseArea) -> Self {
        self.mouse = Some(area);
        self
    }

    /// Makes the node receive drags, inside the scene and from other applications.
    pub fn drop_area(mut self, area: crate::drag::DropArea) -> Self {
        self.drop = Some(area);
        self
    }

    /// Makes the node the start of drags inside the scene.
    pub fn drag_source(mut self, source: crate::drag::DragSource) -> Self {
        self.drag_source = Some(source);
        self
    }

    /// Sets the cursor shown while hovered.
    pub fn cursor(mut self, shape: CursorShape) -> Self {
        self.cursor = Some(shape);
        self
    }

    /// Sets whether the node can take keyboard focus.
    pub fn focusable(mut self, v: bool) -> Self {
        self.focusable = v;
        self
    }

    /// True while the node has active focus.
    pub fn active_focus(&self) -> Signal<bool> {
        self.active_focus.clone()
    }

    /// Sets the key handler; it returns true when it consumed the event.
    pub fn on_key(mut self, f: impl FnMut(&KeyEvent) -> bool + 'static) -> Self {
        self.key = Some(Box::new(f));
        self
    }

    /// Sets the handler for pointer press, drag and release samples.
    pub fn on_pointer(mut self, f: impl FnMut(PointerEvent) + 'static) -> Self {
        self.pointer = Some(Box::new(f));
        self
    }

    /// Ties the lifetime of `effect` to the node.
    pub fn bind(self, effect: Effect) -> Self {
        self.own(effect)
    }

    /// Ties the lifetime of any value to the node, for example a subscription.
    pub fn own(mut self, value: impl Any) -> Self {
        self.owned.push(Box::new(value));
        self
    }

    /// Like [`Node::own`] on a node already built.
    pub fn keep(&mut self, value: impl Any) {
        self.owned.push(Box::new(value));
    }

    /// Sets the colour of a rectangle or text node; ignored with a warning on other kinds.
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

    /// Sets the gradient of a rectangle node.
    pub fn gradient(mut self, v: impl IntoSignal<Option<Gradient>>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => s.gradient = v.into_signal(),
            _ => tracing::warn!("gradient() only applies to Rect"),
        }
        self
    }

    /// Sets the corner radius of a rectangle node.
    pub fn radius(mut self, v: impl IntoSignal<f32>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => s.radius = v.into_signal(),
            _ => tracing::warn!("radius() only applies to Rect"),
        }
        self
    }

    /// Sets per-corner radii of a rectangle node, top left, top right, bottom right, bottom left.
    pub fn corner_radii(mut self, v: impl IntoSignal<Option<[f32; 4]>>) -> Self {
        match &mut self.kind {
            Kind::Rect(s) => s.corner_radii = v.into_signal(),
            _ => tracing::warn!("corner_radii() only applies to Rect"),
        }
        self
    }

    /// Sets the border of a rectangle node.
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

    /// Sets the font size of a text node.
    pub fn font_size(mut self, v: impl IntoSignal<f32>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.size = v.into_signal(),
            _ => tracing::warn!("font_size() only applies to Text"),
        }
        self
    }

    /// Sets the font weight of a text node.
    pub fn font_weight(mut self, v: impl IntoSignal<u16>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.weight = v.into_signal(),
            _ => tracing::warn!("font_weight() only applies to Text"),
        }
        self
    }

    /// Sets whether a text node uses styled variants of its family.
    pub fn styled(mut self, v: bool) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.styled = v,
            _ => tracing::warn!("styled() only applies to Text"),
        }
        self
    }

    /// Sets variable-font axes of a text node from four-letter tags such as `wdth`.
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

    /// Sets the font family of a text node.
    pub fn font_family(mut self, v: impl IntoSignal<String>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.family = v.into_signal(),
            _ => tracing::warn!("font_family() only applies to Text"),
        }
        self
    }

    /// Sets the horizontal alignment of a text node; without it, text whose first strong character is right-to-left aligns right.
    pub fn h_align(mut self, v: impl IntoSignal<HAlign>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => {
                s.h_align = v.into_signal();
                s.h_align_explicit = true;
            }
            _ => tracing::warn!("h_align() only applies to Text"),
        }
        self
    }

    /// Sets the letter case of a text node (QML `font.capitalization`).
    pub fn capitalization(mut self, v: impl IntoSignal<Capitalization>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.capitalization = v.into_signal(),
            _ => tracing::warn!("capitalization() only applies to Text"),
        }
        self
    }

    /// Sets the vertical alignment of a text node.
    pub fn v_align(mut self, v: impl IntoSignal<VAlign>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.v_align = v.into_signal(),
            _ => tracing::warn!("v_align() only applies to Text"),
        }
        self
    }

    /// Sets the eliding of a text node.
    pub fn elide(mut self, v: impl IntoSignal<Elide>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.elide = v.into_signal(),
            _ => tracing::warn!("elide() only applies to Text"),
        }
        self
    }

    /// Sets the wrapping of a text node.
    pub fn wrap_mode(mut self, v: impl IntoSignal<WrapMode>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.wrap = v.into_signal(),
            _ => tracing::warn!("wrap_mode() only applies to Text"),
        }
        self
    }

    /// Sets the largest number of lines of a text node, zero for unlimited.
    pub fn max_lines(mut self, v: impl IntoSignal<usize>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.max_lines = v.into_signal(),
            _ => tracing::warn!("max_lines() only applies to Text"),
        }
        self
    }

    /// Sets the line height of a text node.
    pub fn line_height(mut self, v: impl IntoSignal<Option<f32>>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.line_height = v.into_signal(),
            _ => tracing::warn!("line_height() only applies to Text"),
        }
        self
    }

    /// Sets the letter spacing of a text node.
    pub fn letter_spacing(mut self, v: impl IntoSignal<f32>) -> Self {
        match &mut self.kind {
            Kind::Text(s) => s.letter_spacing = v.into_signal(),
            _ => tracing::warn!("letter_spacing() only applies to Text"),
        }
        self
    }

    /// Sets how an image node fits its box.
    pub fn fill_mode(mut self, v: impl IntoSignal<FillMode>) -> Self {
        match &mut self.kind {
            Kind::Image(s) => s.fill_mode = v.into_signal(),
            _ => tracing::warn!("fill_mode() only applies to Image"),
        }
        self
    }

    /// Sets the source size of an image node.
    pub fn source_size(mut self, v: impl IntoSignal<Option<Vec2>>) -> Self {
        match &mut self.kind {
            Kind::Image(s) => s.source_size = v.into_signal(),
            _ => tracing::warn!("source_size() only applies to Image"),
        }
        self
    }

    /// Sets whether an image node plays animation.
    pub fn animated(mut self, v: impl IntoSignal<bool>) -> Self {
        match &mut self.kind {
            Kind::Image(s) => s.animated = v.into_signal(),
            _ => tracing::warn!("animated() only applies to Image"),
        }
        self
    }

    /// Sets whether an image node is drawn from a mipmap chain, off by default as in Qt.
    pub fn mipmap(mut self, v: impl IntoSignal<bool>) -> Self {
        match &mut self.kind {
            Kind::Image(s) => s.mipmap = v.into_signal(),
            _ => tracing::warn!("mipmap() only applies to Image"),
        }
        self
    }

    /// The loading state of an image node, or `None` for other kinds.
    pub fn image_status(&self) -> Option<Signal<ImageStatus>> {
        match &self.kind {
            Kind::Image(s) => Some(s.status.clone()),
            _ => None,
        }
    }

    /// The parent in the tree.
    pub fn parent(&self) -> Option<NodeId> {
        self.parent
    }

    /// The children in insertion order.
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
