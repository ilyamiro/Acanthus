use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time::Duration;

use acanthus_core::Signal;
use acanthus_io::{IoError, TimerHandle, Watch, timer};
use calloop::LoopHandle;

use crate::kit::{Cmd, Exec};
use crate::platform::app_matcher::AppMatcher;
use crate::platform::desktop::{DesktopEntry, application_dirs, locales_from_env, scan};

const RESCAN_DELAY: Duration = Duration::from_millis(250);

struct Inner {
    dirs: Vec<PathBuf>,
    locales: Vec<String>,
    desktops: Vec<String>,
    entries: RefCell<Vec<Rc<DesktopEntry>>>,
    matcher: RefCell<Rc<AppMatcher>>,
    revision: Signal<u64>,
    pending: RefCell<Option<TimerHandle>>,
    terminal: String,
    exec: Rc<dyn Exec>,
}

pub struct DesktopEntries {
    inner: Rc<Inner>,
    _watches: Vec<Watch>,
}

impl Inner {
    fn rescan(&self) {
        let entries: Vec<Rc<DesktopEntry>> = scan(&self.dirs, &self.locales)
            .into_iter()
            .filter(|e| e.is_visible_in(&self.desktops))
            .map(Rc::new)
            .collect();
        *self.matcher.borrow_mut() = Rc::new(AppMatcher::new(&entries));
        *self.entries.borrow_mut() = entries;
        self.revision.update(|r| *r += 1);
    }
}

impl DesktopEntries {
    pub fn start<D: 'static>(
        handle: &LoopHandle<'static, D>,
        env: impl Fn(&str) -> Option<String>,
        exec: Rc<dyn Exec>,
    ) -> Result<DesktopEntries, IoError> {
        let desktops = env("XDG_CURRENT_DESKTOP")
            .map(|d| d.split(':').filter(|s| !s.is_empty()).map(str::to_owned).collect())
            .unwrap_or_default();
        let inner = Rc::new(Inner {
            dirs: application_dirs(&env),
            locales: locales_from_env(&env),
            desktops,
            entries: RefCell::default(),
            matcher: RefCell::new(Rc::new(AppMatcher::new(&[]))),
            revision: Signal::new(0),
            pending: RefCell::default(),
            terminal: env("TERMINAL").unwrap_or_default(),
            exec,
        });
        inner.rescan();

        let mut watches = Vec::new();
        for dir in &inner.dirs {
            if !dir.is_dir() {
                continue;
            }
            let (weak, timer_handle): (Weak<Inner>, _) = (Rc::downgrade(&inner), acanthus_io::WeakHandle::new(handle));
            let watch = Watch::directory(handle, dir, move |_| {
                let Some(inner) = weak.upgrade() else { return };
                if inner.pending.borrow().is_some() {
                    return;
                }
                let Some(loop_handle) = timer_handle.upgrade() else { return };
                let target = Rc::downgrade(&inner);
                let scheduled = timer::after(&loop_handle, RESCAN_DELAY, move || {
                    if let Some(inner) = target.upgrade() {
                        inner.pending.borrow_mut().take();
                        inner.rescan();
                    }
                });
                *inner.pending.borrow_mut() = scheduled.ok();
            })?;
            watches.push(watch);
        }
        Ok(DesktopEntries { inner, _watches: watches })
    }

    pub fn revision(&self) -> Signal<u64> {
        self.inner.revision.clone()
    }

    pub fn entries(&self) -> Vec<Rc<DesktopEntry>> {
        self.inner.revision.get();
        self.inner.entries.borrow().clone()
    }

    pub fn matcher(&self) -> Rc<AppMatcher> {
        self.inner.revision.get();
        self.inner.matcher.borrow().clone()
    }

    pub fn find(&self, id: &str) -> Option<Rc<DesktopEntry>> {
        self.matcher().entry(id)
    }

    pub fn launch(&self, entry: &DesktopEntry, files: &[String]) {
        let terminal = (!self.inner.terminal.is_empty()).then_some(self.inner.terminal.as_str());
        self.inner.exec.detached(Cmd::new(entry.launch_command(files, terminal)));
    }

    pub fn rescan_now(&self) {
        self.inner.rescan();
    }
}

#[cfg(test)]
mod tests {
    use crate::kit::testing::FakeExec;
    use std::collections::HashMap;
    use std::time::Instant;

    use calloop::EventLoop;

    use super::*;

    fn env_for(dir: &std::path::Path) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = [
            ("HOME".to_owned(), dir.to_string_lossy().into_owned()),
            ("XDG_DATA_HOME".to_owned(), dir.to_string_lossy().into_owned()),
            ("XDG_DATA_DIRS".to_owned(), String::new()),
            ("XDG_CURRENT_DESKTOP".to_owned(), "Hyprland".to_owned()),
            ("LANG".to_owned(), "de_DE.UTF-8".to_owned()),
        ]
        .into();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn rescans_when_applications_change_and_respects_desktop_visibility() {
        let home = tempfile::tempdir().unwrap();
        let apps = home.path().join("applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("a.desktop"),
            "[Desktop Entry]\nType=Application\nName=Alpha\nName[de]=Alfa\nExec=alpha",
        )
        .unwrap();
        std::fs::write(
            apps.join("gnome.desktop"),
            "[Desktop Entry]\nType=Application\nName=G\nExec=g\nOnlyShowIn=GNOME;",
        )
        .unwrap();

        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let service =
            DesktopEntries::start(&event_loop.handle(), env_for(home.path()), FakeExec::new().exec()).unwrap();
        let names: Vec<String> = service.entries().iter().map(|e| e.name.clone()).collect();
        assert_eq!(names, ["Alfa"]);
        assert_eq!(service.matcher().resolve("a"), "a");
        let before = service.revision().get_untracked();

        std::fs::write(
            apps.join("b.desktop"),
            "[Desktop Entry]\nType=Application\nName=Beta\nExec=/opt/bin/beta %u\nStartupWMClass=BetaClass",
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while service.revision().get_untracked() == before && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert_eq!(service.entries().len(), 2);
        assert_eq!(service.matcher().resolve("betaclass"), "b");
        assert_eq!(service.find("B.desktop").unwrap().name, "Beta");

        std::fs::remove_file(apps.join("a.desktop")).unwrap();
        let before = service.revision().get_untracked();
        let deadline = Instant::now() + Duration::from_secs(5);
        while service.revision().get_untracked() == before && Instant::now() < deadline {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
        assert!(service.find("a").is_none());
    }
}
