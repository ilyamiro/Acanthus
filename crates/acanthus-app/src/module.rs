use std::any::{Any, TypeId, type_name};
use std::cell::RefCell;
use std::collections::HashMap;
use std::error::Error;

use acanthus_wayland::App;

/// What a started module hands back: an opaque value kept alive for the lifetime of the app.
pub type Running = Box<dyn Any>;

/// A unit of an application (bar, popup, service wiring) started once with the app context `Cx`.
pub trait Module<Cx: ?Sized> {
    /// Stable name used in logs and error messages.
    fn name(&self) -> &'static str;

    /// Builds the module's surfaces and wiring. The returned value is kept alive by the caller.
    fn start(&self, app: &mut App, cx: &Cx) -> Result<Running, Box<dyn Error>>;
}

/// Typed map through which modules share values: one value per type, cloned out on request.
#[derive(Default)]
pub struct Provided {
    values: RefCell<HashMap<TypeId, Box<dyn Any>>>,
}

impl Provided {
    /// Creates an empty map.
    pub fn new() -> Provided {
        Provided::default()
    }

    /// Stores `value`, replacing any earlier value of the same type.
    pub fn provide<T: Clone + 'static>(&self, value: T) {
        self.values.borrow_mut().insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Returns a clone of the value of type `T`, if a module provided one.
    pub fn get<T: Clone + 'static>(&self) -> Option<T> {
        self.values.borrow().get(&TypeId::of::<T>()).and_then(|value| value.downcast_ref::<T>()).cloned()
    }

    /// Like [`Provided::get`] but an error naming the missing type, for modules that cannot start without it.
    pub fn require<T: Clone + 'static>(&self) -> Result<T, Box<dyn Error>> {
        self.get::<T>().ok_or_else(|| format!("no module provided {}", type_name::<T>()).into())
    }
}

/// Starts `modules` in order. `on_stage` is told what is about to start (for crash reports).
/// Stops at the first failure and names the module in the error.
pub fn start_all<'m, Cx: ?Sized, M: Module<Cx> + ?Sized + 'm>(
    modules: impl IntoIterator<Item = &'m M>,
    app: &mut App,
    cx: &Cx,
    mut on_stage: impl FnMut(&str),
) -> Result<Vec<Running>, Box<dyn Error>> {
    let mut running = Vec::new();
    for module in modules {
        on_stage(&format!("starting module {}", module.name()));
        let started = module.start(app, cx).map_err(|error| format!("module {}: {error}", module.name()))?;
        tracing::debug!("module {} started", module.name());
        running.push(started);
    }
    Ok(running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provided_values_are_keyed_by_type() {
        let provided = Provided::new();
        provided.provide(7_u32);
        provided.provide("name".to_owned());
        assert_eq!(provided.get::<u32>(), Some(7));
        assert_eq!(provided.get::<String>().as_deref(), Some("name"));
        assert_eq!(provided.get::<u8>(), None);
    }

    #[test]
    fn providing_again_replaces_the_value() {
        let provided = Provided::new();
        provided.provide(1_u32);
        provided.provide(2_u32);
        assert_eq!(provided.get::<u32>(), Some(2));
    }

    #[test]
    fn require_names_the_missing_type() {
        let error = Provided::new().require::<u64>().unwrap_err().to_string();
        assert!(error.contains("u64"), "{error}");
    }
}
