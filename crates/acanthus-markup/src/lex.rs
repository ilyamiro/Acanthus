use crate::{Diagnostic, Span};
use acanthus_core::Color;

/// What a token is.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    /// An identifier or keyword.
    Ident(String),
    /// A number literal.
    Number(f64),
    /// A string literal with escapes resolved.
    Text(String),
    /// A colour literal.
    Color(Color),
    /// Punctuation or an operator.
    Punct(&'static str),
    /// The end of the file.
    Eof,
}

/// A token with its place and whether a newline precedes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    /// The kind and value.
    pub kind: TokenKind,
    /// Where it is in the source.
    pub span: Span,
    /// True when a line break lies between the previous token and this one.
    pub newline: bool,
}

const PUNCT: [&str; 25] = [
    "<=", ">=", "==", "!=", "&&", "||", "{", "}", "(", ")", "[", "]", ":", ";", ",", ".", "?", "!", "+", "-", "*", "/",
    "%", "<", ">",
];

/// Splits `src` into tokens; the last one is always [`TokenKind::Eof`].
pub fn lex(src: &str) -> Result<Vec<Token>, Diagnostic> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut newline = false;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\n' => {
                newline = true;
                i += 1;
            }
            _ if c.is_ascii_whitespace() => i += 1,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..]
                    .find("*/")
                    .ok_or_else(|| Diagnostic::new(Span { start: i, end: i + 2 }, "unterminated block comment"))?;
                newline |= src[i..i + 2 + end].contains('\n');
                i += end + 4;
            }
            _ => {
                let start = i;
                let kind = if c.is_ascii_alphabetic() || c == b'_' {
                    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    TokenKind::Ident(src[start..i].to_string())
                } else if c.is_ascii_digit() || (c == b'.' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit)) {
                    i = number_end(bytes, i);
                    let value =
                        src[start..i].parse().map_err(|_| Diagnostic::new(Span { start, end: i }, "invalid number"))?;
                    TokenKind::Number(value)
                } else if c == b'"' {
                    let (text, end) = string(src, i)?;
                    i = end;
                    TokenKind::Text(text)
                } else if c == b'#' {
                    i += 1;
                    while i < bytes.len() && bytes[i].is_ascii_hexdigit() {
                        i += 1;
                    }
                    TokenKind::Color(colour(&src[start + 1..i]).ok_or_else(|| {
                        Diagnostic::new(Span { start, end: i }, "a colour has 3, 6 or 8 hex digits after `#`")
                    })?)
                } else if let Some(p) = PUNCT.iter().find(|p| src[i..].starts_with(**p)) {
                    i += p.len();
                    TokenKind::Punct(p)
                } else {
                    let ch = src[i..].chars().next().unwrap_or(' ');
                    return Err(Diagnostic::new(
                        Span { start, end: start + ch.len_utf8() },
                        format!("unexpected character `{ch}`"),
                    ));
                };
                tokens.push(Token { kind, span: Span { start, end: i }, newline });
                newline = false;
            }
        }
    }
    tokens.push(Token { kind: TokenKind::Eof, span: Span { start: src.len(), end: src.len() }, newline });
    Ok(tokens)
}

fn number_end(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if bytes.get(i) == Some(&b'.') && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(bytes.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if bytes.get(j).is_some_and(u8::is_ascii_digit) {
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    i
}

fn string(src: &str, start: usize) -> Result<(String, usize), Diagnostic> {
    let mut out = String::new();
    let mut chars = src[start + 1..].char_indices();
    while let Some((offset, c)) = chars.next() {
        let at = start + 1 + offset;
        match c {
            '"' => return Ok((out, at + 1)),
            '\n' => break,
            '\\' => {
                let (_, e) =
                    chars.next().ok_or_else(|| Diagnostic::new(Span { start, end: at + 1 }, "unterminated string"))?;
                match e {
                    '"' | '\\' => out.push(e),
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'u' => {
                        let rest = &src[at + 2..];
                        let close = rest.strip_prefix('{').and_then(|r| r.find('}')).ok_or_else(|| {
                            Diagnostic::new(Span { start: at, end: at + 2 }, "expected `{hex}` after `\\u`")
                        })?;
                        let code = u32::from_str_radix(&rest[1..=close], 16).ok().and_then(char::from_u32).ok_or_else(
                            || Diagnostic::new(Span { start: at, end: at + close + 4 }, "invalid unicode escape"),
                        )?;
                        out.push(code);
                        for _ in 0..close + 2 {
                            chars.next();
                        }
                    }
                    _ => {
                        return Err(Diagnostic::new(
                            Span { start: at, end: at + 1 + e.len_utf8() },
                            format!("unknown escape `\\{e}`"),
                        ));
                    }
                }
            }
            _ => out.push(c),
        }
    }
    Err(Diagnostic::new(Span { start, end: start + 1 }, "unterminated string"))
}

fn colour(hex: &str) -> Option<Color> {
    let v = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        3 => Some(Color::rgb8(((v >> 8) & 0xf) as u8 * 17, ((v >> 4) & 0xf) as u8 * 17, (v & 0xf) as u8 * 17)),
        6 => Some(Color::hex(v)),
        8 => Some(Color::hexa(v)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn tokens_and_literals() {
        assert_eq!(
            kinds("a: 1.5e1 + .5 /* x */\n\"q\\n\\u{41}\" #fff"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Punct(":"),
                TokenKind::Number(15.0),
                TokenKind::Punct("+"),
                TokenKind::Number(0.5),
                TokenKind::Text("q\nA".into()),
                TokenKind::Color(Color::rgb8(255, 255, 255)),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn operators_prefer_the_longest_match_and_newlines_are_flagged() {
        let t = lex("a <= b\n&& c /* x\ny */ d").unwrap();
        assert_eq!(t[1].kind, TokenKind::Punct("<="));
        assert!(t[3].newline && !t[2].newline);
        assert!(t[5].newline);
    }

    #[test]
    fn colours_with_alpha() {
        assert_eq!(kinds("#11223344")[0], TokenKind::Color(Color::hexa(0x1122_3344)));
        assert!(lex("#12").is_err());
    }

    #[test]
    fn errors_point_at_the_problem() {
        let src = "a: \"open\nb: 1";
        let d = lex(src).unwrap_err();
        assert_eq!(d.message, "unterminated string");
        assert_eq!(d.position(src), (1, 4));
        let d = lex("a: $").unwrap_err();
        assert!(d.message.contains('$'));
        assert!(lex("/* open").is_err());
        assert!(lex("\"\\q\"").unwrap_err().message.contains("\\q"));
    }

    #[test]
    fn rendering_shows_the_line_and_a_caret_with_the_hint() {
        let src = "Rect {\n    colr: 1\n}";
        let d = Diagnostic::unknown(Span { start: 11, end: 15 }, "property", "colr", ["color", "width"]);
        let out = d.render("a.acn", src);
        assert!(out.contains("--> a.acn:2:5"));
        assert!(out.contains("2 |     colr: 1"));
        assert!(out.contains("^^^^ did you mean `color`?"));
    }
}
