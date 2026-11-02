use crate::{Component, Diagnostic, Document, Expr, Functions, Member, Scope, Span, Value};
use acanthus_anim::{Easing, Motion, SpringParams};
use acanthus_core::{Color, Effect, Signal};
use acanthus_scene::{Node, NodeId, Prop as NodeProp, Scene};
use acanthus_widgets::component::{Component as Widget, Kind, Prop, Props, Registry};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// A built tree: the root node, the nodes by id, and the bindings that keep it live.
pub struct Built {
    /// The node of the root component.
    pub root: NodeId,
    /// The node of every component that declared an `id`.
    pub ids: HashMap<String, NodeId>,
    _effects: Vec<Effect>,
}

type IdEntry = (Rc<dyn Widget>, Rc<Props>);

struct Locals {
    ids: RefCell<HashMap<String, IdEntry>>,
    declared: RefCell<HashMap<String, Signal<Value>>>,
    arg: RefCell<Option<Value>>,
    globals: Rc<dyn Scope>,
}

impl Scope for Locals {
    fn roots(&self) -> Vec<String> {
        let mut roots = self.globals.roots();
        roots.extend(self.ids.borrow().keys().cloned());
        roots.extend(self.declared.borrow().keys().cloned());
        roots
    }

    fn get(&self, path: &[String]) -> Option<Value> {
        match path {
            [v] if v == "value" && self.arg.borrow().is_some() => self.arg.borrow().clone(),
            [name] if self.declared.borrow().contains_key(name) => Some(self.declared.borrow()[name].get()),
            [id, prop] if self.ids.borrow().contains_key(id) => {
                let (widget, props) = self.ids.borrow()[id].clone();
                let spec = widget.props().iter().find(|s| s.name == prop)?;
                Some(match spec.kind {
                    Kind::Number => Value::Number(props.number(prop).get().into()),
                    Kind::Text => Value::Text(props.text(prop).get()),
                    Kind::Bool => Value::Bool(props.bool(prop).get()),
                    Kind::Color => Value::Color(props.color(prop).get()),
                    Kind::Index => Value::Number(props.index(prop).get() as f64),
                    _ => return None,
                })
            }
            _ => self.globals.get(path),
        }
    }
}

pub(crate) struct Names(pub(crate) Vec<String>);

impl Scope for Names {
    fn roots(&self) -> Vec<String> {
        self.0.clone()
    }

    fn get(&self, _: &[String]) -> Option<Value> {
        None
    }
}

fn is_event(kind: Kind) -> bool {
    matches!(kind, Kind::Event | Kind::NumberEvent | Kind::BoolEvent | Kind::IndexEvent)
}

fn collect<'a>(
    c: &'a Component,
    ids: &mut HashSet<&'a str>,
    declared: &mut HashSet<&'a str>,
    out: &mut Vec<Diagnostic>,
) {
    for m in &c.members {
        match m {
            Member::Id(n, s) if !ids.insert(n) => out.push(Diagnostic::new(*s, format!("the id `{n}` is used twice"))),
            Member::Property(n, s, _) if !declared.insert(n) => {
                out.push(Diagnostic::new(*s, format!("the property `{n}` is declared twice")))
            }
            Member::Child(child) => collect(child, ids, declared, out),
            _ => {}
        }
    }
}

fn validate_component(
    c: &Component,
    registry: &Registry,
    names: &[String],
    functions: &Functions,
    out: &mut Vec<Diagnostic>,
) {
    let Some(widget) = registry.get(&c.name) else { return };
    let kind_of = |n: &str| widget.props().iter().find(|s| s.name == n).map(|s| s.kind);
    let with_value: Vec<String> = names.iter().cloned().chain(["value".to_string()]).collect();
    for m in &c.members {
        match m {
            Member::Set(n, s, e) => {
                match kind_of(n) {
                    Some(k) if is_event(k) => {
                        out.push(Diagnostic::new(*s, format!("`{n}` is an event; write `on_{n}`")))
                    }
                    Some(Kind::TextSlots | Kind::Node | Kind::Point | Kind::Flickable) => {
                        out.push(Diagnostic::new(*s, format!("`{n}` cannot be set from markup yet")))
                    }
                    _ => {}
                }
                out.extend(e.check(&Names(names.to_vec()), functions));
            }
            Member::Handler(n, s, e) => {
                if kind_of(n).is_some_and(|k| !is_event(k)) {
                    out.push(Diagnostic::new(*s, format!("`{n}` is not an event")));
                }
                let roots = if kind_of(n) == Some(Kind::Event) { names.to_vec() } else { with_value.clone() };
                out.extend(e.check(&Names(roots), functions));
            }
            Member::Property(_, _, e) => out.extend(e.check(&Names(names.to_vec()), functions)),
            Member::Behavior(n, s, body) => {
                if node_prop(n).is_none() {
                    out.push(Diagnostic::unknown(*s, "animatable property", n, NODE_PROPS.iter().map(|p| p.name())));
                }
                if let Err(d) = motion(body, functions) {
                    out.push(d);
                }
            }
            Member::Child(child) => validate_component(child, registry, names, functions, out),
            Member::Id(..) => {}
        }
    }
}

