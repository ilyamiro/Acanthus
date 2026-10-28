//! The component contract: a widget declares typed, named, defaultable props and builds itself from them.
//!
//! The Rust builder API and the `.serp` compiler meet here. A [`Component`] lists its [`PropSpec`]s, a
//! [`Registry`] resolves names, fills defaults and rejects unknown or mistyped props with a suggestion
//! before `build` runs, so a typo never reaches the scene as a silent fallback.

use std::collections::HashMap;
use std::fmt;
use std::rc::Rc;

use acanthus_core::{Color, Signal};
use acanthus_scene::{NodeId, Scene};

/// The type of a prop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A number signal.
    Number,
    /// A text signal.
    Text,
    /// A boolean signal.
    Bool,
    /// A colour signal.
    Color,
    /// A callback without arguments.
    Event,
    /// A callback receiving a number.
    NumberEvent,
}

impl Kind {
    /// The name of the type as the language spells it.
    pub fn word(self) -> &'static str {
        match self {
            Kind::Number => "number",
            Kind::Text => "text",
            Kind::Bool => "bool",
            Kind::Color => "color",
            Kind::Event => "event",
            Kind::NumberEvent => "number event",
        }
    }
}

/// The value of a prop. A static value is a signal nobody writes; a bound one is the signal of its source.
#[derive(Clone)]
pub enum Prop {
    /// A number signal.
    Number(Signal<f32>),
    /// A text signal.
    Text(Signal<String>),
    /// A boolean signal.
    Bool(Signal<bool>),
    /// A colour signal.
    Color(Signal<Color>),
    /// A callback without arguments.
    Event(Rc<dyn Fn()>),
    /// A callback receiving a number.
    NumberEvent(Rc<dyn Fn(f32)>),
}

impl Prop {
    /// The type of the value.
    pub fn kind(&self) -> Kind {
        match self {
            Prop::Number(_) => Kind::Number,
            Prop::Text(_) => Kind::Text,
            Prop::Bool(_) => Kind::Bool,
            Prop::Color(_) => Kind::Color,
            Prop::Event(_) => Kind::Event,
            Prop::NumberEvent(_) => Kind::NumberEvent,
        }
    }

    /// A static number.
    pub fn number(value: f32) -> Prop {
        Prop::Number(Signal::new(value))
    }

    /// A static text.
    pub fn text(value: impl Into<String>) -> Prop {
        Prop::Text(Signal::new(value.into()))
    }

    /// A static boolean.
    pub fn bool(value: bool) -> Prop {
        Prop::Bool(Signal::new(value))
    }

    /// A static colour.
    pub fn color(value: Color) -> Prop {
        Prop::Color(Signal::new(value))
    }
}

/// Declaration of one prop of a component.
pub struct PropSpec {
    /// Name of the prop.
    pub name: &'static str,
    /// Type of the prop.
    pub kind: Kind,
    /// Value used when the prop is not given; `None` makes the prop required.
    pub default: Option<fn() -> Prop>,
}

impl PropSpec {
    /// An optional number prop.
    pub const fn number(name: &'static str, default: fn() -> Prop) -> PropSpec {
        PropSpec { name, kind: Kind::Number, default: Some(default) }
    }

    /// An optional boolean prop.
    pub const fn bool(name: &'static str, default: fn() -> Prop) -> PropSpec {
        PropSpec { name, kind: Kind::Bool, default: Some(default) }
    }

    /// An optional callback without arguments; it does nothing by default.
    pub const fn event(name: &'static str) -> PropSpec {
        PropSpec { name, kind: Kind::Event, default: Some(|| Prop::Event(Rc::new(|| {}))) }
    }

    /// An optional callback receiving a number; it does nothing by default.
    pub const fn number_event(name: &'static str) -> PropSpec {
        PropSpec { name, kind: Kind::NumberEvent, default: Some(|| Prop::NumberEvent(Rc::new(|_| {}))) }
    }

    /// A prop of `kind` that must be given.
    pub const fn required(name: &'static str, kind: Kind) -> PropSpec {
        PropSpec { name, kind, default: None }
    }
}

/// The validated props of one component instance: every declared prop is present with its declared type.
pub struct Props {
    values: HashMap<&'static str, Prop>,
}

impl Props {
    fn get(&self, name: &str) -> &Prop {
        self.values.get(name).unwrap_or_else(|| panic!("prop `{name}` is not declared by the component"))
    }

    /// The number prop `name`.
    ///
    /// # Panics
    /// When the component did not declare `name` as a number.
    pub fn number(&self, name: &str) -> Signal<f32> {
        match self.get(name) {
            Prop::Number(signal) => signal.clone(),
            other => panic!("prop `{name}` is a {}, not a number", other.kind().word()),
        }
    }

    /// The text prop `name`.
    ///
    /// # Panics
    /// When the component did not declare `name` as text.
    pub fn text(&self, name: &str) -> Signal<String> {
        match self.get(name) {
            Prop::Text(signal) => signal.clone(),
            other => panic!("prop `{name}` is a {}, not text", other.kind().word()),
        }
    }

