use crate::{Diagnostic, Span, Token, TokenKind, lex};
use acanthus_core::Color;
use std::collections::BTreeMap;
use std::ops::RangeInclusive;

/// A value of the expression language.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// Every number is an `f64`.
    Number(f64),
    /// A text.
    Text(String),
    /// A colour.
    Color(Color),
    /// A list.
    List(Vec<Value>),
}

impl Value {
    fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::Text(_) => "text",
            Value::Color(_) => "color",
            Value::List(_) => "list",
        }
    }
}

/// A unary or binary operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `-x`.
    Neg,
    /// `!x`.
    Not,
    /// `+`.
    Add,
    /// `-`.
    Sub,
    /// `*`.
    Mul,
    /// `/`.
    Div,
    /// `%`.
    Rem,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `==`.
    Eq,
    /// `!=`.
    Ne,
    /// `&&`.
    And,
    /// `||`.
    Or,
}

/// A node of an expression with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    /// Where the expression is in the source.
    pub span: Span,
    /// What it is.
    pub kind: ExprKind,
}

/// The shapes of an expression.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// A literal.
    Lit(Value),
    /// A dotted path such as `theme.base` or `slider.value`.
    Path(Vec<String>),
    /// A list literal.
    List(Vec<Expr>),
    /// A unary operation.
    Unary(Op, Box<Expr>),
    /// A binary operation.
    Binary(Op, Box<Expr>, Box<Expr>),
    /// `cond ? a : b`.
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// A call of a whitelisted function.
    Call(String, Span, Vec<Expr>),
    /// `list[index]`.
    Index(Box<Expr>, Box<Expr>),
}

/// The names an expression can read: ids, properties and host globals.
pub trait Scope {
    /// The first segments of every path that can resolve.
    fn roots(&self) -> Vec<String>;
    /// The current value at `path`, or `None` when it does not exist.
    fn get(&self, path: &[String]) -> Option<Value>;
}

type Function = Box<dyn Fn(&[Value]) -> Result<Value, String>>;
type LiteralCheck = Box<dyn Fn(&str) -> Option<String>>;

/// The whitelist of functions expressions may call.
pub struct Functions {
    table: BTreeMap<String, (RangeInclusive<usize>, Function)>,
    literals: BTreeMap<String, LiteralCheck>,
}

impl Functions {
    /// An empty whitelist.
    pub fn new() -> Functions {
        Functions { table: BTreeMap::new(), literals: BTreeMap::new() }
    }

    /// The built-ins `min max clamp abs round floor ceil mix len fmt_time`.
    pub fn standard() -> Functions {
        let mut f = Functions::new();
        f.register("min", 1..=usize::MAX, |a| fold(a, f64::min));
        f.register("max", 1..=usize::MAX, |a| fold(a, f64::max));
        f.register("clamp", 3..=3, |a| match (&a[0], &a[1], &a[2]) {
            (Value::Number(v), Value::Number(lo), Value::Number(hi)) => Ok(Value::Number(v.max(*lo).min(*hi))),
            _ => Err("clamp takes three numbers".into()),
        });
        for (name, op) in
            [("abs", f64::abs as fn(f64) -> f64), ("round", f64::round), ("floor", f64::floor), ("ceil", f64::ceil)]
        {
            f.register(name, 1..=1, move |a| match &a[0] {
                Value::Number(v) => Ok(Value::Number(op(*v))),
                other => Err(format!("{name} takes a number, not {}", other.type_name())),
            });
        }
        f.register("mix", 3..=3, |a| match (&a[0], &a[1], &a[2]) {
            (Value::Number(x), Value::Number(y), Value::Number(t)) => Ok(Value::Number(x + (y - x) * t)),
            (Value::Color(x), Value::Color(y), Value::Number(t)) => {
                let t = *t as f32;
                let l = |p: f32, q: f32| p + (q - p) * t;
                Ok(Value::Color(Color::rgba(l(x.r, y.r), l(x.g, y.g), l(x.b, y.b), l(x.a, y.a))))
            }
            _ => Err("mix takes two numbers or two colours and a number".into()),
        });
        f.register("fmt_time", 1..=1, |a| match &a[0] {
            Value::Number(v) if *v >= 0.0 => {
                let s = *v as u64;
                Ok(Value::Text(if s >= 3600 {
                    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
                } else {
                    format!("{}:{:02}", s / 60, s % 60)
                }))
            }
            other => Err(format!("fmt_time takes seconds, not {}", other.type_name())),
        });
        f.register("len", 1..=1, |a| match &a[0] {
            Value::List(l) => Ok(Value::Number(l.len() as f64)),
            Value::Text(t) => Ok(Value::Number(t.chars().count() as f64)),
            other => Err(format!("len takes a list or text, not {}", other.type_name())),
        });
        f
    }

