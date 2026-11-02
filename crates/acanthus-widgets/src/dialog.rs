//! Dialog: a modal card with a title, a message and a row of buttons over a scrim that dims and blocks what is behind.

use std::rc::Rc;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Signal};
use acanthus_scene::{Align, Anchors, CursorShape, Key, Layout, MouseArea, MouseButton, Node, NodeId, Scene, WrapMode};

use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::{BASE, FAST};

/// Description of a dialog.
pub struct DialogSpec {
    /// Whether the dialog is showing; the caller sets it false from `on_choose` or `on_dismiss`.
    pub open: Signal<bool>,
    /// Heading of the card.
    pub title: Signal<String>,
    /// Message under the heading.
    pub body: Signal<String>,
    /// One label per button, left to right.
    pub buttons: Vec<Signal<String>>,
    /// Index of the highlighted button, which Enter picks.
    pub primary: usize,
    /// Width of the card.
    pub width: f32,
    /// Padding inside the card.
    pub pad: f32,
    /// Space between the card's rows and between buttons.
    pub gap: f32,
    /// Corner radius of the card and the buttons.
    pub radius: f32,
    /// Colour dimming what is behind.
    pub scrim: Signal<Color>,
    /// Card fill.
    pub fill: Signal<Color>,
    /// Card and button outline.
    pub border: Signal<Color>,
    /// Fill of the primary button.
    pub accent: Signal<Color>,
    /// Label colour on the primary button.
    pub accent_text: Signal<Color>,
    /// Fill of the other buttons.
    pub button: Signal<Color>,
    /// Title and button label colour.
    pub text: Signal<Color>,
    /// Message colour.
    pub muted: Signal<Color>,
    /// Font family.
    pub font_family: Signal<String>,
    /// Font size of the message and the buttons; the title is a quarter larger.
    pub font_size: f32,
    /// Called with the index of the button picked by a click or Enter.
    pub on_choose: Rc<dyn Fn(usize)>,
    /// Called when the scrim is clicked or Escape is pressed.
    pub on_dismiss: Rc<dyn Fn()>,
}

fn dialog_button(
    scene: &mut Scene,
    parent: NodeId,
    label: &Signal<String>,
    primary: bool,
    spec: &DialogSpec,
    choose: Rc<dyn Fn()>,
) {
    let mut keep = Keep::default();
    let area = MouseArea::new().on_click(move |button| {
        if button == MouseButton::Left {
            choose();
        }
    });
    let hovered = area.hovered();
    let (base, text) = if primary { (&spec.accent, &spec.accent_text) } else { (&spec.button, &spec.text) };
    let fill = {
        let (base, hovered) = (base.clone(), hovered);
        let target = keep.derive(move || {
            let color = base.get();
            if hovered.get() { acanthus_core::lighten(color, 0.1) } else { color }
        });
        keep.ease(&target, FAST, Easing::Linear)
    };
    let label_node = Node::text(label).font_family(&spec.font_family).font_size(spec.font_size).color(text);
    let measured = label_node.implicit_size();
    let (pad, height) = (spec.gap * 1.5, spec.font_size * 2.2);
    let width = keep.derive(move || measured.get().x + pad * 2.0);
    let button = scene.add(
        parent,
        Node::rect()
            .color(&fill)
            .radius(spec.radius)
            .border(1.0, &spec.border)
            .width(&width)
            .height(height)
            .mouse(area)
            .cursor(CursorShape::Pointer),
    );
    scene.add(button, label_node.anchors(Anchors::center()));
    if let Some(node) = scene.get_mut(button) {
        node.keep(keep);
    }
}

