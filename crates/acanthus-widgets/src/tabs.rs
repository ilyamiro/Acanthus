//! Tabs: a row of labels with an underline that slides to the selected one, over a stack of panels of which only the selected one shows.

use std::rc::Rc;

use acanthus_anim::{Easing, Keep};
use acanthus_core::{Color, Signal};
use acanthus_scene::{Anchors, CursorShape, MouseArea, MouseButton, Node, NodeId, Scene};

use crate::choice::step_selection;
use crate::component::{Component, Prop, PropSpec, Props};
use crate::timing::{BASE, FAST};

/// Thickness of the underline and of the rule under the tab row.
const RULE: f32 = 2.0;

/// Description of a set of tabs.
pub struct TabsSpec {
    /// One label per tab.
    pub labels: Vec<Signal<String>>,
    /// Index of the selected tab; the caller updates it from `on_select`.
    pub current: Signal<usize>,
    /// Colour of the selected label and the underline.
    pub accent: Signal<Color>,
    /// Colour of the other labels.
    pub muted: Signal<Color>,
    /// Colour of the rule under the whole row.
    pub rule: Signal<Color>,
    /// Label font family.
    pub font_family: Signal<String>,
    /// Label font size.
    pub font_size: f32,
    /// Horizontal padding around each label.
    pub pad: f32,
    /// Height of the tab row.
    pub bar_height: f32,
    /// Width of the row and of the panel area.
    pub width: Signal<f32>,
    /// Height of the panel area.
    pub panel_height: Signal<f32>,
    /// Called with the index picked by a click, Left or Right on a focused tab.
    pub on_select: Rc<dyn Fn(usize)>,
}

/// The nodes of built tabs.
pub struct Tabs {
    /// Column holding the tab row followed by the panel area.
    pub root: NodeId,
    /// One panel per label, shown while its tab is selected; the caller adds the content.
    pub panels: Vec<NodeId>,
}

/// Left edge and width of tab `index` given the width of every label and the padding on both sides.
pub fn tab_span(label_widths: &[f32], pad: f32, index: usize) -> (f32, f32) {
    let width = |w: &f32| w + pad * 2.0;
    let x = label_widths.iter().take(index).map(width).sum();
    (x, label_widths.get(index).map_or(0.0, width))
}

/// Builds the tabs under `parent`.
pub fn tabs(scene: &mut Scene, parent: NodeId, spec: TabsSpec) -> Tabs {
    let TabsSpec {
        labels,
        current,
        accent,
        muted,
        rule,
        font_family,
        font_size,
        pad,
        bar_height,
        width,
        panel_height,
        on_select,
    } = spec;
    let mut keep = Keep::default();
    let count = labels.len();
    let root = scene.add(parent, Node::column(0.0));
    let bar = scene.add(root, Node::item().width(&width).height(bar_height));
    scene.add(bar, Node::rect().color(&rule).y(bar_height - RULE).width(&width).height(RULE));
    let row = scene.add(bar, Node::row(0.0).height(bar_height));
    let mut measured = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        let color = {
            let (current, accent, muted) = (current.clone(), accent.clone(), muted.clone());
            let target = keep.derive(move || if current.get() == index { accent.get() } else { muted.get() });
            keep.ease(&target, FAST, Easing::Linear)
        };
        let text = Node::text(label).font_family(&font_family).font_size(font_size).color(&color);
        measured.push(text.implicit_size());
        let cell_width = {
            let size = text.implicit_size();
            keep.derive(move || size.get().x + pad * 2.0)
        };
        let activate: Rc<dyn Fn()> = {
            let on_select = on_select.clone();
            Rc::new(move || on_select(index))
        };
        let area = {
            let activate = activate.clone();
            MouseArea::new().on_click(move |button| {
                if button == MouseButton::Left {
                    activate();
                }
            })
        };
        let cell = Node::item()
            .width(&cell_width)
            .height(bar_height)
            .mouse(area)
            .cursor(CursorShape::Pointer)
            .focusable(true)
            .on_key({
                let (current, on_select) = (current.clone(), on_select.clone());
                move |event| match event.key {
                    acanthus_scene::Key::Char(' ') | acanthus_scene::Key::Enter => {
                        activate();
                        true
                    }
                    key => step_selection(key, current.get_untracked(), count).is_some_and(|next| {
                        on_select(next);
                        true
                    }),
                }
            });
        let cell = scene.add(row, cell);
        scene.add(cell, text.anchors(Anchors::center()));
    }
    let span = {
        let (current, measured) = (current.clone(), measured);
        keep.derive(move || {
            let widths: Vec<f32> = measured.iter().map(|size| size.get().x).collect();
            tab_span(&widths, pad, current.get())
        })
    };
    let underline_x = {
        let span = span.clone();
        let target = keep.derive(move || span.get().0);
        keep.ease(&target, BASE, Easing::OutCubic)
    };
    let underline_w = {
        let target = keep.derive(move || span.get().1);
        keep.ease(&target, BASE, Easing::OutCubic)
    };
    scene.add(bar, Node::rect().color(&accent).x(&underline_x).y(bar_height - RULE).width(&underline_w).height(RULE));
    let area = scene.add(root, Node::item().width(&width).height(&panel_height));
    let panels = (0..count)
        .map(|index| {
            let shown = {
                let current = current.clone();
                keep.derive(move || current.get() == index)
            };
            scene.add(area, Node::item().anchors(Anchors::fill(0.0)).visible(&shown))
        })
        .collect();
    if let Some(node) = scene.get_mut(root) {
        node.keep(keep);
    }
    Tabs { root, panels }
}