    /// Checks the first argument of `name` at compile time when it is a text literal: `check` returns the error message for an unacceptable one (a missing translation key, an unknown action).
    pub fn check_literal(&mut self, name: &str, check: impl Fn(&str) -> Option<String> + 'static) {
        self.literals.insert(name.to_string(), Box::new(check));
    }

    /// Adds a function accepting `arity` arguments; a later call with the same name replaces it.
    pub fn register(
        &mut self,
        name: &str,
        arity: RangeInclusive<usize>,
        f: impl Fn(&[Value]) -> Result<Value, String> + 'static,
    ) {
        self.table.insert(name.to_string(), (arity, Box::new(f)));
    }
}

impl Default for Functions {
    fn default() -> Self {
        Functions::standard()
    }
}

fn fold(args: &[Value], f: fn(f64, f64) -> f64) -> Result<Value, String> {
    let mut acc: Option<f64> = None;
    for a in args {
        let Value::Number(n) = a else { return Err(format!("expected numbers, found {}", a.type_name())) };
        acc = Some(acc.map_or(*n, |p| f(p, *n)));
    }
    acc.map(Value::Number).ok_or_else(|| "expected at least one number".into())
}

impl Expr {
    /// Parses `src` as one expression; trailing tokens are an error.
    pub fn parse(src: &str) -> Result<Expr, Diagnostic> {
        let tokens = lex(src)?;
        let mut pos = 0;
        let e = parse_at(&tokens, &mut pos)?;
        match &tokens[pos] {
            Token { kind: TokenKind::Eof, .. } => Ok(e),
            t => Err(Diagnostic::new(t.span, "unexpected token after the expression")),
        }
    }

