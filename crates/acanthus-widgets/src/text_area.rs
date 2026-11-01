//! Multi-line text area: a bordered card around a wrapping, scrolling text editor with a placeholder.

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Effect, Signal};
use acanthus_scene::{Node, NodeId, Scene, TextInput, TextInputStyle};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::scrollbar::scroll_handle;
use crate::timing::FAST;

const SCROLL_WIDTH: f32 = 3.0;

/// Colours and font a text area is drawn with.
#[derive(Clone)]
pub struct TextAreaTokens {
    /// Font family of the typed text.
    pub font_family: Signal<String>,
    /// Card fill.
    pub fill: Signal<Color>,
    /// Typed text colour.
    pub text: Signal<Color>,
    /// Placeholder and idle border colour.
    pub muted: Signal<Color>,
    /// Caret, selection and focused border colour.
    pub accent: Signal<Color>,
}

/// Description of a text area.
pub struct TextArea<'a> {
    /// Colours and font.
    pub tokens: &'a TextAreaTokens,
    /// Text input that holds the text; the caller reads it.
    pub input: &'a TextInput,
    /// Hint shown while the area is empty.
    pub placeholder: Signal<String>,
    /// Font size of the text.
    pub font_size: f32,
    /// Padding between the card edge and the text.
    pub pad: f32,
    /// Corner radius of the card.
    pub radius: Signal<f32>,
    /// Width of the card.
    pub width: Signal<f32>,
    /// Height of the card; longer text scrolls to keep the caret in view.
    pub height: Signal<f32>,
    /// Gives the area keyboard focus while true.
    pub focus: Signal<bool>,
}

