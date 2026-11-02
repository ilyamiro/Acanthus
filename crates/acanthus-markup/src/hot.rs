use crate::{Built, Diagnostic, Functions, Scope, build, parse_all};
use acanthus_core::Color;
use acanthus_io::{IoError, Watch};
use acanthus_scene::{Anchors, Node, NodeId, Scene};
use acanthus_widgets::component::Registry;
use calloop::LoopHandle;
use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

/// An ACN file kept on screen and rebuilt whenever it changes.
///
/// A save that fails to parse or compile leaves the last good tree in place and shows a red band
/// with the first `file:line:col` error at the bottom of the parent; the next good save removes it.
pub struct Hot {
    path: PathBuf,
    registry: Rc<Registry>,
    functions: Rc<Functions>,
    globals: Rc<dyn Scope>,
    built: Option<Built>,
    band: Option<NodeId>,
    dirty: Rc<Cell<bool>>,
    _watch: Option<Watch>,
}

impl Hot {
    /// A reloader for `path`; nothing is built until [`Hot::poll`] or [`Hot::reload`] runs.
    pub fn new(path: PathBuf, registry: Rc<Registry>, functions: Rc<Functions>, globals: Rc<dyn Scope>) -> Hot {
        Hot {
            path,
            registry,
            functions,
            globals,
            built: None,
            band: None,
            dirty: Rc::new(Cell::new(true)),
            _watch: None,
        }
    }

