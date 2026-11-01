use crate::expr::parse_at;
use crate::{Diagnostic, Expr, Span, Token, TokenKind, lex};
use acanthus_widgets::component::Registry;

/// The newest syntax version this reader understands.
pub const VERSION: u32 = 0;

/// A parsed `.acn` file.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// The syntax version the file declares (`acn N`), or [`VERSION`] when it declares none.
    pub version: u32,
    /// The root component.
    pub root: Component,
}

/// A component with its members.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    /// The component name, such as `Slider`.
    pub name: String,
    /// Where the name is.
    pub span: Span,
    /// Everything between the braces, in source order.
    pub members: Vec<Member>,
}

/// One entry in a component body.
#[derive(Debug, Clone, PartialEq)]
pub enum Member {
    /// `id: name`.
    Id(String, Span),
    /// `property name: expr`, a property of the enclosing component.
    Property(String, Span, Expr),
    /// `name: expr`, a property assignment.
    Set(String, Span, Expr),
    /// `on_event: expr`, an event handler.
    Handler(String, Span, Expr),
    /// `Behavior on name { Component }`.
    Behavior(String, Span, Component),
    /// A nested component.
    Child(Component),
}

const KEYWORDS: [&str; 3] = ["id", "property", "Behavior"];

/// Parses an ACN file.
pub fn parse(src: &str) -> Result<Document, Diagnostic> {
    let tokens = lex(src)?;
    let mut p = Parser { tokens: &tokens, pos: 0 };
    let mut version = VERSION;
    if p.word("acn") {
        let t = p.next();
        let TokenKind::Number(n) = t.kind else {
            return Err(Diagnostic::new(t.span, "expected the version number after `acn`"));
        };
        if n.fract() != 0.0 || n < 0.0 || n as u32 > VERSION {
            return Err(Diagnostic::new(
                t.span,
                format!("this reader supports acn {VERSION}, the file declares acn {n}"),
            ));
        }
        version = n as u32;
    }
    let root = p.component()?;
    match p.peek() {
        Token { kind: TokenKind::Eof, .. } => Ok(Document { version, root }),
        t => Err(Diagnostic::new(t.span, "a file holds one root component")),
    }
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl Parser<'_> {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn at(&self, offset: usize) -> &Token {
        &self.tokens[(self.pos + offset).min(self.tokens.len() - 1)]
    }

    fn next(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if t.kind != TokenKind::Eof {
            self.pos += 1;
        }
        t
    }

    fn word(&mut self, w: &str) -> bool {
        let hit = matches!(&self.peek().kind, TokenKind::Ident(i) if i == w);
        if hit {
            self.pos += 1;
        }
        hit
    }

    fn is_punct(&self, offset: usize, p: &str) -> bool {
        matches!(&self.at(offset).kind, TokenKind::Punct(q) if *q == p)
    }

    fn punct(&mut self, p: &str) -> Result<Span, Diagnostic> {
        let t = self.peek();
        if matches!(&t.kind, TokenKind::Punct(q) if *q == p) {
            Ok(self.next().span)
        } else {
            Err(Diagnostic::new(t.span, format!("expected `{p}`")))
        }
    }

    fn ident(&mut self, what: &str) -> Result<(String, Span), Diagnostic> {
        let t = self.next();
        match t.kind {
            TokenKind::Ident(name) => Ok((name, t.span)),
            _ => Err(Diagnostic::new(t.span, format!("expected {what}"))),
        }
    }

    fn component(&mut self) -> Result<Component, Diagnostic> {
        let (name, span) = self.ident("a component name")?;
        if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            return Err(Diagnostic::new(
                span,
                format!("a component name starts with a capital letter, `{name}` does not"),
            ));
        }
        self.punct("{")?;
        let mut members = Vec::new();
        loop {
            while self.is_punct(0, ";") {
                self.pos += 1;
            }
            if self.is_punct(0, "}") {
                self.pos += 1;
                return Ok(Component { name, span, members });
            }
            if self.peek().kind == TokenKind::Eof {
                return Err(Diagnostic::new(span, format!("`{name}` is never closed")));
            }
            members.push(self.member()?);
        }
    }

    fn value(&mut self) -> Result<Expr, Diagnostic> {
        let e = parse_at(self.tokens, &mut self.pos)?;
        let t = self.peek();
        if t.newline || matches!(&t.kind, TokenKind::Punct(";" | "}") | TokenKind::Eof) {
            Ok(e)
        } else {
            Err(Diagnostic::new(t.span, "expected a new line or `;` after the value"))
        }
    }

    fn member(&mut self) -> Result<Member, Diagnostic> {
        let t = self.peek().clone();
        let TokenKind::Ident(name) = &t.kind else {
            return Err(Diagnostic::new(t.span, "expected a property or a component"));
        };
        let upper = name.starts_with(|c: char| c.is_ascii_uppercase());
        if name == "Behavior" && matches!(&self.at(1).kind, TokenKind::Ident(w) if w == "on") {
            self.pos += 2;
            let (prop, span) = self.ident("a property name")?;
            self.punct("{")?;
            let body = self.component()?;
            self.punct("}")?;
            return Ok(Member::Behavior(prop, span, body));
        }
        if name == "property" && matches!(self.at(1).kind, TokenKind::Ident(_)) && self.is_punct(2, ":") {
            self.pos += 1;
            let (prop, span) = self.ident("a property name")?;
            self.pos += 1;
            return Ok(Member::Property(prop, span, self.value()?));
        }
        if name == "id" && self.is_punct(1, ":") {
            self.pos += 2;
            let (id, span) = self.ident("a name")?;
            return Ok(Member::Id(id, span));
        }
        if upper && self.is_punct(1, "{") {
            return self.component().map(Member::Child);
        }
        if !upper && self.is_punct(1, ":") {
            self.pos += 2;
            let e = self.value()?;
            let handler = name.strip_prefix("on_").is_some_and(|rest| !rest.is_empty());
            return Ok(if handler {
                Member::Handler(name.clone(), t.span, e)
            } else {
                Member::Set(name.clone(), t.span, e)
            });
        }
        let mut d = Diagnostic::new(t.span, format!("expected `:` or `{{` after `{name}`"));
        d.hint = acanthus_widgets::component::nearest(name, KEYWORDS).map(|k| format!("did you mean `{k}`?"));
        Err(d)
    }
}

