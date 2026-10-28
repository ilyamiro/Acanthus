use std::cell::{Cell, RefCell};
use std::io::{self, ErrorKind, Read};
use std::rc::Rc;

use acanthus_core::Signal;
use acanthus_scene::ClipboardBackend;

const TEXT_MIMES: [&str; 5] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING", "TEXT", "STRING"];

pub(crate) const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn offered_mimes() -> impl Iterator<Item = &'static str> {
    TEXT_MIMES.into_iter()
}

pub(crate) fn is_own_offer(mimes: &[String]) -> bool {
    mimes.len() == TEXT_MIMES.len() && TEXT_MIMES.iter().all(|m| mimes.iter().any(|o| o == m))
}

pub(crate) fn pick_text_mime(offered: &[String]) -> Option<&'static str> {
    TEXT_MIMES.into_iter().find(|wanted| offered.iter().any(|m| m == wanted))
}

/// What the compositor currently holds as the clipboard selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    /// MIME types the owner offers.
    pub mime_types: Vec<String>,
    /// The text, once read; `None` while it is being read or when no text type is offered.
    pub text: Option<String>,
}

#[derive(Debug)]
pub(crate) enum Pump {
    Pending,
    Done(Vec<u8>),
    TooLarge,
}

#[derive(Debug, Default)]
pub(crate) struct ReadJob {
    buffer: Vec<u8>,
}

impl ReadJob {
    pub fn pump(&mut self, reader: &mut impl Read, limit: usize) -> io::Result<Pump> {
        let mut chunk = [0u8; 16 * 1024];
        match reader.read(&mut chunk) {
            Ok(0) => Ok(Pump::Done(std::mem::take(&mut self.buffer))),
            Ok(n) => {
                if self.buffer.len() + n > limit {
                    return Ok(Pump::TooLarge);
                }
                self.buffer.extend_from_slice(&chunk[..n]);
                Ok(Pump::Pending)
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => Ok(Pump::Pending),
            Err(e) => Err(e),
        }
    }
}

pub(crate) fn decode_text(bytes: Vec<u8>) -> String {
    match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    }
}

pub(crate) struct Shared {
    pub selection: Signal<Option<Selection>>,
    pub text: RefCell<String>,
    pub pending: RefCell<Option<String>>,
    pub generation: Cell<u64>,
    pub expect_own: Cell<bool>,
}

/// The desktop clipboard as seen through `ext-data-control-v1` or `zwlr_data_control_v1`, which work without keyboard focus. Without either protocol it behaves as an in-process clipboard.
#[derive(Clone)]
pub struct SystemClipboard {
    pub(crate) shared: Rc<Shared>,
}

impl SystemClipboard {
    pub(crate) fn new() -> Self {
        SystemClipboard {
            shared: Rc::new(Shared {
                selection: Signal::new(None),
                text: RefCell::new(String::new()),
                pending: RefCell::new(None),
                generation: Cell::new(0),
                expect_own: Cell::new(false),
            }),
        }
    }

    /// Signal of the current selection: its MIME types and its text once read.
    pub fn selection(&self) -> Signal<Option<Selection>> {
        self.shared.selection.clone()
    }

    /// The selection's text, empty when there is none.
    pub fn text(&self) -> String {
        self.shared.text.borrow().clone()
    }

    /// Becomes the selection owner offering `text` as plain text.
    pub fn set_text(&self, text: &str) {
        *self.shared.text.borrow_mut() = text.to_owned();
        *self.shared.pending.borrow_mut() = Some(text.to_owned());
        self.shared.generation.set(self.shared.generation.get() + 1);
        self.shared.selection.set(Some(Selection {
            mime_types: offered_mimes().map(str::to_owned).collect(),
            text: Some(text.to_owned()),
        }));
    }
}

impl ClipboardBackend for SystemClipboard {
    fn text(&self) -> String {
        SystemClipboard::text(self)
    }

    fn set_text(&self, text: &str) {
        SystemClipboard::set_text(self, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Script(Vec<io::Result<Vec<u8>>>);

    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Ok(0);
            }
            match self.0.remove(0) {
                Ok(bytes) => {
                    buf[..bytes.len()].copy_from_slice(&bytes);
                    Ok(bytes.len())
                }
                Err(e) => Err(e),
            }
        }
    }

    fn run(script: Vec<io::Result<Vec<u8>>>, limit: usize) -> Pump {
        let mut reader = Script(script);
        let mut job = ReadJob::default();
        loop {
            match job.pump(&mut reader, limit).unwrap() {
                Pump::Pending => continue,
                other => return other,
            }
        }
    }

    #[test]
    fn mime_preference_follows_the_table() {
        let offered = ["STRING".to_owned(), "text/plain".to_owned(), "image/png".to_owned()];
        assert_eq!(pick_text_mime(&offered), Some("text/plain"));
        assert_eq!(
            pick_text_mime(&["text/plain;charset=utf-8".to_owned(), "text/plain".to_owned()]),
            Some("text/plain;charset=utf-8")
        );
        assert_eq!(pick_text_mime(&["image/png".to_owned()]), None);
    }

    #[test]
    fn own_offer_is_recognised_by_its_mime_set() {
        let own: Vec<String> = offered_mimes().map(str::to_owned).collect();
        assert!(is_own_offer(&own));
        assert!(!is_own_offer(&own[..2]));
        assert!(!is_own_offer(&["text/plain".to_owned()]));
    }

    #[test]
    fn chunks_are_joined_until_eof() {
        let Pump::Done(bytes) =
            run(vec![Ok(b"he".to_vec()), Err(ErrorKind::WouldBlock.into()), Ok(b"llo".to_vec())], 100)
        else {
            panic!("expected done");
        };
        assert_eq!(bytes, b"hello");
    }

    #[test]
    fn oversize_data_is_dropped() {
        assert!(matches!(run(vec![Ok(vec![b'a'; 8]), Ok(vec![b'a'; 8])], 10), Pump::TooLarge));
    }

    #[test]
    fn read_errors_are_reported() {
        let mut reader = Script(vec![Err(ErrorKind::BrokenPipe.into())]);
        assert!(ReadJob::default().pump(&mut reader, 10).is_err());
    }

    #[test]
    fn invalid_utf8_is_repaired() {
        assert_eq!(decode_text(vec![b'a', 0xff, b'b']), "a\u{fffd}b");
        assert_eq!(decode_text("ünï".as_bytes().to_vec()), "ünï");
    }

    #[test]
    fn set_text_updates_cache_signal_and_queue() {
        let clipboard = SystemClipboard::new();
        clipboard.set_text("hi");
        assert_eq!(clipboard.text(), "hi");
        assert_eq!(clipboard.shared.pending.borrow().as_deref(), Some("hi"));
        let selection = clipboard.selection().get_untracked().unwrap();
        assert_eq!(selection.text.as_deref(), Some("hi"));
        assert!(selection.mime_types.iter().any(|m| m == "text/plain;charset=utf-8"));
    }

    #[test]
    fn backend_trait_reaches_the_cache() {
        let clipboard = SystemClipboard::new();
        let backend: &dyn ClipboardBackend = &clipboard;
        backend.set_text("x");
        assert_eq!(backend.text(), "x");
    }
}