    /// Starts watching the file on the event loop; a change makes the next [`Hot::poll`] reload.
    pub fn watch<D: 'static>(&mut self, handle: &LoopHandle<'static, D>) -> Result<(), IoError> {
        let dirty = self.dirty.clone();
        self._watch = Some(Watch::file(handle, &self.path, move |_| dirty.set(true))?);
        Ok(())
    }

    /// The tree currently on screen.
    pub fn built(&self) -> Option<&Built> {
        self.built.as_ref()
    }

    /// Reloads when the file changed since the last call (or was never built); `None` when it did not.
    pub fn poll(&mut self, scene: &mut Scene, parent: NodeId) -> Option<Result<(), Vec<Diagnostic>>> {
        self.dirty.get().then(|| self.reload(scene, parent))
    }

    /// Reads, parses and builds the file under `parent`, then swaps it for the previous tree. On any
    /// error the previous tree stays, the error band shows the first error and all errors are returned.
    pub fn reload(&mut self, scene: &mut Scene, parent: NodeId) -> Result<(), Vec<Diagnostic>> {
        self.dirty.set(false);
        let file = self.path.display().to_string();
        let src = match std::fs::read_to_string(&self.path) {
            Ok(src) => src,
            Err(e) => {
                return Err(self.fail(
                    scene,
                    parent,
                    &file,
                    "",
                    vec![Diagnostic::new(Default::default(), e.to_string())],
                ));
            }
        };
        let result = parse_all(&src)
            .and_then(|doc| build(&doc, &self.registry, self.functions.clone(), self.globals.clone(), scene, parent));
        match result {
            Ok(built) => {
                if let Some(old) = self.built.replace(built) {
                    scene.remove(old.root);
                }
                if let Some(band) = self.band.take() {
                    scene.remove(band);
                }
                Ok(())
            }
            Err(errors) => Err(self.fail(scene, parent, &file, &src, errors)),
        }
    }

    fn fail(
        &mut self,
        scene: &mut Scene,
        parent: NodeId,
        file: &str,
        src: &str,
        errors: Vec<Diagnostic>,
    ) -> Vec<Diagnostic> {
        let (line, col) = errors[0].position(src);
        let text = format!("{file}:{line}:{col}: {}", errors[0].message);
        tracing::warn!("{text}");
        if let Some(band) = self.band.take() {
            scene.remove(band);
        }
        let edges = Anchors { left: Some(0.0), right: Some(0.0), bottom: Some(0.0), ..Anchors::default() };
        let band = scene.add(
            parent,
            Node::rect().color(Color::rgba(0.55, 0.08, 0.08, 0.95)).anchors(edges).height(40.0).z(10_000.0),
        );
        let inset = Anchors { left: Some(12.0), v_center: true, ..Anchors::default() };
        scene.add(band, Node::text(text).color(Color::WHITE).font_size(13.0).anchors(inset));
        self.band = Some(band);
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acanthus_core::Signal;
    use acanthus_scene::Fonts;
    use acanthus_widgets::component::{Component, Prop, PropSpec, Props};

    struct Box_;

    impl Component for Box_ {
        fn name(&self) -> &'static str {
            "Box"
        }

        fn props(&self) -> &'static [PropSpec] {
            const PROPS: &[PropSpec] = &[PropSpec::number("width", || Prop::number(1.0))];
            PROPS
        }

        fn build(&self, scene: &mut Scene, parent: NodeId, props: &Props) -> NodeId {
            scene.add(parent, Node::rect().width(props.number("width")).height(2.0))
        }
    }

    struct None_;

    impl Scope for None_ {
        fn roots(&self) -> Vec<String> {
            Vec::new()
        }

        fn get(&self, _: &[String]) -> Option<crate::Value> {
            None
        }
    }

    fn rig(dir: &tempfile::TempDir, src: &str) -> (Hot, Scene, PathBuf) {
        let path = dir.path().join("a.acn");
        std::fs::write(&path, src).unwrap();
        let mut registry = Registry::new();
        registry.register(Box_);
        let hot = Hot::new(path.clone(), Rc::new(registry), Rc::new(Functions::standard()), Rc::new(None_));
        (hot, Scene::new(Fonts::new()), path)
    }

    fn width(scene: &Scene, hot: &Hot) -> f32 {
        scene.get(hot.built().unwrap().root).unwrap().width.clone().map_or(0.0, |s: Signal<f32>| s.get_untracked())
    }

    #[test]
    fn a_good_save_replaces_the_tree() {
        let dir = tempfile::tempdir().unwrap();
        let (mut hot, mut scene, path) = rig(&dir, "Box { width: 5 }");
        let root = scene.root();
        assert!(hot.poll(&mut scene, root).unwrap().is_ok());
        assert_eq!(width(&scene, &hot), 5.0);
        assert!(hot.poll(&mut scene, root).is_none());
        std::fs::write(&path, "Box { width: 9 }").unwrap();
        hot.reload(&mut scene, root).unwrap();
        assert_eq!(width(&scene, &hot), 9.0);
        assert_eq!(scene.get(root).unwrap().children().len(), 1);
    }

    #[test]
    fn a_broken_save_keeps_the_last_tree_and_shows_a_band() {
        let dir = tempfile::tempdir().unwrap();
        let (mut hot, mut scene, path) = rig(&dir, "Box { width: 5 }");
        let root = scene.root();
        hot.reload(&mut scene, root).unwrap();
        let before = hot.built().unwrap().root;
        std::fs::write(&path, "Box {\n widht: 9\n}").unwrap();
        let errors = hot.reload(&mut scene, root).unwrap_err();
        assert!(errors[0].message.contains("unknown property `widht`"));
        assert_eq!(hot.built().unwrap().root, before);
        assert_eq!(scene.get(root).unwrap().children().len(), 2);
        std::fs::write(&path, "Box { width: 7").unwrap();
        hot.reload(&mut scene, root).unwrap_err();
        assert_eq!(scene.get(root).unwrap().children().len(), 2);
        std::fs::write(&path, "Box { width: 7 }").unwrap();
        hot.reload(&mut scene, root).unwrap();
        assert_eq!(scene.get(root).unwrap().children().len(), 1);
        assert_eq!(width(&scene, &hot), 7.0);
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let (mut hot, mut scene, path) = rig(&dir, "Box {}");
        std::fs::remove_file(path).unwrap();
        let root = scene.root();
        assert!(hot.reload(&mut scene, root).is_err());
        assert!(hot.built().is_none());
    }
}
