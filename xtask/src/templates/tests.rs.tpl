use std::cell::RefCell;
use std::rc::Rc;

use super::controller::{Controller, Host};
use super::model::{self, State};

#[derive(Default)]
struct FakeHost {
    seen: RefCell<Vec<State>>,
}

impl Host for FakeHost {
    fn changed(&self, state: State) {
        self.seen.borrow_mut().push(state);
    }
}

#[test]
fn model_counts_up() {
    assert_eq!(model::advance(State::default()).count, 1);
}

#[test]
fn controller_updates_signals_and_tells_the_host() {
    let host = Rc::new(FakeHost::default());
    let controller = Controller::new(host.clone());
    controller.advance();
    assert_eq!(controller.state.get_untracked().count, 1);
    assert_eq!(controller.label.get_untracked(), "{{name}} 1");
    assert_eq!(host.seen.borrow().len(), 1);
}