    /// Resolves every path root and function name and checks call arities, collecting all errors.
    pub fn check(&self, scope: &dyn Scope, functions: &Functions) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        let roots = scope.roots();
        self.walk(&mut |e| match &e.kind {
            ExprKind::Path(p) if !roots.contains(&p[0]) => {
                let span = Span { start: e.span.start, end: e.span.start + p[0].len() };
                out.push(Diagnostic::unknown(span, "name", &p[0], roots.iter().map(String::as_str)));
            }
            ExprKind::Call(name, span, args) => match functions.table.get(name) {
                None => {
                    out.push(Diagnostic::unknown(*span, "function", name, functions.table.keys().map(String::as_str)))
                }
                Some((arity, _)) if !arity.contains(&args.len()) => {
                    out.push(Diagnostic::new(
                        e.span,
                        format!("`{name}` takes {} arguments, found {}", arity_text(arity), args.len()),
                    ));
                }
                Some(_) => {
                    if let (Some(check), Some(Expr { kind: ExprKind::Lit(Value::Text(t)), span: arg })) =
                        (functions.literals.get(name), args.first())
                        && let Some(message) = check(t)
                    {
                        out.push(Diagnostic::new(*arg, message));
                    }
                }
            },
            _ => {}
        });
        out
    }

    fn walk(&self, f: &mut dyn FnMut(&Expr)) {
        f(self);
        match &self.kind {
            ExprKind::Lit(_) | ExprKind::Path(_) => {}
            ExprKind::List(items) | ExprKind::Call(_, _, items) => items.iter().for_each(|e| e.walk(f)),
            ExprKind::Unary(_, a) => a.walk(f),
            ExprKind::Binary(_, a, b) | ExprKind::Index(a, b) => {
                a.walk(f);
                b.walk(f);
            }
            ExprKind::Ternary(a, b, c) => {
                a.walk(f);
                b.walk(f);
                c.walk(f);
            }
        }
    }

    /// Evaluates the expression against the current values of `scope`.
    pub fn eval(&self, scope: &dyn Scope, functions: &Functions) -> Result<Value, Diagnostic> {
        let err = |m: String| Diagnostic::new(self.span, m);
        match &self.kind {
            ExprKind::Lit(v) => Ok(v.clone()),
            ExprKind::Path(p) => scope.get(p).ok_or_else(|| err(format!("`{}` has no value", p.join(".")))),
            ExprKind::List(items) => {
                items.iter().map(|e| e.eval(scope, functions)).collect::<Result<_, _>>().map(Value::List)
            }
            ExprKind::Unary(op, a) => match (op, a.eval(scope, functions)?) {
                (Op::Neg, Value::Number(n)) => Ok(Value::Number(-n)),
                (Op::Not, Value::Bool(b)) => Ok(Value::Bool(!b)),
                (_, v) => Err(err(format!("cannot apply the operator to {}", v.type_name()))),
            },
            ExprKind::Binary(Op::And | Op::Or, a, b) => {
                let and = matches!(self.kind, ExprKind::Binary(Op::And, ..));
                let left = boolean(a, scope, functions)?;
                if left != and { Ok(Value::Bool(left)) } else { boolean(b, scope, functions).map(Value::Bool) }
            }
            ExprKind::Binary(op, a, b) => {
                binary(*op, a.eval(scope, functions)?, b.eval(scope, functions)?).map_err(err)
            }
            ExprKind::Ternary(c, a, b) => {
                if boolean(c, scope, functions)? {
                    a.eval(scope, functions)
                } else {
                    b.eval(scope, functions)
                }
            }
            ExprKind::Call(name, span, args) => {
                let (arity, f) = functions.table.get(name).ok_or_else(|| {
                    Diagnostic::unknown(*span, "function", name, functions.table.keys().map(String::as_str))
                })?;
                if !arity.contains(&args.len()) {
                    return Err(err(format!("`{name}` takes {} arguments, found {}", arity_text(arity), args.len())));
                }
                let values: Vec<Value> = args.iter().map(|e| e.eval(scope, functions)).collect::<Result<_, _>>()?;
                f(&values).map_err(err)
            }
            ExprKind::Index(a, i) => match (a.eval(scope, functions)?, i.eval(scope, functions)?) {
                (Value::List(l), Value::Number(n)) if n >= 0.0 && n.fract() == 0.0 => {
                    l.get(n as usize).cloned().ok_or_else(|| err(format!("index {n} is outside a list of {}", l.len())))
                }
                _ => Err(err("indexing needs a list and a whole number".into())),
            },
        }
    }
}

fn arity_text(r: &RangeInclusive<usize>) -> String {
    match (r.start(), r.end()) {
        (a, b) if a == b => a.to_string(),
        (a, &usize::MAX) => format!("at least {a}"),
        (a, b) => format!("{a} to {b}"),
    }
}

fn boolean(e: &Expr, scope: &dyn Scope, functions: &Functions) -> Result<bool, Diagnostic> {
    match e.eval(scope, functions)? {
        Value::Bool(b) => Ok(b),
        v => Err(Diagnostic::new(e.span, format!("expected bool, found {}", v.type_name()))),
    }
}

