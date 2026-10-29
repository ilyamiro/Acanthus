use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// Where an image comes from.
pub enum ImageSource {
    /// Nothing; the image stays null.
    None,
    /// A file path.
    File(PathBuf),
    /// A themed icon name.
    Icon(String),
    /// A registered in-memory image.
    Memory(u64),
}

impl ImageSource {
    /// Parses a QML-style source string: `file://` URLs, absolute paths, `image://icon/` names, bare icon names and memory ids.
    pub fn parse(source: &str) -> ImageSource {
        let source = source.trim();
        if source.is_empty() {
            return ImageSource::None;
        }
        if let Some(id) = crate::memory::parse(source) {
            return ImageSource::Memory(id);
        }
        if let Some(name) = source.strip_prefix("image://icon/") {
            let name = name.split('?').next().unwrap_or(name);
            return if name.is_empty() { ImageSource::None } else { ImageSource::Icon(name.to_owned()) };
        }
        let path = source.strip_prefix("file://").unwrap_or(source);
        let path = path.split('?').next().unwrap_or(path);
        if path.is_empty() {
            return ImageSource::None;
        }
        if path.starts_with('/') || source.starts_with("file://") {
            ImageSource::File(PathBuf::from(percent_decode(path)))
        } else {
            ImageSource::Icon(path.to_owned())
        }
    }

    /// Whether the image can be rendered sharply at any size: SVG files and themed icons.
    pub fn is_scalable(&self) -> bool {
        match self {
            ImageSource::File(path) => crate::decode::is_svg(path),
            ImageSource::Icon(_) => true,
            _ => false,
        }
    }
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes.get(i + 2).copied().unwrap_or(0)))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_owned())
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_forms_the_qml_uses() {
        assert_eq!(ImageSource::parse(""), ImageSource::None);
        assert_eq!(ImageSource::parse("file:///a/b.png?rev=3"), ImageSource::File("/a/b.png".into()));
        assert_eq!(ImageSource::parse("/a/b c.png"), ImageSource::File("/a/b c.png".into()));
        assert_eq!(ImageSource::parse("file:///a/b%20c.png"), ImageSource::File("/a/b c.png".into()));
        assert_eq!(ImageSource::parse("image://icon/firefox"), ImageSource::Icon("firefox".into()));
        assert_eq!(ImageSource::parse("firefox"), ImageSource::Icon("firefox".into()));
        assert_eq!(ImageSource::parse("image://icon/"), ImageSource::None);
        assert_eq!(ImageSource::parse("image://mem/00000000000000ff"), ImageSource::Memory(255));
    }
}
