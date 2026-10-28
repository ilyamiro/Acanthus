use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use acanthus_io::ExitInfo;

use super::exec::run_blocking;
use super::{Cmd, Exec, Running};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const CURL_TIMEOUT_EXIT: i32 = 28;

/// An HTTP GET request executed through `curl`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The URL.
    pub url: String,
    /// Overall timeout.
    pub timeout: Duration,
    /// Extra headers as name and value.
    pub headers: Vec<(String, String)>,
    /// User agent, or `None` for curl's own.
    pub user_agent: Option<String>,
}

impl Request {
    /// A request for `url` with a ten-second timeout.
    pub fn new(url: impl Into<String>) -> Request {
        Request { url: url.into(), timeout: DEFAULT_TIMEOUT, headers: Vec::new(), user_agent: None }
    }

    /// Sets the timeout.
    pub fn timeout(mut self, timeout: Duration) -> Request {
        self.timeout = timeout;
        self
    }

    /// Sets the timeout in seconds.
    pub fn timeout_secs(self, seconds: u64) -> Request {
        self.timeout(Duration::from_secs(seconds))
    }

    /// Adds a header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Request {
        self.headers.push((name.into(), value.into()));
        self
    }

    /// Adds several headers.
    pub fn headers(mut self, headers: &[(&str, &str)]) -> Request {
        self.headers.extend(headers.iter().map(|(name, value)| ((*name).to_owned(), (*value).to_owned())));
        self
    }

    /// Sets the user agent.
    pub fn user_agent(mut self, agent: impl Into<String>) -> Request {
        self.user_agent = Some(agent.into());
        self
    }
}

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// The helper process could not be started.
    Spawn,
    /// The request timed out or was killed.
    Timeout,
    /// The helper exited with this non-zero code.
    Exit(i32),
    /// The server answered with this HTTP status.
    Status(u16),
    /// The response was empty.
    Empty,
}

fn base_argv(request: &Request, flags: &[&str]) -> Vec<String> {
    let mut argv: Vec<String> = ["curl"].iter().chain(flags).map(|part| (*part).to_owned()).collect();
    argv.push("--max-time".to_owned());
    argv.push(request.timeout.as_secs().max(1).to_string());
    if let Some(agent) = &request.user_agent {
        argv.push("-A".to_owned());
        argv.push(agent.clone());
    }
    for (name, value) in &request.headers {
        argv.push("-H".to_owned());
        argv.push(format!("{name}: {value}"));
    }
    argv
}

/// The command line that fetches `request` and prints the body.
pub fn get_argv(request: &Request) -> Vec<String> {
    let mut argv = base_argv(request, &["-sfL"]);
    argv.push(request.url.clone());
    argv
}

/// The command line that saves the response of `request` to `dest` and prints the HTTP status.
pub fn download_argv(request: &Request, dest: &Path) -> Vec<String> {
    let mut argv = base_argv(request, &["-s", "-L"]);
    argv.extend(["-o".to_owned(), dest.to_string_lossy().into_owned()]);
    argv.extend(["-w".to_owned(), "%{http_code}".to_owned(), request.url.clone()]);
    argv
}

fn classify_get(info: ExitInfo) -> Result<String, HttpError> {
    match info.code {
        Some(0) if info.stdout.is_empty() => Err(HttpError::Empty),
        Some(0) => Ok(info.stdout),
        Some(CURL_TIMEOUT_EXIT) => Err(HttpError::Timeout),
        Some(code) => Err(HttpError::Exit(code)),
        None if info.signal.is_some() => Err(HttpError::Timeout),
        None => Err(HttpError::Spawn),
    }
}

fn classify_download(info: ExitInfo) -> Result<u16, HttpError> {
    match info.stdout.trim().parse::<u16>() {
        Ok(status) => Ok(status),
        Err(_) if info.code == Some(CURL_TIMEOUT_EXIT) => Err(HttpError::Timeout),
        Err(_) => Err(HttpError::Spawn),
    }
}

/// Downloads `request` to `dest` synchronously and returns the HTTP status.
pub fn download_blocking(request: &Request, dest: &Path) -> Result<u16, HttpError> {
    let info = run_blocking(&Cmd::new(download_argv(request, dest))).ok_or(HttpError::Spawn)?;
    classify_download(info)
}

pub type Parser<T> = Rc<dyn Fn(&str) -> Option<T>>;

/// One place to try in [`Http::first_of`]: a request and the parser of its body.
pub struct Source<T> {
    /// The request.
    pub request: Request,
    /// Turns the body into a value, `None` when it is not usable.
    pub parse: Parser<T>,
}

/// Runs HTTP requests through an [`Exec`], so tests can fake them.
#[derive(Clone)]
pub struct Http {
    exec: Rc<dyn Exec>,
}

impl Http {
    /// Creates a client running requests with `exec`.
    pub fn new(exec: Rc<dyn Exec>) -> Http {
        Http { exec }
    }

    /// Fetches the body of `request` and calls `done`; the returned handle cancels it.
    pub fn get(
        &self,
        request: Request,
        done: impl FnOnce(Result<String, HttpError>) + 'static,
    ) -> Option<Box<dyn Running>> {
        self.exec.run(Cmd::new(get_argv(&request)), Box::new(move |info| done(classify_get(info))))
    }

    /// Downloads `request` to `dest` and calls `done` with the HTTP status; the returned handle cancels it.
    pub fn download(
        &self,
        request: Request,
        dest: &Path,
        done: impl FnOnce(Result<u16, HttpError>) + 'static,
    ) -> Option<Box<dyn Running>> {
        self.exec.run(Cmd::new(download_argv(&request, dest)), Box::new(move |info| done(classify_download(info))))
    }