const NODE_PROPS: [NodeProp; 11] = [
    NodeProp::X,
    NodeProp::Y,
    NodeProp::Width,
    NodeProp::Height,
    NodeProp::Opacity,
    NodeProp::Scale,
    NodeProp::Rotation,
    NodeProp::Color,
    NodeProp::Radius,
    NodeProp::BorderWidth,
    NodeProp::BorderColor,
];

fn node_prop(name: &str) -> Option<NodeProp> {
    NODE_PROPS.into_iter().find(|p| p.name().replace('-', "_") == name)
}

const EASINGS: [(&str, Easing); 22] = [
    ("Linear", Easing::Linear),
    ("InQuad", Easing::InQuad),
    ("OutQuad", Easing::OutQuad),
    ("InOutQuad", Easing::InOutQuad),
    ("InCubic", Easing::InCubic),
    ("OutCubic", Easing::OutCubic),
    ("InOutCubic", Easing::InOutCubic),
    ("InQuart", Easing::InQuart),
    ("OutQuart", Easing::OutQuart),
    ("InOutQuart", Easing::InOutQuart),
    ("InQuint", Easing::InQuint),
    ("OutQuint", Easing::OutQuint),
    ("InOutQuint", Easing::InOutQuint),
    ("InSine", Easing::InSine),
    ("OutSine", Easing::OutSine),
    ("InOutSine", Easing::InOutSine),
    ("InExpo", Easing::InExpo),
    ("OutExpo", Easing::OutExpo),
    ("InOutExpo", Easing::InOutExpo),
    ("InCirc", Easing::InCirc),
    ("OutCirc", Easing::OutCirc),
    ("InOutCirc", Easing::InOutCirc),
];

fn easing(name: &str) -> Option<Easing> {
    match name {
        "InBack" => Some(Easing::IN_BACK),
        "OutBack" => Some(Easing::OUT_BACK),
        "InOutBack" => Some(Easing::IN_OUT_BACK),
        _ => EASINGS.iter().find(|(n, _)| *n == name).map(|(_, e)| *e),
    }
}

fn motion(body: &Component, functions: &Functions) -> Result<Motion, Diagnostic> {
    let (mut duration, mut ease) = (250.0, Easing::default());
    let mut spring = SpringParams::new(0.0, 0.0);
    let known: &[&str] = match body.name.as_str() {
        "Tween" => &["duration", "easing"],
        "Spring" => &["spring", "damping", "mass", "epsilon", "max_velocity"],
        other => {
            return Err(Diagnostic::unknown(body.span, "motion", other, ["Tween", "Spring"]));
        }
    };
    for m in &body.members {
        let Member::Set(n, s, e) = m else {
            return Err(Diagnostic::new(body.span, "a motion takes only plain properties"));
        };
        if !known.contains(&n.as_str()) {
            return Err(Diagnostic::unknown(*s, "motion property", n, known.iter().copied()));
        }
        let value = e.eval(&Names(Vec::new()), functions)?;
        match (n.as_str(), value) {
            ("duration", Value::Number(v)) => duration = v,
            ("easing", Value::Text(t)) => {
                ease = easing(&t).ok_or_else(|| {
                    Diagnostic::unknown(
                        e.span,
                        "easing",
                        &t,
                        EASINGS.iter().map(|(n, _)| *n).chain(["InBack", "OutBack", "InOutBack"]),
                    )
                })?
            }
            ("spring", Value::Number(v)) => spring.spring = v as f32,
            ("damping", Value::Number(v)) => spring.damping = v as f32,
            ("mass", Value::Number(v)) => spring.mass = v as f32,
            ("epsilon", Value::Number(v)) => spring.epsilon = v as f32,
            ("max_velocity", Value::Number(v)) => spring.max_velocity = v as f32,
            (n, _) => return Err(Diagnostic::new(e.span, format!("`{n}` has the wrong type"))),
        }
    }
    Ok(if body.name == "Tween" { Motion::tween(duration.max(0.0) as u32, ease) } else { Motion::Spring(spring) })
}

