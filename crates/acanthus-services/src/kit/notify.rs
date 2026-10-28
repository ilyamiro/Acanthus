use std::rc::Rc;

use super::{Cmd, Exec};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

impl Urgency {
    pub fn from_hint(value: u8) -> Urgency {
        match value {
            0 => Urgency::Low,
            2 => Urgency::Critical,
            _ => Urgency::Normal,
        }
    }

    pub fn as_number(self) -> u8 {
        match self {
            Urgency::Low => 0,
            Urgency::Normal => 1,
            Urgency::Critical => 2,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Urgency::Low => "low",
            Urgency::Normal => "normal",
            Urgency::Critical => "critical",
        }
    }
}

pub struct Notice {
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub icon: String,
    pub urgency: Urgency,
    pub actions: Vec<(String, String)>,
    pub on_action: Option<Box<dyn FnOnce(String)>>,
}

impl Notice {
    pub fn new(app_name: impl Into<String>, summary: impl Into<String>, body: impl Into<String>) -> Notice {
        Notice {
            app_name: app_name.into(),
            summary: summary.into(),
            body: body.into(),
            icon: String::new(),
            urgency: Urgency::Normal,
            actions: Vec::new(),
            on_action: None,
        }
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Notice {
        self.icon = icon.into();
        self
    }

    pub fn urgency(mut self, urgency: Urgency) -> Notice {
        self.urgency = urgency;
        self
    }

    pub fn action(mut self, id: impl Into<String>, text: impl Into<String>) -> Notice {
        self.actions.push((id.into(), text.into()));
        self
    }

    pub fn on_action(mut self, handler: impl FnOnce(String) + 'static) -> Notice {
        self.on_action = Some(Box::new(handler));
        self
    }
}

pub trait Notifier {
    fn notify(&self, notice: Notice);
}

pub fn notify_send_argv(notice: &Notice) -> Vec<String> {
    let mut argv: Vec<String> = ["notify-send", "-a", &notice.app_name, "-u", notice.urgency.as_str()]
        .iter()
        .map(|part| (*part).to_owned())
        .collect();
    if !notice.icon.is_empty() {
        argv.extend(["-i".to_owned(), notice.icon.clone()]);
    }
    for (id, text) in &notice.actions {
        argv.extend(["-A".to_owned(), format!("{id}={text}")]);
    }
    argv.push(notice.summary.clone());
    argv.push(notice.body.clone());
    argv
}

pub struct NotifySendNotifier {
    exec: Rc<dyn Exec>,
}

impl NotifySendNotifier {
    pub fn new(exec: Rc<dyn Exec>) -> NotifySendNotifier {
        NotifySendNotifier { exec }
    }
}

impl Notifier for NotifySendNotifier {
    fn notify(&self, mut notice: Notice) {
        let argv = notify_send_argv(&notice);
        let handler = notice.on_action.take();
        self.exec.run(
            Cmd::new(argv),
            Box::new(move |info| {
                let chosen = info.stdout.trim();
                if let Some(handler) = handler
                    && info.success()
                    && !chosen.is_empty()
                {
                    handler(chosen.to_owned());
                }
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use acanthus_io::ExitInfo;

    use super::*;
    use crate::kit::testing::{FakeExec, FakeNotifier};

    #[test]
    fn the_argv_carries_app_urgency_icon_actions_summary_and_body_in_order() {
        let notice = Notice::new("System", "Low battery", "20 %")
            .urgency(Urgency::Critical)
            .icon("battery-level-20-symbolic")
            .action("default", "Open");
        assert_eq!(
            notify_send_argv(&notice),
            [
                "notify-send",
                "-a",
                "System",
                "-u",
                "critical",
                "-i",
                "battery-level-20-symbolic",
                "-A",
                "default=Open",
                "Low battery",
                "20 %"
            ]
        );
        assert_eq!(notify_send_argv(&Notice::new("a", "s", "b")), ["notify-send", "-a", "a", "-u", "normal", "s", "b"]);
    }

    #[test]
    fn the_chosen_action_reaches_the_handler_and_a_dismissal_does_not() {
        let fake = FakeExec::immediate();
        let notifier = NotifySendNotifier::new(fake.exec());
        let chosen = Rc::new(RefCell::new(Vec::new()));
        for stdout in ["default\n", "\n"] {
            fake.reply(ExitInfo { code: Some(0), stdout: stdout.to_owned(), ..Default::default() });
            let sink = chosen.clone();
            notifier.notify(
                Notice::new("a", "s", "b").action("default", "Open").on_action(move |id| sink.borrow_mut().push(id)),
            );
        }
        assert_eq!(*chosen.borrow(), ["default"]);
        assert_eq!(fake.call_count(), 2);
    }

    #[test]
    fn the_fake_records_notices_and_can_choose_an_action() {
        let fake = FakeNotifier::new();
        let chosen = Rc::new(RefCell::new(String::new()));
        let sink = chosen.clone();
        fake.notify(Notice::new("a", "s", "b").on_action(move |id| *sink.borrow_mut() = id));
        assert_eq!(fake.summaries(), ["s"]);
        fake.choose(0, "default");
        assert_eq!(*chosen.borrow(), "default");
        assert_eq!(fake.with(0, |notice| notice.app_name.clone()), "a");
    }

    #[test]
    fn urgency_round_trips_through_the_hint_number() {
        for urgency in [Urgency::Low, Urgency::Normal, Urgency::Critical] {
            assert_eq!(Urgency::from_hint(urgency.as_number()), urgency);
        }
    }
}
