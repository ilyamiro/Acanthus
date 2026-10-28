use std::ffi::OsString;
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::path::Path;

use calloop::generic::Generic;
use calloop::{Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};

use crate::error::IoError;
use crate::weak::WeakHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
/// A change to an entry of a watched directory.
pub struct WatchEvent {
    /// File name inside the directory.
    pub name: OsString,
    /// Whether the entry was removed or moved away.
    pub removed: bool,
}

/// An inotify watch on the event loop. Dropping it stops watching.
pub struct Watch {
    remove: Option<Box<dyn FnOnce()>>,
}

impl Watch {
    /// Watches one file through its parent directory, so replacement by rename is seen.
    pub fn file<D: 'static>(
        handle: &LoopHandle<'static, D>,
        path: &Path,
        mut on_change: impl FnMut(&WatchEvent) + 'static,
    ) -> Result<Watch, IoError> {
        let dir = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).to_path_buf();
        let file_name = path.file_name().map(OsString::from).unwrap_or_default();
        Watch::directory(handle, &dir, move |event| {
            if event.name == file_name {
                on_change(event);
            }
        })
    }

    /// Calls `on_change` when the content of an existing file is modified.
    pub fn modify<D: 'static>(
        handle: &LoopHandle<'static, D>,
        path: &Path,
        mut on_change: impl FnMut() + 'static,
    ) -> Result<Watch, IoError> {
        let fd = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)?;
        inotify::add_watch(&fd, path, WatchFlags::MODIFY | WatchFlags::CLOSE_WRITE)?;
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        let token: RegistrationToken = handle.insert_source(source, move |_, fd, _| {
            let mut buffer = [MaybeUninit::<u8>::uninit(); 1024];
            let mut reader = inotify::Reader::new(&**fd, &mut buffer);
            let mut changed = false;
            while reader.next().is_ok() {
                changed = true;
            }
            if changed {
                on_change();
            }
            Ok(PostAction::Continue)
        })?;
        let weak = WeakHandle::new(handle);
        Ok(Watch { remove: Some(Box::new(move || weak.remove(token))) })
    }

    /// Watches a directory for written, moved, created and removed entries.
    pub fn directory<D: 'static>(
        handle: &LoopHandle<'static, D>,
        dir: &Path,
        mut on_change: impl FnMut(&WatchEvent) + 'static,
    ) -> Result<Watch, IoError> {
        let fd = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC)?;
        let mask = WatchFlags::CLOSE_WRITE
            | WatchFlags::MOVED_TO
            | WatchFlags::MOVED_FROM
            | WatchFlags::CREATE
            | WatchFlags::DELETE;
        inotify::add_watch(&fd, dir, mask)?;
        let source = Generic::new(fd, Interest::READ, Mode::Level);
        let token: RegistrationToken = handle.insert_source(source, move |_, fd, _| {
            drain(fd, &mut on_change);
            Ok(PostAction::Continue)
        })?;
        let weak = WeakHandle::new(handle);
        Ok(Watch { remove: Some(Box::new(move || weak.remove(token))) })
    }
}

fn drain(fd: &OwnedFd, on_change: &mut impl FnMut(&WatchEvent)) {
    let mut buffer = [MaybeUninit::<u8>::uninit(); 4096];
    let mut reader = inotify::Reader::new(fd, &mut buffer);
    while let Ok(event) = reader.next() {
        let Some(name) = event.file_name() else { continue };
        let removed = event.events().intersects(ReadFlags::DELETE | ReadFlags::MOVED_FROM);
        let name = OsString::from(String::from_utf8_lossy(name.to_bytes()).into_owned());
        on_change(&WatchEvent { name, removed });
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        if let Some(remove) = self.remove.take() {
            remove();
        }
    }
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watch")
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::time::Duration;

    use calloop::EventLoop;

    use super::*;
    use crate::fs::atomic_write;

    fn pump(event_loop: &mut EventLoop<'static, ()>) {
        for _ in 0..5 {
            event_loop.dispatch(Duration::from_millis(20), &mut ()).unwrap();
        }
    }

    #[test]
    fn reports_in_place_and_atomic_writes_of_the_target_only() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("settings.json");
        std::fs::write(&target, "a").unwrap();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let seen = Rc::new(RefCell::new(Vec::new()));
        let sink = seen.clone();
        let _watch = Watch::file(&event_loop.handle(), &target, move |e| sink.borrow_mut().push(e.clone())).unwrap();

        std::fs::write(dir.path().join("other.json"), "x").unwrap();
        pump(&mut event_loop);
        assert!(seen.borrow().iter().all(|e| e.name == "settings.json"), "{:?}", seen.borrow());
        seen.borrow_mut().clear();

        std::fs::write(&target, "b").unwrap();
        pump(&mut event_loop);
        assert!(!seen.borrow().is_empty());
        seen.borrow_mut().clear();

        atomic_write(&target, b"c").unwrap();
        pump(&mut event_loop);
        assert!(seen.borrow().iter().any(|e| !e.removed && e.name == "settings.json"));
    }

    #[test]
    fn dropping_the_watch_stops_events() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("f");
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let count = Rc::new(RefCell::new(0));
        let c = count.clone();
        let watch = Watch::file(&event_loop.handle(), &target, move |_| *c.borrow_mut() += 1).unwrap();
        drop(watch);
        std::fs::write(&target, "x").unwrap();
        pump(&mut event_loop);
        assert_eq!(*count.borrow(), 0);
    }

    #[test]
    fn modify_watch_fires_on_in_place_writes() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("brightness");
        std::fs::write(&target, "10").unwrap();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let count = Rc::new(RefCell::new(0));
        let c = count.clone();
        let _watch = Watch::modify(&event_loop.handle(), &target, move || *c.borrow_mut() += 1).unwrap();
        std::fs::write(&target, "20").unwrap();
        pump(&mut event_loop);
        assert!(*count.borrow() >= 1);
    }

    #[test]
    fn directory_watch_reports_new_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut event_loop: EventLoop<'static, ()> = EventLoop::try_new().unwrap();
        let names = Rc::new(RefCell::new(Vec::new()));
        let n = names.clone();
        let _watch =
            Watch::directory(&event_loop.handle(), dir.path(), move |e| n.borrow_mut().push(e.name.clone())).unwrap();
        std::fs::write(dir.path().join("en.json"), "{}").unwrap();
        pump(&mut event_loop);
        assert!(names.borrow().iter().any(|n| n == "en.json"));
    }
}