    /// The boolean prop `name`.
    ///
    /// # Panics
    /// When the component did not declare `name` as a boolean.
    pub fn bool(&self, name: &str) -> Signal<bool> {
        match self.get(name) {
            Prop::Bool(signal) => signal.clone(),
            other => panic!("prop `{name}` is a {}, not a bool", other.kind().word()),
        }
    }

    /// The colour prop `name`.
    ///
    /// # Panics
    /// When the component did not declare `name` as a colour.
    pub fn color(&self, name: &str) -> Signal<Color> {
        match self.get(name) {
            Prop::Color(signal) => signal.clone(),
            other => panic!("prop `{name}` is a {}, not a color", other.kind().word()),
        }
    }

    /// The callback prop `name`.
    ///
    /// # Panics
    /// When the component did not declare `name` as an event.
    pub fn event(&self, name: &str) -> Rc<dyn Fn()> {
        match self.get(name) {
            Prop::Event(callback) => callback.clone(),
            other => panic!("prop `{name}` is a {}, not an event", other.kind().word()),
        }
    }

    /// The number callback prop `name`.
    ///
    /// # Panics
    /// When the component did not declare `name` as a number event.
    pub fn number_event(&self, name: &str) -> Rc<dyn Fn(f32)> {
        match self.get(name) {
            Prop::NumberEvent(callback) => callback.clone(),
            other => panic!("prop `{name}` is a {}, not a number event", other.kind().word()),
        }
    }
}

/// A widget the registry can build by name.
pub trait Component {
    /// Name the component is registered and written under.
    fn name(&self) -> &'static str;

    /// Props the component accepts.
    fn props(&self) -> &'static [PropSpec];

    /// Builds the component under `parent` from validated `props` and returns its root node.
    fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId;
}

/// Why a component could not be built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BuildError {
    /// No component has this name.
    UnknownComponent {
        /// The name that was asked for.
        name: String,
        /// The closest registered name.
        suggestion: Option<String>,
    },
    /// The component has no prop of this name.
    UnknownProp {
        /// The component.
        component: &'static str,
        /// The prop that was given.
        name: String,
        /// The closest declared prop.
        suggestion: Option<String>,
    },
    /// The prop was given a value of another type.
    WrongKind {
        /// The component.
        component: &'static str,
        /// The prop.
        name: &'static str,
        /// The declared type.
        expected: Kind,
        /// The type that was given.
        found: Kind,
    },
    /// A required prop was not given.
    Missing {
        /// The component.
        component: &'static str,
        /// The prop.
        name: &'static str,
    },
    /// The prop was given twice.
    Duplicate {
        /// The component.
        component: &'static str,
        /// The prop.
        name: &'static str,
    },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fn hint(f: &mut fmt::Formatter<'_>, suggestion: &Option<String>) -> fmt::Result {
            match suggestion {
                Some(word) => write!(f, ", did you mean `{word}`?"),
                None => Ok(()),
            }
        }
        match self {
            BuildError::UnknownComponent { name, suggestion } => {
                write!(f, "unknown component `{name}`")?;
                hint(f, suggestion)
            }
            BuildError::UnknownProp { component, name, suggestion } => {
                write!(f, "`{component}` has no prop `{name}`")?;
                hint(f, suggestion)
            }
            BuildError::WrongKind { component, name, expected, found } => {
                write!(f, "`{component}.{name}` expects a {}, got a {}", expected.word(), found.word())
            }
            BuildError::Missing { component, name } => write!(f, "`{component}` needs the prop `{name}`"),
            BuildError::Duplicate { component, name } => write!(f, "`{component}.{name}` is given twice"),
        }
    }
}

impl std::error::Error for BuildError {}

/// The candidate closest to `word`, if one is near enough to be a plausible typo.
pub fn nearest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (word.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|candidate| (edit_distance(word, candidate), candidate))
        .filter(|(distance, _)| *distance <= limit)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in table.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in table[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (table[i - 1][j] + 1).min(table[i][j - 1] + 1).min(table[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(table[i - 2][j - 2] + 1);
            }
            table[i][j] = best;
        }
    }
    table[a.len()][b.len()]
}

/// Components by name.
#[derive(Default)]
pub struct Registry {
    components: HashMap<&'static str, Rc<dyn Component>>,
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Registry {
        Registry::default()
    }

    /// Registers `component` under its name, replacing an earlier one of the same name.
    pub fn register(&mut self, component: impl Component + 'static) {
        self.components.insert(component.name(), Rc::new(component));
    }

    /// The registered component `name`.
    pub fn get(&self, name: &str) -> Option<Rc<dyn Component>> {
        self.components.get(name).cloned()
    }