/// Builds the scrim and card filling `parent` and returns the scrim, which takes the focus while the dialog is open.
pub fn dialog(scene: &mut Scene, parent: NodeId, spec: DialogSpec) -> NodeId {
    let mut keep = Keep::default();
    let presence = {
        let open = spec.open.clone();
        let target = keep.derive(move || if open.get() { 1.0_f32 } else { 0.0 });
        keep.ease(&target, BASE, Easing::OutCubic)
    };
    let visible = {
        let presence = presence.clone();
        keep.derive(move || presence.get() > 0.0)
    };
    let card_scale = {
        let presence = presence.clone();
        keep.derive(move || 0.94 + 0.06 * presence.get())
    };
    let (on_dismiss, on_choose, primary, count) =
        (spec.on_dismiss.clone(), spec.on_choose.clone(), spec.primary, spec.buttons.len());
    let click_dismiss = spec.on_dismiss.clone();
    let swallow = MouseArea::new().on_click(move |button| {
        if button == MouseButton::Left {
            click_dismiss();
        }
    });
    let open = spec.open.clone();
    let scrim = scene.add(
        parent,
        Node::rect()
            .color(&spec.scrim)
            .opacity(&presence)
            .visible(&visible)
            .anchors(Anchors::fill(0.0))
            .mouse(swallow)
            .focusable(true)
            .on_key(move |event| match event.key {
                Key::Escape => {
                    on_dismiss();
                    true
                }
                Key::Enter if primary < count => {
                    on_choose(primary);
                    true
                }
                _ => false,
            }),
    );
    scene.focus_when(scrim, open);
    let card = scene.add(
        scrim,
        Node::rect()
            .color(&spec.fill)
            .radius(spec.radius * 1.5)
            .border(1.0, &spec.border)
            .width(spec.width)
            .layout(Layout::column(spec.gap).padding(spec.pad, spec.pad))
            .scale(&card_scale)
            .anchors(Anchors::center())
            .mouse(MouseArea::new()),
    );
    let inner = spec.width - spec.pad * 2.0;
    scene.add(
        card,
        Node::text(&spec.title)
            .font_family(&spec.font_family)
            .font_size(spec.font_size * 1.25)
            .font_weight(600)
            .color(&spec.text)
            .width(inner)
            .align(Align::Start),
    );
    scene.add(
        card,
        Node::text(&spec.body)
            .font_family(&spec.font_family)
            .font_size(spec.font_size)
            .color(&spec.muted)
            .wrap_mode(WrapMode::Word)
            .width(inner)
            .align(Align::Start),
    );
    scene.add(card, Node::column(spec.gap).width(inner));
    let row = scene.add(card, Node::row(spec.gap).align(Align::End));
    for (index, label) in spec.buttons.iter().enumerate() {
        let choose = spec.on_choose.clone();
        dialog_button(scene, row, label, index == spec.primary, &spec, Rc::new(move || choose(index)));
    }
    if let Some(node) = scene.get_mut(scrim) {
        node.keep(keep);
    }
    scrim
}

/// The dialog as a [`Component`] named `Dialog`.
pub struct DialogComponent;

