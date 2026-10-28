#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub count: u32,
}

pub fn advance(state: State) -> State {
    State { count: state.count.saturating_add(1) }
}

pub fn label(state: State) -> String {
    format!("{{name}} {}", state.count)
}