fn binary(op: Op, a: Value, b: Value) -> Result<Value, String> {
    use Value::*;
    let mismatch = |a: &Value, b: &Value| format!("cannot combine {} and {}", a.type_name(), b.type_name());
    match (op, &a, &b) {
        (Op::Eq, ..) => Ok(Bool(a == b)),
        (Op::Ne, ..) => Ok(Bool(a != b)),
        (Op::Add, Text(x), Text(y)) => Ok(Text(format!("{x}{y}"))),
        (_, Number(x), Number(y)) => Ok(match op {
            Op::Add => Number(x + y),
            Op::Sub => Number(x - y),
            Op::Mul => Number(x * y),
            Op::Div => Number(x / y),
            Op::Rem => Number(x % y),
            Op::Lt => Bool(x < y),
            Op::Le => Bool(x <= y),
            Op::Gt => Bool(x > y),
            _ => Bool(x >= y),
        }),
        (Op::Lt | Op::Le | Op::Gt | Op::Ge, Text(x), Text(y)) => Ok(Bool(match op {
            Op::Lt => x < y,
            Op::Le => x <= y,
            Op::Gt => x > y,
            _ => x >= y,
        })),
        _ => Err(mismatch(&a, &b)),
    }
}

pub(crate) struct Cursor<'a> {
    tokens: &'a [Token],
    pos: &'a mut usize,
    depth: usize,
}

pub(crate) fn parse_at(tokens: &[Token], pos: &mut usize) -> Result<Expr, Diagnostic> {
    Cursor { tokens, pos, depth: 0 }.ternary()
}

const LEVELS: [&[(&str, Op)]; 6] = [
    &[("||", Op::Or)],
    &[("&&", Op::And)],
    &[("==", Op::Eq), ("!=", Op::Ne)],
    &[("<=", Op::Le), (">=", Op::Ge), ("<", Op::Lt), (">", Op::Gt)],
    &[("+", Op::Add), ("-", Op::Sub)],
    &[("*", Op::Mul), ("/", Op::Div), ("%", Op::Rem)],
];

