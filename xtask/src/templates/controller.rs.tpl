use std::rc::Rc;

use acanthus_core::Signal;

use super::model::{self, State};

pub trait Host {
    fn changed(&self, state: State);
}

pub struct Controller {
    pub state: Signal<State>,
    pub label: Signal<String>,
    host: Rc<dyn Host>,
}

impl Controller {
    pub fn new(host: Rc<dyn Host>) -> Rc<Self> {
        let state = State::default();
        Rc::new(Self { state: Signal::new(state), label: Signal::new(model::label(state)), host })
    }

    pub fn advance(&self) {
        let next = model::advance(self.state.get_untracked());
        self.state.set(next);
        self.label.set(model::label(next));
        self.host.changed(next);
    }
}