fn placeholder(kind: Kind) -> Prop {
    match kind {
        Kind::Number => Prop::number(0.0),
        Kind::Text => Prop::text(""),
        Kind::Bool => Prop::bool(false),
        Kind::Color => Prop::color(Color::TRANSPARENT),
        Kind::Index => Prop::Index(Signal::new(0)),
        Kind::TextList => Prop::TextList(Signal::new(Vec::new())),
        _ => Prop::NumberList(Signal::new(Vec::new())),
    }
}

fn assign(target: &Prop, v: Value) -> Result<(), String> {
    let wrong = |v: &Value| format!("a {} prop cannot take {v:?}", target.kind().word());
    match (target, &v) {
        (Prop::Number(s), Value::Number(n)) => s.set(*n as f32),
        (Prop::Text(s), Value::Text(t)) => s.set(t.clone()),
        (Prop::Bool(s), Value::Bool(b)) => s.set(*b),
        (Prop::Color(s), Value::Color(c)) => s.set(*c),
        (Prop::Index(s), Value::Number(n)) if *n >= 0.0 && n.fract() == 0.0 => s.set(*n as usize),
        (Prop::TextList(s), Value::List(l)) => {
            let items: Option<Vec<String>> =
                l.iter().map(|i| if let Value::Text(t) = i { Some(t.clone()) } else { None }).collect();
            s.set(items.ok_or_else(|| wrong(&v))?)
        }
        (Prop::NumberList(s), Value::List(l)) => {
            let items: Option<Vec<f32>> =
                l.iter().map(|i| if let Value::Number(n) = i { Some(*n as f32) } else { None }).collect();
            s.set(items.ok_or_else(|| wrong(&v))?)
        }
        _ => return Err(wrong(&v)),
    }
    Ok(())
}

struct Walk<'a> {
    registry: &'a Registry,
    scene: &'a mut Scene,
    locals: Rc<Locals>,
    functions: Rc<Functions>,
    declared: Vec<(Expr, Signal<Value>)>,
    bindings: Vec<(Expr, Prop)>,
    ids: HashMap<String, NodeId>,
}

impl Walk<'_> {
    fn handler(&self, kind: Kind, e: Expr, name: &str) -> Prop {
        let (locals, functions, name) = (self.locals.clone(), self.functions.clone(), name.to_string());
        let run = Rc::new(move |arg: Option<Value>| {
            let previous = locals.arg.replace(arg);
            if let Err(d) = e.eval(&*locals, &functions) {
                tracing::warn!("{name}: {}", d.message);
            }
            locals.arg.replace(previous);
        });
        match kind {
            Kind::NumberEvent => Prop::NumberEvent(Rc::new(move |v| run(Some(Value::Number(v.into()))))),
            Kind::BoolEvent => Prop::BoolEvent(Rc::new(move |v| run(Some(Value::Bool(v))))),
            Kind::IndexEvent => Prop::IndexEvent(Rc::new(move |v| run(Some(Value::Number(v as f64))))),
            _ => Prop::Event(Rc::new(move || run(None))),
        }
    }

    fn component(&mut self, c: &Component, parent: NodeId) -> Result<NodeId, Diagnostic> {
        let widget = self.registry.get(&c.name).expect("names are validated before the walk");
        let mut given = Vec::new();
        let mut id = None;
        for m in &c.members {
            if let Member::Property(n, _, e) = m {
                let signal = Signal::new(Value::Null);
                self.locals.declared.borrow_mut().insert(n.clone(), signal.clone());
                self.declared.push((e.clone(), signal));
            }
        }
        for m in &c.members {
            match m {
                Member::Id(n, _) => id = Some(n.clone()),
                Member::Set(n, _, e) => {
                    let kind = widget.props().iter().find(|s| s.name == n).expect("validated").kind;
                    let target = placeholder(kind);
                    if let Ok(v) = e.eval(&*self.locals, &self.functions) {
                        let _ = assign(&target, v);
                    }
                    self.bindings.push((e.clone(), target.clone()));
                    given.push((n.clone(), target));
                }
                Member::Handler(n, _, e) => {
                    let kind = widget.props().iter().find(|s| s.name == n).expect("validated").kind;
                    given.push((n.clone(), self.handler(kind, e.clone(), n)));
                }
                _ => {}
            }
        }
        let (_, props) = self.registry.resolve(&c.name, given).map_err(|e| Diagnostic::new(c.span, e.to_string()))?;
        let props = Rc::new(props);
        if let Some(id) = &id {
            self.locals.ids.borrow_mut().insert(id.clone(), (widget.clone(), props.clone()));
        }
        let node = widget.build(self.scene, parent, &props);
        if let Some(id) = id {
            self.ids.insert(id, node);
        }
        for m in &c.members {
            if let Member::Behavior(prop, _, body) = m
                && let (Some(prop), Ok(motion), Some(slot)) =
                    (node_prop(prop), motion(body, &self.functions), self.scene.get_mut(node))
            {
                let taken = std::mem::replace(slot, Node::item());
                *slot = taken.animate(prop, motion);
            }
        }
        for m in &c.members {
            if let Member::Child(child) = m
                && let Err(d) = self.component(child, node)
            {
                self.scene.remove(node);
                return Err(d);
            }
        }
        Ok(node)
    }
}