impl Component for DialogComponent {
    fn name(&self) -> &'static str {
        "Dialog"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::bool("open", || Prop::bool(false)),
            PropSpec::text("title", || Prop::text("")),
            PropSpec::text("body", || Prop::text("")),
            PropSpec::text_slots("buttons"),
            PropSpec::index("primary", || Prop::Index(Signal::new(0))),
            PropSpec::number("width", || Prop::number(360.0)),
            PropSpec::scaled("pad", "spacing.lg", || Prop::number(20.0)),
            PropSpec::scaled("gap", "spacing.md", || Prop::number(12.0)),
            PropSpec::scaled("radius", "radii.md", || Prop::number(8.0)),
            PropSpec::text("font_family", || Prop::text("")),
            PropSpec::number("font_size", || Prop::number(14.0)),
            PropSpec::themed("scrim", "crust"),
            PropSpec::themed("fill", "base"),
            PropSpec::themed("border", "surface2"),
            PropSpec::themed("accent", "primary"),
            PropSpec::themed("accent_text", "base"),
            PropSpec::themed("button", "surface0"),
            PropSpec::themed("text", "text"),
            PropSpec::themed("muted", "subtext0"),
            PropSpec::event("on_dismiss"),
            PropSpec::index_event("on_choose"),
        ];
        PROPS
    }

    fn slots(&self, scene: &Scene, root: NodeId) -> Vec<NodeId> {
        let card = scene.get(root).and_then(|n| n.children().first().copied());
        card.and_then(|c| scene.get(c)).and_then(|n| n.children().get(2).copied()).into_iter().collect()
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        let get = |name: &str| props.number(name).get_untracked();
        dialog(
            scene,
            parent,
            DialogSpec {
                open: props.bool("open"),
                title: props.text("title"),
                body: props.text("body"),
                buttons: props.text_slots("buttons"),
                primary: props.index("primary").get_untracked(),
                width: get("width"),
                pad: get("pad"),
                gap: get("gap"),
                radius: get("radius"),
                scrim: props.color("scrim"),
                fill: props.color("fill"),
                border: props.color("border"),
                accent: props.color("accent"),
                accent_text: props.color("accent_text"),
                button: props.color("button"),
                text: props.color("text"),
                muted: props.color("muted"),
                font_family: props.text("font_family"),
                font_size: get("font_size"),
                on_choose: props.index_event("on_choose"),
                on_dismiss: props.event("on_dismiss"),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use acanthus_core::Vec2;
    use acanthus_scene::{Fonts, KeyEvent};

    use super::*;
    use crate::component::Registry;

    #[test]
    fn the_dialog_fades_in_picks_buttons_and_dismisses_by_scrim_or_escape() {
        let mut registry = Registry::new();
        registry.register(DialogComponent);
        let open = Signal::new(false);
        let chosen = Rc::new(RefCell::new(Vec::new()));
        let dismissed = Rc::new(Cell::new(0));
        let mut given: Vec<(String, Prop)> =
            ["scrim", "fill", "border", "accent", "accent_text", "button", "text", "muted"]
                .map(|name| (name.to_owned(), Prop::color(Color::hex(0x445566))))
                .to_vec();
        given.push(("open".to_owned(), Prop::Bool(open.clone())));
        given.push(("title".to_owned(), Prop::text("Delete file")));
        given.push(("body".to_owned(), Prop::text("This cannot be undone and the file will be gone for good.")));
        given.push(("font_family".to_owned(), Prop::text("Adwaita Mono")));
        given.push((
            "buttons".to_owned(),
            Prop::TextSlots(["Cancel", "Delete"].map(|t| Signal::new(t.to_owned())).to_vec()),
        ));
        given.push(("primary".to_owned(), Prop::Index(Signal::new(1))));
        let sink = chosen.clone();
        given.push(("on_choose".to_owned(), Prop::IndexEvent(Rc::new(move |i| sink.borrow_mut().push(i)))));
        let count = dismissed.clone();
        given.push(("on_dismiss".to_owned(), Prop::Event(Rc::new(move || count.set(count.get() + 1)))));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let scrim = registry.build("Dialog", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(800.0, 600.0), 1.0);
        scene.frame();
        let shown = |scene: &Scene| scene.get(scrim).unwrap().visible.get_untracked();
        assert!(!shown(&scene));
        open.set(true);
        for _ in 0..30 {
            acanthus_anim::tick(0.016);
            scene.frame();
        }
        assert!(shown(&scene));
        let card = scene.get(scrim).unwrap().children()[0];
        let card_at = scene.geometry(card).unwrap();
        assert_eq!(card_at.w, 360.0);
        assert!(card_at.h > 100.0 && card_at.x > 100.0);
        let row = *scene.get(card).unwrap().children().last().unwrap();
        let buttons = scene.get(row).unwrap().children().to_vec();
        let at = scene.geometry(buttons[0]).unwrap();
        let click = |scene: &mut Scene, at: Vec2| {
            scene.pointer_motion_at(at, 0.0);
            scene.pointer_button_at(MouseButton::Left, true, 0.0);
            scene.pointer_button_at(MouseButton::Left, false, 0.0);
        };
        click(&mut scene, Vec2::new(at.x + 4.0, at.y + 4.0));
        assert_eq!(*chosen.borrow(), vec![0]);
        assert_eq!(dismissed.get(), 0);
        click(&mut scene, Vec2::new(card_at.x + 4.0, card_at.y + 4.0));
        assert_eq!(dismissed.get(), 0);
        click(&mut scene, Vec2::new(2.0, 2.0));
        assert_eq!(dismissed.get(), 1);
        assert!(scene.key_event(&KeyEvent::new(Key::Enter)));
        assert_eq!(*chosen.borrow(), vec![0, 1]);
        assert!(scene.key_event(&KeyEvent::new(Key::Escape)));
        assert_eq!(dismissed.get(), 2);
        open.set(false);
        for _ in 0..30 {
            acanthus_anim::tick(0.016);
            scene.frame();
        }
        assert!(!shown(&scene));
    }

    #[test]
    fn the_content_slot_sits_between_the_message_and_the_buttons() {
        let mut registry = Registry::new();
        registry.register(DialogComponent);
        let mut given: Vec<(String, Prop)> =
            ["scrim", "fill", "border", "accent", "accent_text", "button", "text", "muted"]
                .map(|name| (name.to_owned(), Prop::color(Color::hex(0x445566))))
                .to_vec();
        given.push(("buttons".to_owned(), Prop::TextSlots(vec![Signal::new("Ok".to_owned())])));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let scrim = registry.build("Dialog", &mut scene, root, given).unwrap();
        let slots = DialogComponent.slots(&scene, scrim);
        let card = scene.get(scrim).unwrap().children()[0];
        let kids = scene.get(card).unwrap().children().to_vec();
        assert_eq!(slots, vec![kids[2]]);
        assert_eq!(kids.len(), 4);
    }
}
