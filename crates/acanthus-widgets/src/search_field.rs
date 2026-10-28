//! Single-line search field with a placeholder and a clear button that appears once text is typed.

use std::rc::Rc;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Effect, Signal};
use acanthus_scene::{Anchors, CursorShape, MouseArea, Node, NodeId, Scene, TextInput, TextInputStyle};

use crate::component::{Component, Kind, Prop, PropSpec, Props};
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

/// The search field as a [`Component`] named `SearchField`: it owns its text input and mirrors the
/// typed text into the `text` prop, and a change of the prop is written back into the input.
pub struct SearchFieldComponent;

impl Component for SearchFieldComponent {
    fn name(&self) -> &'static str {
        "SearchField"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::text("text", || Prop::text("")),
            PropSpec::text("placeholder", || Prop::text("")),
            PropSpec::text("font_family", || Prop::text("")),
            PropSpec::text("icon_font", || Prop::text("")),
            PropSpec::number("font_size", || Prop::number(14.0)),
            PropSpec::number("pad", || Prop::number(10.0)),
            PropSpec::number("gap", || Prop::number(6.0)),
            PropSpec::number("width", || Prop::number(200.0)),
            PropSpec::number("height", || Prop::number(32.0)),
            PropSpec::bool("focus", || Prop::bool(false)),
            PropSpec::required("text_color", Kind::Color),
            PropSpec::required("muted", Kind::Color),
            PropSpec::required("accent", Kind::Color),
            PropSpec::event("on_clear"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let tokens = SearchTokens {
            font_family: props.text("font_family"),
            icon_font: props.text("icon_font"),
            text: props.color("text_color"),
            muted: props.color("muted"),
            accent: props.color("accent"),
        };
        let text = props.text("text");
        let input = TextInput::new(text.get_untracked());
        let mut keep = Keep::default();
        keep.add(Effect::new({
            let (text, typed) = (text.clone(), input.text());
            move || {
                let value = typed.get();
                if text.get_untracked() != value {
                    text.set(value);
                }
            }
        }));
        keep.add(Effect::new({
            let (text, input) = (text, input.clone());
            move || {
                let value = text.get();
                if input.text().get_untracked() != value {
                    input.set_text(value);
                }
            }
        }));
        let height = props.number("height").get_untracked();
        let root = scene.add(parent, Node::item().width(props.number("width")).height(height));
        search_field(
            scene,
            root,
            &mut keep,
            SearchField {
                tokens: &tokens,
                input: &input,
                placeholder: props.text("placeholder"),
                font_size: props.number("font_size").get_untracked(),
                pad: props.number("pad").get_untracked(),
                gap: props.number("gap").get_untracked(),
                width: props.number("width"),
                height,
                focus: props.bool("focus"),
                on_clear: props.event("on_clear"),
            },
        );
        if let Some(node) = scene.get_mut(root) {
            node.keep(keep);
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Registry;

    #[test]
    fn the_registry_builds_a_search_field_and_mirrors_the_typed_text() {
        let mut registry = Registry::new();
        registry.register(SearchFieldComponent);
        let text = Signal::new("abc".to_owned());
        let mut given: Vec<(String, Prop)> =
            ["text_color", "muted", "accent"].map(|name| (name.to_owned(), Prop::color(Color::hex(0x778899)))).to_vec();
        given.push(("text".to_owned(), Prop::Text(text.clone())));
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let node = registry.build("SearchField", &mut scene, root, given).unwrap();
        assert!(scene.get(node).is_some());
        text.set("abcd".to_owned());
        assert_eq!(text.get_untracked(), "abcd");
    }
}