/// Compiles `doc` into nodes under `parent`: every component is built through `registry`, every
/// `name: expr` becomes a binding that re-evaluates when a signal it read changes, and every
/// `on_event: expr` runs when the event fires. Names, kinds and calls are all checked first, so a
/// failure returns every located error and leaves the scene untouched.
///
/// Expressions read the ids and `property` names of the file and the roots of `globals`; a handler
/// of a number, bool or index event also reads the event's argument as `value`. A binding that
/// fails to evaluate at run time keeps its last value and logs the reason.
pub fn build(
    doc: &Document,
    registry: &Registry,
    functions: Rc<Functions>,
    globals: Rc<dyn Scope>,
    scene: &mut Scene,
    parent: NodeId,
) -> Result<Built, Vec<Diagnostic>> {
    let expanded = Document { version: doc.version, root: crate::repeat::expand(&doc.root, &functions)? };
    let doc = &expanded;
    let mut errors = doc.check_names(registry);
    let (mut ids, mut declared) = (HashSet::new(), HashSet::new());
    collect(&doc.root, &mut ids, &mut declared, &mut errors);
    let names: Vec<String> =
        globals.roots().into_iter().chain(ids.iter().chain(declared.iter()).map(|s| s.to_string())).collect();
    validate_component(&doc.root, registry, &names, &functions, &mut errors);
    if !errors.is_empty() {
        errors.sort_by_key(|d| d.span.start);
        errors.dedup_by(|a, b| a.span == b.span && a.message == b.message);
        return Err(errors);
    }
    let locals =
        Rc::new(Locals { ids: RefCell::default(), declared: RefCell::default(), arg: RefCell::new(None), globals });
    let mut walk = Walk {
        registry,
        scene,
        locals: locals.clone(),
        functions: functions.clone(),
        declared: Vec::new(),
        bindings: Vec::new(),
        ids: HashMap::new(),
    };
    let root = walk.component(&doc.root, parent).map_err(|d| vec![d])?;
    let mut effects = Vec::new();
    for (expr, signal) in walk.declared {
        let (locals, functions) = (locals.clone(), functions.clone());
        effects.push(Effect::new(move || match expr.eval(&*locals, &functions) {
            Ok(v) => signal.set(v),
            Err(d) => tracing::warn!("property: {}", d.message),
        }));
    }
    for (expr, target) in walk.bindings {
        let (locals, functions) = (locals.clone(), functions.clone());
        effects.push(Effect::new(move || match expr.eval(&*locals, &functions) {
            Ok(v) => {
                if let Err(m) = assign(&target, v) {
                    tracing::warn!("{}", Diagnostic::new(Span::default(), m));
                }
            }
            Err(d) => tracing::warn!("binding: {}", d.message),
        }));
    }
    Ok(Built { root, ids: walk.ids, _effects: effects })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;
    use acanthus_core::Vec2;
    use acanthus_scene::{Fonts, Node};
    use acanthus_widgets::component::PropSpec;

    #[derive(Clone)]
    struct Taken {
        width: Signal<f32>,
        label: Signal<String>,
        click: Rc<dyn Fn()>,
        moved: Rc<dyn Fn(f32)>,
    }

    struct Probe(Rc<RefCell<Vec<Taken>>>);

    impl Widget for Probe {
        fn name(&self) -> &'static str {
            "Probe"
        }

        fn props(&self) -> &'static [PropSpec] {
            const PROPS: &[PropSpec] = &[
                PropSpec::number("width", || Prop::number(10.0)),
                PropSpec::text("label", || Prop::text("")),
                PropSpec::bool("on", || Prop::bool(false)),
                PropSpec::event("on_click"),
                PropSpec::number_event("on_moved"),
                PropSpec::point("body", || Prop::Point(Signal::new(Vec2::ZERO))),
            ];
            PROPS
        }

        fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
            self.0.borrow_mut().push(Taken {
                width: props.number("width"),
                label: props.text("label"),
                click: props.event("on_click"),
                moved: props.number_event("on_moved"),
            });
            scene.add(parent, Node::rect().width(props.number("width")).height(4.0))
        }
    }

    struct Globals(Signal<f64>);

    impl Scope for Globals {
        fn roots(&self) -> Vec<String> {
            vec!["volume".into()]
        }

        fn get(&self, path: &[String]) -> Option<Value> {
            (path == ["volume"]).then(|| Value::Number(self.0.get()))
        }
    }

    struct Run {
        scene: Scene,
        built: Result<Built, Vec<Diagnostic>>,
        taken: Rc<RefCell<Vec<Taken>>>,
        log: Rc<RefCell<Vec<Value>>>,
    }

    fn compile(src: &str, volume: &Signal<f64>) -> Run {
        let taken = Rc::new(RefCell::new(Vec::new()));
        let mut registry = Registry::new();
        registry.register(Probe(taken.clone()));
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut functions = Functions::standard();
        let sink = log.clone();
        functions.register("log", 1..=1, move |a| {
            sink.borrow_mut().push(a[0].clone());
            Ok(Value::Null)
        });
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let doc = parse(src).unwrap();
        let built = build(&doc, &registry, Rc::new(functions), Rc::new(Globals(volume.clone())), &mut scene, root);
        Run { scene, built, taken, log }
    }

    #[test]
    fn bindings_follow_globals_and_declared_properties() {
        let volume = Signal::new(0.5);
        let src = "Probe {\n id: a\n property double: volume * 2\n width: double + 1\n label: \"v\" + (a.on ? \"!\" : \"?\")\n}";
        let run = compile(src, &volume);
        let built = run.built.unwrap();
        assert!(built.ids.contains_key("a"));
        let taken = run.taken.borrow()[0].clone();
        assert_eq!((taken.width.get_untracked(), taken.label.get_untracked().as_str()), (2.0, "v?"));
        volume.set(3.0);
        assert_eq!(taken.width.get_untracked(), 7.0);
    }

    #[test]
    fn forward_references_between_components_resolve() {
        let volume = Signal::new(0.0);
        let src = "Probe {\n width: b.width + 1\n Probe { id: b\n width: 4 }\n}";
        let run = compile(src, &volume);
        let _built = run.built.unwrap();
        assert_eq!(run.taken.borrow()[0].width.get_untracked(), 5.0);
        assert_eq!(run.scene.get(_built.root).unwrap().children().len(), 1);
    }

    #[test]
    fn handlers_run_with_the_event_argument() {
        let volume = Signal::new(0.0);
        let src = "Probe {\n on_click: log(\"clicked\")\n on_moved: log(value * 2)\n}";
        let run = compile(src, &volume);
        let _built = run.built.unwrap();
        let taken = run.taken.borrow()[0].clone();
        (taken.click)();
        (taken.moved)(1.5);
        assert_eq!(*run.log.borrow(), vec![Value::Text("clicked".into()), Value::Number(3.0)]);
    }

    #[test]
    fn errors_are_all_reported_and_nothing_is_built() {
        let volume = Signal::new(0.0);
        let src = "Probe {\n widht: 1\n label: vol\n on_click: logg(1)\n body: 1\n on_click: value\n Behavior on wdth { Tween {} }\n}";
        let run = compile(src, &volume);
        let errors = run.built.err().unwrap();
        let messages: Vec<_> = errors.iter().map(|d| d.message.as_str()).collect();
        for expected in [
            "unknown property `widht`",
            "unknown name `vol`",
            "unknown function `logg`",
            "`body` cannot be set",
            "animatable property",
        ] {
            assert!(messages.iter().any(|m| m.contains(expected)), "{expected} in {messages:?}");
        }
        assert!(messages.iter().any(|m| m.contains("unknown name `value`")));
        assert!(run.scene.get(run.scene.root()).unwrap().children().is_empty());
        assert!(run.taken.borrow().is_empty());
    }

    #[test]
    fn behavior_makes_the_node_glide() {
        let volume = Signal::new(0.0);
        let src = "Probe {\n width: volume\n Behavior on width { Tween { duration: 100; easing: \"Linear\" } }\n}";
        let run = compile(src, &volume);
        let built = run.built.unwrap();
        volume.set(10.0);
        let width = run.scene.get(built.root).unwrap().width.clone().unwrap();
        assert_eq!(width.get_untracked(), 0.0);
        acanthus_anim::tick(0.05);
        assert!((width.get_untracked() - 5.0).abs() < 1e-3);
    }

    #[test]
    fn bad_motions_are_located_errors() {
        let volume = Signal::new(0.0);
        let src = "Probe {\n Behavior on width { Tween { easing: \"OutCubc\" } }\n Behavior on opacity { Spring { stiff: 1 } }\n Behavior on x { Fade {} }\n}";
        let errors = compile(src, &volume).built.err().unwrap();
        let messages: Vec<_> = errors.iter().map(|d| d.message.as_str()).collect();
        assert!(errors.iter().any(|d| d.hint.as_deref() == Some("did you mean `OutCubic`?")), "{errors:?}");
        assert!(messages.iter().any(|m| m.contains("unknown motion property `stiff`")), "{messages:?}");
        assert!(messages.iter().any(|m| m.contains("unknown motion `Fade`")), "{messages:?}");
    }

    #[test]
    fn repeater_makes_copies_with_their_index() {
        let volume = Signal::new(0.0);
        let src = "Probe {\n Repeater {\n count: 3\n Probe { width: index * 10 + 1 }\n }\n}";
        let run = compile(src, &volume);
        let built = run.built.unwrap();
        let widths: Vec<f32> = run.taken.borrow().iter().skip(1).map(|t| t.width.get_untracked()).collect();
        assert_eq!(widths, vec![1.0, 11.0, 21.0]);
        assert_eq!(run.scene.get(built.root).unwrap().children().len(), 3);
    }

    #[test]
    fn nested_repeaters_keep_their_own_index() {
        let volume = Signal::new(0.0);
        let src = "Probe {\n Repeater { count: 2\n Probe { width: index\n Repeater { count: index + 1\n Probe { width: 100 + index } } } }\n}";
        let run = compile(src, &volume);
        run.built.unwrap();
        let widths: Vec<f32> = run.taken.borrow().iter().skip(1).map(|t| t.width.get_untracked()).collect();
        assert_eq!(widths, vec![0.0, 100.0, 1.0, 100.0, 101.0]);
    }

    #[test]
    fn repeater_mistakes_are_located_errors() {
        let volume = Signal::new(0.0);
        for (src, expected) in [
            ("Probe { Repeater { Probe {} } }", "needs `count`"),
            ("Probe { Repeater { count: 2.5\n Probe {} } }", "whole number"),
            ("Probe { Repeater { count: volume\n Probe {} } }", "`volume` has no value"),
            ("Probe { Repeater { count: 1\n width: 2 } }", "takes `count`"),
            ("Repeater { count: 1 }", "root"),
            ("Probe { width: index }", "unknown name `index`"),
        ] {
            let errors = compile(src, &volume).built.err().unwrap();
            assert!(errors.iter().any(|d| d.message.contains(expected)), "{expected} in {errors:?}");
        }
    }

    #[test]
    fn duplicate_ids_and_properties_are_errors() {
        let volume = Signal::new(0.0);
        let run = compile("Probe { id: a\n property p: 1\n Probe { id: a\n property p: 2 } }", &volume);
        let errors = run.built.err().unwrap();
        assert!(errors.iter().any(|d| d.message.contains("id `a` is used twice")));
        assert!(errors.iter().any(|d| d.message.contains("property `p` is declared twice")));
    }
}
