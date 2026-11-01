use acanthus_widgets::component::nearest;

/// A byte range of the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Span {
    /// First byte.
    pub start: usize,
    /// One past the last byte.
    pub end: usize,
}

impl Span {
    /// The smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span { start: self.start.min(other.start), end: self.end.max(other.end) }
    }
}

/// An error in an ACN file with its place and an optional suggestion.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    /// Where the problem is.
    pub span: Span,
    /// What is wrong.
    pub message: String,
    /// A suggestion shown after the caret, such as `did you mean `color`?`.
    pub hint: Option<String>,
}

impl Diagnostic {
    /// An error at `span`.
    pub fn new(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic { span, message: message.into(), hint: None }
    }

    /// An error about an unknown `word` of the given `what` (such as `property`), suggesting the
    /// nearest of `known` when one is close enough.
    pub fn unknown<'a>(span: Span, what: &str, word: &str, known: impl IntoIterator<Item = &'a str>) -> Diagnostic {
        let mut d = Diagnostic::new(span, format!("unknown {what} `{word}`"));
        d.hint = nearest(word, known).map(|n| format!("did you mean `{n}`?"));
        d
    }

    /// One-based line and column of the start of the span.
    pub fn position(&self, src: &str) -> (usize, usize) {
        let upto = &src[..self.span.start.min(src.len())];
        let line = upto.matches('\n').count() + 1;
        let col = upto.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        (line, col)
    }

    /// The error as text: `file:line:col`, the source line and a caret line.
    pub fn render(&self, file: &str, src: &str) -> String {
        let (line, col) = self.position(src);
        let text = src.lines().nth(line - 1).unwrap_or("");
        let width = src[self.span.start.min(src.len())..self.span.end.min(src.len())]
            .chars()
            .count()
            .clamp(1, text.chars().count().saturating_sub(col - 1).max(1));
        let gutter = line.to_string();
        let pad = " ".repeat(gutter.len());
        let mut out =
            format!("error: {}\n{pad}--> {file}:{line}:{col}\n{pad} |\n{gutter} | {text}\n{pad} | ", self.message);
        out.push_str(&" ".repeat(col - 1));
        out.push_str(&"^".repeat(width));
        if let Some(hint) = &self.hint {
            out.push(' ');
            out.push_str(hint);
        }
        out.push('\n');
        out
    }
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}