    /// Tries `sources` in order and calls `done` with the first parsed value, or `None` when every source fails.
    pub fn first_of<T: 'static>(&self, sources: Vec<Source<T>>, done: impl FnOnce(Option<T>) + 'static) {
        fn step<T: 'static>(http: Http, mut sources: std::vec::IntoIter<Source<T>>, done: Box<dyn FnOnce(Option<T>)>) {
            let Some(source) = sources.next() else {
                done(None);
                return;
            };
            let next = http.clone();
            http.get(source.request, move |body| match body.ok().and_then(|text| (source.parse)(&text)) {
                Some(value) => done(Some(value)),
                None => step(next, sources, done),
            });
        }
        step(self.clone(), sources.into_iter(), Box::new(done));
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::kit::testing::FakeExec;

    fn http() -> (Http, Rc<FakeExec>) {
        let fake = FakeExec::immediate();
        (Http::new(fake.exec()), fake)
    }

    #[test]
    fn get_puts_the_flags_the_agent_the_headers_and_the_url_in_order() {
        let request =
            Request::new("https://x.test/a").timeout_secs(5).user_agent("agent").header("Referer", "https://r");
        assert_eq!(
            get_argv(&request),
            ["curl", "-sfL", "--max-time", "5", "-A", "agent", "-H", "Referer: https://r", "https://x.test/a"]
        );
    }

    #[test]
    fn download_asks_curl_for_the_status_code() {
        let request = Request::new("https://x.test/a.jpg").timeout_secs(4);
        assert_eq!(
            download_argv(&request, Path::new("/tmp/a")),
            ["curl", "-s", "-L", "--max-time", "4", "-o", "/tmp/a", "-w", "%{http_code}", "https://x.test/a.jpg"]
        );
    }

    #[test]
    fn get_returns_the_body_and_classifies_failures() {
        let (http, fake) = http();
        let seen = Rc::new(RefCell::new(Vec::new()));
        fake.reply_ok("body");
        fake.reply_ok("");
        fake.reply_fail();
        fake.reply(ExitInfo { code: Some(28), ..Default::default() });
        fake.reply(ExitInfo { code: None, ..Default::default() });
        for _ in 0..5 {
            let sink = seen.clone();
            http.get(Request::new("https://x.test"), move |result| sink.borrow_mut().push(result));
        }
        assert_eq!(
            *seen.borrow(),
            [
                Ok("body".to_owned()),
                Err(HttpError::Empty),
                Err(HttpError::Exit(22)),
                Err(HttpError::Timeout),
                Err(HttpError::Spawn)
            ]
        );
    }

    #[test]
    fn download_reports_the_status_code_even_for_errors() {
        let (http, fake) = http();
        fake.reply_ok("200");
        fake.reply_ok("404\n");
        fake.reply(ExitInfo { code: None, ..Default::default() });
        let seen = Rc::new(RefCell::new(Vec::new()));
        for _ in 0..3 {
            let sink = seen.clone();
            http.download(Request::new("https://x.test/a"), Path::new("/tmp/a"), move |r| sink.borrow_mut().push(r));
        }
        assert_eq!(*seen.borrow(), [Ok(200), Ok(404), Err(HttpError::Spawn)]);
    }

    #[test]
    fn a_held_request_can_be_killed_through_its_handle() {
        let fake = FakeExec::new();
        let http = Http::new(fake.exec());
        let handle = http.get(Request::new("https://x.test"), |_| {}).unwrap();
        handle.kill();
        assert_eq!(fake.killed(), 1);
    }

    fn sources(count: u32) -> Vec<Source<u32>> {
        let parse: Parser<u32> = Rc::new(|text| text.parse().ok());
        (1..=count)
            .map(|n| Source { request: Request::new(format!("https://x.test/{n}")), parse: parse.clone() })
            .collect()
    }

    #[test]
    fn first_of_returns_the_first_parsable_body_and_skips_failures() {
        let (http, fake) = http();
        fake.reply_fail();
        fake.reply_ok("junk");
        fake.reply_ok("42");
        fake.reply_ok("99");
        let got = Rc::new(RefCell::new(None));
        let sink = got.clone();
        http.first_of(sources(4), move |value| *sink.borrow_mut() = Some(value));
        assert_eq!(*got.borrow(), Some(Some(42)));
        assert_eq!(fake.urls(), ["https://x.test/1", "https://x.test/2", "https://x.test/3"]);
    }

    #[test]
    fn first_of_reports_none_when_every_source_fails_or_there_are_none() {
        let (http, fake) = http();
        let got = Rc::new(RefCell::new(None));
        let sink = got.clone();
        http.first_of(sources(2), move |value| *sink.borrow_mut() = Some(value));
        assert_eq!(*got.borrow(), Some(None));
        assert_eq!(fake.call_count(), 2);
        let empty = Rc::new(RefCell::new(None));
        let sink = empty.clone();
        http.first_of::<u32>(Vec::new(), move |value| *sink.borrow_mut() = Some(value));
        assert_eq!(*empty.borrow(), Some(None));
    }

    #[test]
    fn the_blocking_download_runs_curl_for_real() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src.bin");
        std::fs::write(&source, b"payload").unwrap();
        let dest = dir.path().join("dest.bin");
        let request = Request::new(format!("file://{}", source.display())).timeout_secs(4);
        match download_blocking(&request, &dest) {
            Ok(_) => assert_eq!(std::fs::read(&dest).unwrap(), b"payload"),
            Err(HttpError::Spawn) => eprintln!("curl is not installed, skipping"),
            Err(other) => panic!("{other:?}"),
        }
    }
}