    /// Registered names, sorted.
    pub fn names(&self) -> Vec<&'static str> {
        let mut names: Vec<_> = self.components.keys().copied().collect();
        names.sort_unstable();
        names
    }

    /// Checks `given` against the declaration of the component `name`: every prop is known, typed and
    /// given once, required ones are present and the rest get their defaults.
    pub fn resolve(&self, name: &str, given: Vec<(String, Prop)>) -> Result<(Rc<dyn Component>, Props), BuildError> {
        let component = self.get(name).ok_or_else(|| BuildError::UnknownComponent {
            name: name.to_owned(),
            suggestion: nearest(name, self.components.keys().copied()).map(str::to_owned),
        })?;
        let specs = component.props();
        let mut values: HashMap<&'static str, Prop> = HashMap::new();
        for (prop_name, value) in given {
            let Some(spec) = specs.iter().find(|spec| spec.name == prop_name) else {
                return Err(BuildError::UnknownProp {
                    component: component.name(),
                    suggestion: nearest(&prop_name, specs.iter().map(|spec| spec.name)).map(str::to_owned),
                    name: prop_name,
                });
            };
            if value.kind() != spec.kind {
                return Err(BuildError::WrongKind {
                    component: component.name(),
                    name: spec.name,
                    expected: spec.kind,
                    found: value.kind(),
                });
            }
            if values.insert(spec.name, value).is_some() {
                return Err(BuildError::Duplicate { component: component.name(), name: spec.name });
            }
        }
        for spec in specs {
            if !values.contains_key(spec.name) {
                let Some(default) = spec.default else {
                    return Err(BuildError::Missing { component: component.name(), name: spec.name });
                };
                values.insert(spec.name, default());
            }
        }
        Ok((component, Props { values }))
    }

    /// Resolves the props and builds the component `name` under `parent`.
    pub fn build(
        &self,
        name: &str,
        scene: &mut Scene,
        parent: NodeId,
        given: Vec<(String, Prop)>,
    ) -> Result<NodeId, BuildError> {
        let (component, props) = self.resolve(name, given)?;
        Ok(component.build(scene, parent, &props))
    }
}

#[cfg(test)]
mod tests {
    use acanthus_scene::{Fonts, Node};

    use super::*;

    struct Swatch;

    impl Component for Swatch {
        fn name(&self) -> &'static str {
            "Swatch"
        }

        fn props(&self) -> &'static [PropSpec] {
            const PROPS: &[PropSpec] = &[
                PropSpec::number("width", || Prop::number(10.0)),
                PropSpec::required("fill", Kind::Color),
                PropSpec::event("on_click"),
            ];
            PROPS
        }

        fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
            scene.add(parent, Node::rect().width(props.number("width")).height(4.0).color(props.color("fill")))
        }
    }

    fn registry() -> Registry {
        let mut registry = Registry::new();
        registry.register(Swatch);
        registry
    }

    fn fill() -> (String, Prop) {
        ("fill".to_owned(), Prop::color(Color::hex(0xff0000)))
    }

    #[test]
    fn defaults_fill_the_props_that_are_not_given() {
        let (_, props) = registry().resolve("Swatch", vec![fill()]).unwrap();
        assert_eq!(props.number("width").get_untracked(), 10.0);
        (props.event("on_click"))();
    }

    #[test]
    fn a_bound_prop_keeps_its_signal() {
        let width = Signal::new(3.0f32);
        let given = vec![fill(), ("width".to_owned(), Prop::Number(width.clone()))];
        let (_, props) = registry().resolve("Swatch", given).unwrap();
        width.set(7.0);
        assert_eq!(props.number("width").get_untracked(), 7.0);
    }

    #[test]
    fn unknown_names_come_with_the_nearest_known_one() {
        let err = registry().resolve("Swatsh", vec![]).err().unwrap();
        assert_eq!(err.to_string(), "unknown component `Swatsh`, did you mean `Swatch`?");
        let err = registry().resolve("Swatch", vec![fill(), ("widht".to_owned(), Prop::number(1.0))]).err().unwrap();
        assert_eq!(err.to_string(), "`Swatch` has no prop `widht`, did you mean `width`?");
        let err = registry().resolve("Swatch", vec![fill(), ("zzzzzz".to_owned(), Prop::number(1.0))]).err().unwrap();
        assert_eq!(err.to_string(), "`Swatch` has no prop `zzzzzz`");
    }

    #[test]
    fn wrong_kinds_missing_and_duplicate_props_are_errors() {
        let r = registry();
        let err = r.resolve("Swatch", vec![("fill".to_owned(), Prop::number(1.0))]).err().unwrap();
        assert_eq!(err.to_string(), "`Swatch.fill` expects a color, got a number");
        let err = r.resolve("Swatch", vec![]).err().unwrap();
        assert_eq!(err, BuildError::Missing { component: "Swatch", name: "fill" });
        let err = r.resolve("Swatch", vec![fill(), fill()]).err().unwrap();
        assert_eq!(err.to_string(), "`Swatch.fill` is given twice");
    }

    #[test]
    fn build_adds_the_node_under_the_parent() {
        let mut scene = Scene::new(Fonts::new());
        let root = scene.root();
        let node = registry().build("Swatch", &mut scene, root, vec![fill()]).unwrap();
        assert!(scene.get(node).is_some());
        assert_eq!(registry().names(), vec!["Swatch"]);
    }

    #[test]
    fn edit_distance_counts_single_edits() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("widht", "width"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(nearest("opacitty", ["opacity", "visible"]), Some("opacity"));
        assert_eq!(nearest("x", ["opacity"]), None);
    }
}