impl Cursor<'_> {
    fn peek(&self) -> &Token {
        &self.tokens[*self.pos]
    }

    fn punct(&self, p: &str) -> bool {
        let t = self.peek();
        matches!(&t.kind, TokenKind::Punct(q) if *q == p) && (self.depth > 0 || !t.newline)
    }

    fn eat(&mut self, p: &str) -> bool {
        let hit = self.punct(p);
        if hit {
            *self.pos += 1;
        }
        hit
    }

    fn expect(&mut self, p: &str) -> Result<Span, Diagnostic> {
        let span = self.peek().span;
        if self.eat(p) { Ok(span) } else { Err(Diagnostic::new(span, format!("expected `{p}`"))) }
    }

    fn ternary(&mut self) -> Result<Expr, Diagnostic> {
        let cond = self.binary(0)?;
        if !self.eat("?") {
            return Ok(cond);
        }
        let a = self.ternary()?;
        self.expect(":")?;
        let b = self.ternary()?;
        let span = cond.span.to(b.span);
        Ok(Expr { span, kind: ExprKind::Ternary(Box::new(cond), Box::new(a), Box::new(b)) })
    }

    fn binary(&mut self, level: usize) -> Result<Expr, Diagnostic> {
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(&(_, op)) = LEVELS[level].iter().find(|(p, _)| self.punct(p)) {
            *self.pos += 1;
            let right = self.binary(level + 1)?;
            let span = left.span.to(right.span);
            left = Expr { span, kind: ExprKind::Binary(op, Box::new(left), Box::new(right)) };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, Diagnostic> {
        let start = self.peek().span;
        let op = if self.eat("-") {
            Op::Neg
        } else if self.eat("!") {
            Op::Not
        } else {
            return self.postfix();
        };
        let a = self.unary()?;
        Ok(Expr { span: start.to(a.span), kind: ExprKind::Unary(op, Box::new(a)) })
    }

    fn postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut e = self.primary()?;
        while self.punct("[") {
            *self.pos += 1;
            self.depth += 1;
            let i = self.ternary()?;
            self.depth -= 1;
            let close = self.expect("]")?;
            e = Expr { span: e.span.to(close), kind: ExprKind::Index(Box::new(e), Box::new(i)) };
        }
        Ok(e)
    }

    fn list(&mut self, close: &str) -> Result<(Vec<Expr>, Span), Diagnostic> {
        self.depth += 1;
        let mut items = Vec::new();
        while !self.punct(close) {
            items.push(self.ternary()?);
            if !self.eat(",") {
                break;
            }
        }
        self.depth -= 1;
        let end = self.expect(close)?;
        Ok((items, end))
    }

    fn primary(&mut self) -> Result<Expr, Diagnostic> {
        let token = self.peek().clone();
        let span = token.span;
        *self.pos += 1;
        let lit = |v| Ok(Expr { span, kind: ExprKind::Lit(v) });
        match token.kind {
            TokenKind::Number(n) => lit(Value::Number(n)),
            TokenKind::Text(t) => lit(Value::Text(t)),
            TokenKind::Color(c) => lit(Value::Color(c)),
            TokenKind::Ident(name) => match name.as_str() {
                "true" => lit(Value::Bool(true)),
                "false" => lit(Value::Bool(false)),
                "null" => lit(Value::Null),
                _ if self.punct("(") => {
                    *self.pos += 1;
                    let (args, end) = self.list(")")?;
                    Ok(Expr { span: span.to(end), kind: ExprKind::Call(name, span, args) })
                }
                _ => {
                    let mut path = vec![name];
                    let mut end = span;
                    while self.punct(".") {
                        *self.pos += 1;
                        let t = self.peek().clone();
                        let TokenKind::Ident(seg) = t.kind else {
                            return Err(Diagnostic::new(t.span, "expected a name after `.`"));
                        };
                        *self.pos += 1;
                        end = t.span;
                        path.push(seg);
                    }
                    Ok(Expr { span: span.to(end), kind: ExprKind::Path(path) })
                }
            },
            TokenKind::Punct("(") => {
                self.depth += 1;
                let inner = self.ternary()?;
                self.depth -= 1;
                let close = self.expect(")")?;
                Ok(Expr { span: span.to(close), kind: inner.kind })
            }
            TokenKind::Punct("[") => {
                let (items, end) = self.list("]")?;
                Ok(Expr { span: span.to(end), kind: ExprKind::List(items) })
            }
            _ => {
                *self.pos -= 1;
                Err(Diagnostic::new(span, "expected an expression"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Env;

    impl Scope for Env {
        fn roots(&self) -> Vec<String> {
            vec!["slider".into(), "theme".into()]
        }

        fn get(&self, path: &[String]) -> Option<Value> {
            match path.join(".").as_str() {
                "slider.value" => Some(Value::Number(0.25)),
                "theme.base" => Some(Value::Color(Color::BLACK)),
                "slider.on" => Some(Value::Bool(true)),
                _ => None,
            }
        }
    }

    fn eval(src: &str) -> Result<Value, Diagnostic> {
        Expr::parse(src)?.eval(&Env, &Functions::standard())
    }

    fn num(src: &str) -> f64 {
        match eval(src).unwrap() {
            Value::Number(n) => n,
            v => panic!("{v:?}"),
        }
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(num("1 + 2 * 3 - 4 / 2"), 5.0);
        assert_eq!(num("(1 + 2) * 3 % 5"), 4.0);
        assert_eq!(num("-2 * -3"), 6.0);
        assert_eq!(num("10 - 3 - 2"), 5.0);
        assert_eq!(eval("1 < 2 && 2 <= 2 == true").unwrap(), Value::Bool(true));
        assert_eq!(num("false ? 1 : true ? 2 : 3"), 2.0);
    }

    #[test]
    fn paths_text_lists_and_calls() {
        assert_eq!(num("slider.value * 4"), 1.0);
        assert_eq!(eval("\"a\" + \"b\"").unwrap(), Value::Text("ab".into()));
        assert_eq!(num("len([1, 2, 3][1] + 1 == 3 ? \"abcd\" : \"\")"), 4.0);
        assert_eq!(num("clamp(max(1, 7, 3), 0, 5) + floor(1.9) + round(2.5)"), 9.0);
        assert_eq!(eval("mix(#000, #fff, 0.5)").unwrap(), Value::Color(Color::rgba(0.5, 0.5, 0.5, 1.0)));
        assert_eq!(num("mix(10, 20, slider.value)"), 12.5);
    }

    #[test]
    fn logic_short_circuits_and_needs_bools() {
        assert_eq!(eval("false && missing.x").unwrap(), Value::Bool(false));
        assert_eq!(eval("true || missing.x").unwrap(), Value::Bool(true));
        assert!(eval("1 && true").unwrap_err().message.contains("expected bool"));
        assert!(eval("1 + \"a\"").unwrap_err().message.contains("cannot combine number and text"));
        assert_eq!(eval("1 == \"1\"").unwrap(), Value::Bool(false));
    }

    #[test]
    fn newline_ends_an_expression_outside_brackets() {
        let tokens = lex("a: 1\n-2").unwrap();
        let mut pos = 2;
        let e = parse_at(&tokens, &mut pos).unwrap();
        assert_eq!(e.kind, ExprKind::Lit(Value::Number(1.0)));
        assert_eq!(tokens[pos].kind, TokenKind::Punct("-"));
        assert_eq!(num("(1\n+ 2)"), 3.0);
    }

    #[test]
    fn check_reports_names_and_arity_with_suggestions() {
        let e = Expr::parse("slidr.value + clmp(1, 2) + abs(1, 2)").unwrap();
        let d = e.check(&Env, &Functions::standard());
        assert_eq!(d.len(), 3);
        assert_eq!(d[0].hint.as_deref(), Some("did you mean `slider`?"));
        assert_eq!(d[1].hint.as_deref(), Some("did you mean `clamp`?"));
        assert!(d[2].message.contains("takes 1 arguments, found 2"));
        assert!(Expr::parse("slider.value").unwrap().check(&Env, &Functions::standard()).is_empty());
    }

    #[test]
    fn syntax_errors_are_located() {
        let d = Expr::parse("1 + ").unwrap_err();
        assert_eq!((d.message.as_str(), d.span.start), ("expected an expression", 4));
        assert!(Expr::parse("(1").unwrap_err().message.contains("expected `)`"));
        assert!(Expr::parse("1 2").unwrap_err().message.contains("after the expression"));
        assert!(Expr::parse("a.").unwrap_err().message.contains("after `.`"));
        assert!(eval("[1][3]").unwrap_err().message.contains("outside"));
        assert!(eval("slider.nope").unwrap_err().message.contains("no value"));
    }

    #[test]
    fn host_functions_join_the_whitelist() {
        let mut f = Functions::standard();
        f.register("t", 1..=1, |a| match &a[0] {
            Value::Text(k) => Ok(Value::Text(format!("<{k}>"))),
            _ => Err("t takes a key".into()),
        });
        assert_eq!(Expr::parse("t(\"ok\")").unwrap().eval(&Env, &f).unwrap(), Value::Text("<ok>".into()));
        assert!(Expr::parse("t(1)").unwrap().eval(&Env, &f).unwrap_err().message.contains("t takes a key"));
    }

    #[test]
    fn fmt_time_and_literal_checks() {
        let mut f = Functions::standard();
        f.check_literal("t", |k| (k != "ok").then(|| format!("no translation `{k}`")));
        f.register("t", 1..=1, |a| Ok(a[0].clone()));
        let eval = |src: &str| Expr::parse(src).unwrap().eval(&Env, &f).unwrap();
        assert_eq!(eval("fmt_time(75)"), Value::Text("1:15".into()));
        assert_eq!(eval("fmt_time(3725)"), Value::Text("1:02:05".into()));
        let bad = Expr::parse("t(\"nope\")").unwrap().check(&Env, &f);
        assert_eq!(bad.len(), 1);
        assert!(bad[0].message.contains("no translation `nope`"));
        assert!(Expr::parse("t(\"ok\")").unwrap().check(&Env, &f).is_empty());
    }
}