impl Document {
    /// Checks every component and property name against `registry`, collecting all errors.
    /// A name that does not exist gets the nearest known one as a hint.
    pub fn check_names(&self, registry: &Registry) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        check_component(&self.root, registry, &mut out);
        out
    }
}

fn check_component(c: &Component, registry: &Registry, out: &mut Vec<Diagnostic>) {
    let Some(component) = registry.get(&c.name) else {
        out.push(Diagnostic::unknown(c.span, "component", &c.name, registry.names()));
        return;
    };
    let declared: Vec<&str> = c
        .members
        .iter()
        .filter_map(|m| if let Member::Property(n, ..) = m { Some(n.as_str()) } else { None })
        .collect();
    let known = || component.props().iter().map(|p| p.name).chain(declared.iter().copied());
    for m in &c.members {
        match m {
            Member::Set(n, s, _) | Member::Handler(n, s, _) if !known().any(|k| k == n) => {
                out.push(Diagnostic::unknown(*s, "property", n, known()));
            }
            Member::Child(child) => check_component(child, registry, out),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "acn 0
Dropdown {
    id: menu
    property open: false
    width: 120 + 8; height: 30
    on_select: action(\"go\", index)
    Behavior on opacity { Tween { duration: 200 } }
    Slider { value: 0.5 }
}";

    #[test]
    fn parses_every_member_kind() {
        let doc = parse(SRC).unwrap();
        assert_eq!((doc.version, doc.root.name.as_str()), (0, "Dropdown"));
        let m = &doc.root.members;
        assert!(matches!(&m[0], Member::Id(n, _) if n == "menu"));
        assert!(matches!(&m[1], Member::Property(n, ..) if n == "open"));
        assert!(matches!(&m[2], Member::Set(n, ..) if n == "width"));
        assert!(matches!(&m[3], Member::Set(n, ..) if n == "height"));
        assert!(matches!(&m[4], Member::Handler(n, ..) if n == "on_select"));
        assert!(matches!(&m[5], Member::Behavior(n, _, b) if n == "opacity" && b.name == "Tween"));
        assert!(matches!(&m[6], Member::Child(c) if c.name == "Slider"));
    }

    #[test]
    fn version_is_optional_and_checked() {
        assert_eq!(parse("Rect {}").unwrap().version, 0);
        assert!(parse("acn 1\nRect {}").unwrap_err().message.contains("supports acn 0"));
        assert!(parse("acn\nRect {}").is_err());
    }

    #[test]
    fn errors_have_places_and_keyword_hints() {
        let src = "Rect {\n    propert x: 1\n}";
        let d = parse(src).unwrap_err();
        assert_eq!(d.position(src), (2, 5));
        assert_eq!(d.hint.as_deref(), Some("did you mean `property`?"));
        assert!(parse("Rect {").unwrap_err().message.contains("never closed"));
        assert!(parse("rect {}").unwrap_err().message.contains("capital"));
        assert!(parse("Rect { a: 1 b: 2 }").unwrap_err().message.contains("new line or `;`"));
        assert!(parse("Rect {} Rect {}").unwrap_err().message.contains("one root"));
        assert!(parse("Rect { a: 1 +\n }").is_err());
        assert!(parse("Rect { 5 }").is_err());
    }

    #[test]
    fn names_are_checked_against_the_registry() {
        let registry = registry();
        let src = "Slidr { value: 1 }";
        let d = parse(src).unwrap().check_names(&registry);
        assert_eq!(d[0].hint.as_deref(), Some("did you mean `Slider`?"));
        let src = "Slider {\n  property mine: 1\n  valu: 1\n  mine: 2\n  on_moved: f(1)\n}";
        let d = parse(src).unwrap().check_names(&registry);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].hint.as_deref(), Some("did you mean `value`?"));
        assert_eq!(d[0].position(src), (3, 3));
        assert!(parse("Slider { value: 1 }").unwrap().check_names(&registry).is_empty());
    }

    pub(super) fn registry() -> Registry {
        acanthus_widgets::standard_registry()
    }
}
