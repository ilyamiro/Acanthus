//! Single-line search field with a placeholder and a clear button that appears once text is typed.

use std::rc::Rc;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Signal};
use acanthus_scene::{Anchors, CursorShape, MouseArea, Node, NodeId, Scene, TextInput, TextInputStyle};

use crate::timing::FAST;

const CLEAR_GLYPH: &str = "\u{f01b4}";

/// Colours and fonts a search field is drawn with.
#[derive(Clone)]
pub struct SearchTokens {
    /// Font family of the typed text.
    pub font_family: Signal<String>,
    /// Font family of the clear glyph.
    pub icon_font: Signal<String>,
    /// Typed text and hovered clear glyph colour.
    pub text: Signal<Color>,
    /// Placeholder and idle clear glyph colour.
    pub muted: Signal<Color>,
    /// Caret and selection colour.
    pub accent: Signal<Color>,
}

/// Description of a search field.
pub struct SearchField<'a> {
    /// Colours and fonts.
    pub tokens: &'a SearchTokens,
    /// Text input that holds the query; the caller reads its text.
    pub input: &'a TextInput,
    /// Hint shown while the field is empty.
    pub placeholder: Signal<String>,
    /// Font size of the text and the clear glyph.
    pub font_size: f32,
    /// Horizontal padding inside the field.
    pub pad: f32,
    /// Gap between the text and the clear button, also added around the glyph.
    pub gap: f32,
    /// Width of the field.
    pub width: Signal<f32>,
    /// Height of the field; the text and the clear button are centred in it.
    pub height: f32,
    /// Gives the field keyboard focus while true.
    pub focus: Signal<bool>,
    /// Called after the clear button emptied the field.
    pub on_clear: Rc<dyn Fn()>,
}

/// Builds the field's text input and clear button under `parent`; the derived signals live in `keep`.
pub fn search_field(scene: &mut Scene, parent: NodeId, keep: &mut Keep, field: SearchField<'_>) {
    let SearchField { tokens, input, placeholder, font_size, pad, gap, width, height, focus, on_clear } = field;
    let clear_size = font_size + gap;
    let has_text = {
        let text = input.text();
        keep.derive(move || text.with(|t| !t.is_empty()))
    };
    let style = TextInputStyle {
        font_size: Signal::new(font_size),
        font_family: tokens.font_family.clone(),
        color: tokens.text.clone(),
        caret_color: tokens.accent.clone(),
        selection_color: keep.alpha(&tokens.accent, 0.35),
        placeholder,
        placeholder_color: tokens.muted.clone(),
        ..TextInputStyle::default()
    };
    let field_height = (font_size * 1.7).round();
    let inner = {
        let width = width.clone();
        keep.derive(move || (width.get() - pad * 2.0 - clear_size - gap).max(0.0))
    };
    let nodes = scene.text_input(
        parent,
        Node::item().x(pad).y(((height - field_height) / 2.0).round()).width(&inner).height(field_height),
        input,
        style,
    );
    scene.focus_when(nodes.field, focus);

    let clear_area = {
        let input = input.clone();
        MouseArea::new().on_click(move |_| {
            input.clear();
            on_clear();
        })
    };
    let hovered = clear_area.hovered();
    let ink = {
        let (text, muted) = (tokens.text.clone(), tokens.muted.clone());
        let target = keep.derive(move || if hovered.get() { text.get() } else { muted.get() });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let clear_x = keep.derive(move || width.get() - pad - clear_size);
    let clear = scene.add(
        parent,
        Node::item()
            .x(&clear_x)
            .y(((height - clear_size) / 2.0).round())
            .size(clear_size, clear_size)
            .visible(&has_text)
            .cursor(CursorShape::Pointer)
            .mouse(clear_area),
    );
    let glyph = Signal::new(CLEAR_GLYPH.to_owned());
    scene.add(
        clear,
        Node::text(&glyph).font_family(&tokens.icon_font).font_size(font_size).color(&ink).anchors(Anchors::center()),
    );
}
