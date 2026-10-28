use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub struct LineReader {
    reader: BufReader<UnixStream>,
    pending: String,
}

impl LineReader {
    pub fn new(stream: UnixStream, poll: Duration) -> std::io::Result<LineReader> {
        stream.set_read_timeout(Some(poll))?;
        Ok(LineReader { reader: BufReader::new(stream), pending: String::new() })
    }

    pub fn next_line(&mut self) -> std::io::Result<Option<String>> {
        match self.reader.read_line(&mut self.pending) {
            Ok(0) => Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof)),
            Ok(_) if self.pending.ends_with('\n') => {
                let line = std::mem::take(&mut self.pending);
                Ok(Some(line.trim_end().to_owned()))
            }
            Ok(_) => Ok(None),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    use super::*;

    #[test]
    fn partial_lines_survive_read_timeouts() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        let mut lines = LineReader::new(reader, Duration::from_millis(20)).unwrap();
        assert_eq!(lines.next_line().unwrap(), None);
        writer.write_all(b"hel").unwrap();
        assert_eq!(lines.next_line().unwrap(), None);
        writer.write_all(b"lo\nsecond\n").unwrap();
        assert_eq!(lines.next_line().unwrap().as_deref(), Some("hello"));
        assert_eq!(lines.next_line().unwrap().as_deref(), Some("second"));
        drop(writer);
        assert!(lines.next_line().is_err());
    }
}
