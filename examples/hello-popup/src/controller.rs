use acanthus_app::Popup;
use acanthus_core::Signal;

use crate::model;

#[derive(Clone)]
pub struct Controller {
    pub open: Signal<bool>,
    pub text: Signal<String>,
}

impl Controller {
    pub fn new() -> Controller {
        Controller { open: Signal::new(false), text: Signal::new(model::message("")) }
    }
}

impl Popup for Controller {
    fn names(&self) -> &'static [&'static str] {
        &[model::NAME, "hello"]
    }

    fn group(&self) -> Option<&'static str> {
        Some(model::STACK)
    }

    fn is_open(&self) -> bool {
        self.open.get_untracked()
    }

    fn open(&self, arg: &str) {
        self.text.set(model::message(arg));
        self.open.set(true);
    }

    fn close(&self) {
        self.open.set(false);
    }
}