/// The tabs as a [`Component`] named `Tabs`; the panel of tab `i` is child `i` of the root's second child.
pub struct TabsComponent;

impl Component for TabsComponent {
    fn name(&self) -> &'static str {
        "Tabs"
    }

    fn props(&self) -> &'static [PropSpec] {
        const PROPS: &[PropSpec] = &[
            PropSpec::text_slots("labels"),
            PropSpec::index("current", || Prop::Index(Signal::new(0))),
            PropSpec::text("font_family", || Prop::text("")),
            PropSpec::number("font_size", || Prop::number(14.0)),
            PropSpec::scaled("pad", "spacing.md", || Prop::number(12.0)),
            PropSpec::number("bar_height", || Prop::number(36.0)),
            PropSpec::number("width", || Prop::number(300.0)),
            PropSpec::number("panel_height", || Prop::number(200.0)),
            PropSpec::themed("accent", "primary"),
            PropSpec::themed("muted", "subtext0"),
            PropSpec::themed("rule", "surface1"),
            PropSpec::index_event("on_select"),
        ];
        PROPS
    }

    fn slots(&self, scene: &Scene, root: NodeId) -> Vec<NodeId> {
        let second = scene.get(root).map(|n| n.children()[1]);
        second.and_then(|s| scene.get(s)).map(|n| n.children().to_vec()).unwrap_or_default()
    }

    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
        tabs(
            scene,
            parent,
            TabsSpec {
                labels: props.text_slots("labels"),
                current: props.index("current"),
                accent: props.color("accent"),
                muted: props.color("muted"),
                rule: props.color("rule"),
                font_family: props.text("font_family"),
                font_size: props.number("font_size").get_untracked(),
                pad: props.number("pad").get_untracked(),
                bar_height: props.number("bar_height").get_untracked(),
                width: props.number("width"),
                panel_height: props.number("panel_height"),
                on_select: props.index_event("on_select"),
            },
        )
        .root
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use acanthus_core::Vec2;
    use acanthus_scene::Fonts;

    use super::*;
    use crate::component::Registry;

    #[test]
    fn spans_stack_left_to_right_with_padding() {
        let widths = [30.0, 50.0];
        assert_eq!(tab_span(&widths, 10.0, 0), (0.0, 50.0));
        assert_eq!(tab_span(&widths, 10.0, 1), (50.0, 70.0));
        assert_eq!(tab_span(&widths, 10.0, 5).1, 0.0);
    }

    #[test]
    fn a_click_selects_and_only_the_current_panel_shows() {
        let mut registry = Registry::new();
        registry.register(TabsComponent);
        let picked = Rc::new(RefCell::new(Vec::new()));
        let sink = picked.clone();
        let current = Signal::new(0_usize);
        let mut given: Vec<(String, Prop)> =
            ["accent", "muted", "rule"].map(|name| (name.to_owned(), Prop::color(Color::hex(0x446688)))).to_vec();
        let labels = ["General", "Audio"].map(|t| Signal::new(t.to_owned()));
        given.push(("labels".to_owned(), Prop::TextSlots(labels.to_vec())));
        given.push(("font_family".to_owned(), Prop::text("Adwaita Mono")));
        given.push(("current".to_owned(), Prop::Index(current.clone())));
        given.push(("on_select".to_owned(), Prop::IndexEvent(Rc::new(move |i| sink.borrow_mut().push(i)))));
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let tabs = registry.build("Tabs", &mut scene, root, given).unwrap();
        scene.set_viewport(Vec2::new(400.0, 300.0), 1.0);
        scene.frame();
        let children = scene.get(tabs).unwrap().children().to_vec();
        let bar = children[0];
        let row = scene.get(bar).unwrap().children()[1];
        let cells = scene.get(row).unwrap().children().to_vec();
        assert_eq!(cells.len(), 2);
        let second = scene.geometry(cells[1]).unwrap();
        assert!(second.x > 0.0);
        scene.pointer_motion_at(Vec2::new(second.x + 4.0, second.y + 4.0), 0.0);
        scene.pointer_button_at(MouseButton::Left, true, 0.0);
        scene.pointer_button_at(MouseButton::Left, false, 0.0);
        assert_eq!(*picked.borrow(), vec![1]);
        current.set(1);
        scene.frame();
        let panels = scene.get(children[1]).unwrap().children().to_vec();
        assert_eq!(panels.len(), 2);
        assert!(!scene.get(panels[0]).unwrap().visible.get_untracked());
        assert!(scene.get(panels[1]).unwrap().visible.get_untracked());
    }
}