/// Builds the card and its editor under `parent` and returns the card; the derived signals live in `keep`.
pub fn text_area(scene: &mut Scene, parent: NodeId, keep: &mut Keep, area: TextArea<'_>) -> NodeId {
    let TextArea { tokens, input, placeholder, font_size, pad, radius, width, height, focus } = area;
    let inner_width = {
        let width = width.clone();
        keep.derive(move || (width.get() - pad * 2.0).max(0.0))
    };
    let inner_height = {
        let height = height.clone();
        keep.derive(move || (height.get() - pad * 2.0).max(0.0))
    };
    let editor = Node::item().x(pad).y(pad).width(&inner_width).height(&inner_height);
    let focused = editor.active_focus();
    let border = {
        let (focused, accent, muted) = (focused, tokens.accent.clone(), tokens.muted.clone());
        let target = keep.derive(move || if focused.get() { accent.get() } else { muted.get() });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let card = scene.add(
        parent,
        Node::rect().color(&tokens.fill).radius(&radius).border(1.0, &border).width(&width).height(&height),
    );
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
    let nodes = scene.text_edit(card, editor, input, style);
    scene.focus_when(nodes.field, focus);
    let handle = {
        let (content, scroll, view) = (input.content_height(), input.scroll_y(), inner_height.clone());
        keep.derive(move || scroll_handle(content.get(), view.get(), scroll.get()))
    };
    let visible = {
        let handle = handle.clone();
        keep.derive(move || handle.get().is_some())
    };
    let top = {
        let handle = handle.clone();
        keep.derive(move || pad + handle.get().map_or(0.0, |h| h.0))
    };
    let length = keep.derive(move || handle.get().map_or(0.0, |h| h.1));
    let x = keep.derive(move || width.get() - pad * 0.5 - SCROLL_WIDTH);
    scene.add(
        card,
        Node::rect()
            .color(keep.alpha(&tokens.muted, 0.6))
            .radius(SCROLL_WIDTH / 2.0)
            .x(&x)
            .y(&top)
            .width(SCROLL_WIDTH)
            .height(&length)
            .visible(&visible),
    );
    card
}

/// The text area as a [`Component`] named `TextArea`: it owns its text input and mirrors the
/// typed text into the `text` prop, and a change of the prop is written back into the input.
pub struct TextAreaComponent;

impl Component for TextAreaComponent {
    fn name(&self) -> &'static str {
        "TextArea"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::text("text", || Prop::text("")),
            PropSpec::text("placeholder", || Prop::text("")),
            PropSpec::text("font_family", || Prop::text("")),
            PropSpec::number("font_size", || Prop::number(14.0)),
            PropSpec::scaled("pad", "spacing.md", || Prop::number(10.0)),
            PropSpec::scaled("radius", "radii.md", || Prop::number(8.0)),
            PropSpec::number("width", || Prop::number(240.0)),
            PropSpec::number("height", || Prop::number(120.0)),
            PropSpec::bool("focus", || Prop::bool(false)),
            PropSpec::themed("fill", "surface0"),
            PropSpec::themed("text_color", "text"),
            PropSpec::themed("muted", "subtext0"),
            PropSpec::themed("accent", "primary"),
        ];
        PROPS
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let tokens = TextAreaTokens {
            font_family: props.text("font_family"),
            fill: props.color("fill"),
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
        let card = text_area(
            scene,
            parent,
            &mut keep,
            TextArea {
                tokens: &tokens,
                input: &input,
                placeholder: props.text("placeholder"),
                font_size: props.number("font_size").get_untracked(),
                pad: props.number("pad").get_untracked(),
                radius: props.number("radius"),
                width: props.number("width"),
                height: props.number("height"),
                focus: props.bool("focus"),
            },
        );
        if let Some(node) = scene.get_mut(card) {
            node.keep(keep);
        }
        card
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Registry;
    use acanthus_scene::{Key, KeyEvent};

    fn given(text: &Signal<String>, focus: &Signal<bool>) -> Vec<(String, Prop)> {
        let mut given: Vec<(String, Prop)> = ["fill", "text_color", "muted", "accent"]
            .map(|name| (name.to_owned(), Prop::color(Color::hex(0x778899))))
            .to_vec();
        given.push(("text".to_owned(), Prop::Text(text.clone())));
        given.push(("focus".to_owned(), Prop::Bool(focus.clone())));
        given.push(("width".to_owned(), Prop::number(120.0)));
        given.push(("height".to_owned(), Prop::number(60.0)));
        given
    }

    #[test]
    fn typing_with_line_breaks_reaches_the_text_prop_and_back() {
        let mut registry = Registry::new();
        registry.register(TextAreaComponent);
        let (text, focus) = (Signal::new("ab".to_owned()), Signal::new(true));
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        registry.build("TextArea", &mut scene, root, given(&text, &focus)).unwrap();
        scene.frame();
        scene.frame();
        assert!(scene.key_event(&KeyEvent::new(Key::Enter)));
        assert!(scene.key_event(&KeyEvent::new(Key::Char('c'))));
        assert_eq!(text.get_untracked(), "ab\nc");
        text.set("x".to_owned());
        scene.frame();
        assert!(scene.key_event(&KeyEvent::new(Key::Char('y'))));
        assert_eq!(text.get_untracked(), "xy");
    }

    #[test]
    fn long_text_shows_a_scroll_indicator_at_the_end() {
        let mut registry = Registry::new();
        registry.register(TextAreaComponent);
        let (text, focus) = (Signal::new(String::new()), Signal::new(true));
        let mut scene = Scene::new(acanthus_scene::Fonts::new());
        let root = scene.root();
        let card = registry.build("TextArea", &mut scene, root, given(&text, &focus)).unwrap();
        scene.frame();
        let indicator = *scene.get(card).unwrap().children().last().unwrap();
        assert!(!scene.get(indicator).unwrap().visible.get_untracked());
        for _ in 0..12 {
            scene.key_event(&KeyEvent::new(Key::Enter));
            scene.frame();
        }
        scene.frame();
        assert!(scene.get(indicator).unwrap().visible.get_untracked());
        let at = scene.geometry(indicator).unwrap();
        assert!(at.y > 10.0 && at.y + at.h <= 60.0);
    }
}
