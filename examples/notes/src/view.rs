use std::rc::Rc;

use acanthus_anim::Keep;
use acanthus_core::{Color, Signal};
use acanthus_runtime::I18n;
use acanthus_scene::{Anchors, CursorShape, ListViewConfig, MouseArea, MouseButton, Node, NodeId, Scene};
use acanthus_widgets::component::Prop;
use acanthus_widgets::standard_registry;

use crate::controller::Controller;
use crate::model::{GAP, HEIGHT, LIST_WIDTH, ROW, WIDTH, title};

const BASE: u32 = 0x1e1e2e;
const SURFACE: u32 = 0x313244;
const BORDER: u32 = 0x585b70;
const TEXT: u32 = 0xcdd6f4;
const MUTED: u32 = 0xa6adc8;
const ACCENT: u32 = 0x89b4fa;

fn role(name: &str) -> Option<Signal<Color>> {
    let hex = match name {
        "primary" => ACCENT,
        "surface0" => SURFACE,
        "surface2" => BORDER,
        "base" => BASE,
        "text" => TEXT,
        "subtext0" => MUTED,
        _ => return None,
    };
    Some(Signal::new(Color::hex(hex)))
}

fn button(
    scene: &mut Scene,
    parent: NodeId,
    keep: &mut Keep,
    label: Signal<String>,
    x: f32,
    click: impl Fn() + 'static,
) {
    let area = MouseArea::new().on_click(move |b| {
        if b == MouseButton::Left {
            click();
        }
    });
    let hovered = area.hovered();
    let fill = keep.derive(move || Color::hex(if hovered.get() { BORDER } else { SURFACE }));
    let width = (LIST_WIDTH - GAP) / 2.0;
    let body = scene.add(
        parent,
        Node::rect().color(&fill).radius(8.0).x(x).size(width, ROW - 4.0).mouse(area).cursor(CursorShape::Pointer),
    );
    scene.add(body, Node::text(label).color(Color::hex(TEXT)).font_size(13.0).anchors(Anchors::center()));
}

pub fn build(scene: &mut Scene, controller: &Controller, i18n: &I18n) -> Keep {
    let mut keep = Keep::default();
    let text = |keep: &mut Keep, key: &'static str| {
        let i18n = i18n.clone();
        keep.derive(move || i18n.t(key))
    };
    let root = scene.root();
    let window = scene.add(root, Node::rect().color(Color::hex(BASE)).anchors(Anchors::fill(0.0)));
    let left = scene.add(window, Node::item().x(GAP).y(GAP).size(LIST_WIDTH, HEIGHT as f32 - GAP * 2.0));
    let (new_label, delete_label) = (text(&mut keep, "notes.new"), text(&mut keep, "notes.delete"));
    let add = controller.clone();
    button(scene, left, &mut keep, new_label, 0.0, move || add.add());
    let remove = controller.clone();
    button(scene, left, &mut keep, delete_label, (LIST_WIDTH + GAP) / 2.0, move || remove.delete());

    let untitled = text(&mut keep, "notes.untitled");
    let selected = controller.selected.clone();
    scene.list_view(
        left,
        Node::item().y(ROW + GAP / 2.0).size(LIST_WIDTH, HEIGHT as f32 - GAP * 2.0 - ROW - GAP / 2.0).clip(true),
        ListViewConfig::default().spacing(4.0),
        &controller.notes,
        |note| note.id,
        move |scene, parent, item| {
            let id = item.value.with_untracked(|note| note.id);
            let mut row_keep = Keep::default();
            let fill = {
                let selected = selected.clone();
                row_keep.derive(move || Color::hex(if selected.get() == Some(id) { BORDER } else { SURFACE }))
            };
            let label = {
                let (value, untitled) = (item.value.clone(), untitled.clone());
                row_keep
                    .derive(move || value.with(|note| title(&note.text).map(str::to_owned)).unwrap_or(untitled.get()))
            };
            let pick = selected.clone();
            let area = MouseArea::new().on_click(move |b| {
                if b == MouseButton::Left {
                    pick.set(Some(id));
                }
            });
            let row = scene.add(
                parent,
                Node::rect().color(&fill).radius(8.0).size(LIST_WIDTH, ROW).mouse(area).cursor(CursorShape::Pointer),
            );
            scene.add(
                row,
                Node::text(label)
                    .color(Color::hex(TEXT))
                    .font_size(13.0)
                    .anchors(Anchors::default().left(10.0).v_center()),
            );
            if let Some(node) = scene.get_mut(row) {
                node.keep(row_keep);
            }
            row
        },
    );

    let registry = standard_registry().with_colors(role);
    let right_x = LIST_WIDTH + GAP * 2.0;
    let right_width = WIDTH as f32 - right_x - GAP;
    let right = scene.add(window, Node::item().x(right_x).y(GAP).size(right_width, HEIGHT as f32 - GAP * 2.0));
    let has_note = {
        let controller = controller.clone();
        keep.derive(move || controller.current().is_some())
    };
    let pinned = {
        let controller = controller.clone();
        keep.derive(move || controller.current().is_some_and(|note| note.pinned))
    };
    let set = controller.clone();
    let toggle: Rc<dyn Fn(bool)> = Rc::new(move |on| set.set_pinned(on));
    let label = text(&mut keep, "notes.pinned");
    registry
        .build(
            "Checkbox",
            scene,
            right,
            vec![
                ("label".into(), Prop::Text(label)),
                ("checked".into(), Prop::Bool(pinned)),
                ("enabled".into(), Prop::Bool(has_note.clone())),
                ("on_toggle".into(), Prop::BoolEvent(toggle)),
            ],
        )
        .expect("checkbox props");
    let placeholder = text(&mut keep, "notes.placeholder");
    let editor = scene.add(right, Node::item().y(ROW));
    registry
        .build(
            "TextArea",
            scene,
            editor,
            vec![
                ("text".into(), Prop::Text(controller.text.clone())),
                ("placeholder".into(), Prop::Text(placeholder)),
                ("width".into(), Prop::number(right_width)),
                ("height".into(), Prop::number(HEIGHT as f32 - GAP * 2.0 - ROW)),
                ("focus".into(), Prop::Bool(has_note)),
            ],
        )
        .expect("text area props");
    keep
}
