use crate::compile::Names;
use crate::{Component, Diagnostic, Functions, Member, Value};

const LIMIT: f64 = 1000.0;

struct Ctx<'a> {
    functions: &'a Functions,
    errors: Vec<Diagnostic>,
}

pub(crate) fn expand(root: &Component, functions: &Functions) -> Result<Component, Vec<Diagnostic>> {
    if root.name == "Repeater" {
        return Err(vec![Diagnostic::new(root.span, "the root component cannot be a Repeater")]);
    }
    let mut cx = Ctx { functions, errors: Vec::new() };
    let root = component(root, None, &mut cx);
    if cx.errors.is_empty() { Ok(root) } else { Err(cx.errors) }
}

fn component(c: &Component, index: Option<f64>, cx: &mut Ctx) -> Component {
    let bind = |e: &crate::Expr| index.map_or_else(|| e.clone(), |i| e.bind("index", i));
    let mut members = Vec::new();
    for m in &c.members {
        match m {
            Member::Child(r) if r.name == "Repeater" => members.extend(repeat(r, index, cx)),
            Member::Child(child) => members.push(Member::Child(component(child, index, cx))),
            Member::Property(n, s, e) => members.push(Member::Property(n.clone(), *s, bind(e))),
            Member::Set(n, s, e) => members.push(Member::Set(n.clone(), *s, bind(e))),
            Member::Handler(n, s, e) => members.push(Member::Handler(n.clone(), *s, bind(e))),
            Member::Behavior(n, s, body) => members.push(Member::Behavior(n.clone(), *s, component(body, index, cx))),
            Member::Id(..) => members.push(m.clone()),
        }
    }
    Component { name: c.name.clone(), span: c.span, members }
}

fn repeat(r: &Component, index: Option<f64>, cx: &mut Ctx) -> Vec<Member> {
    let (mut count, mut bodies) = (None, Vec::new());
    for m in &r.members {
        match m {
            Member::Set(n, _, e) if n == "count" => {
                count = Some(index.map_or_else(|| e.clone(), |i| e.bind("index", i)))
            }
            Member::Child(body) => bodies.push(body),
            _ => cx.errors.push(Diagnostic::new(r.span, "a Repeater takes `count` and components")),
        }
    }
    let Some(count) = count else {
        cx.errors.push(Diagnostic::new(r.span, "a Repeater needs `count`"));
        return Vec::new();
    };
    let n = match count.eval(&Names(Vec::new()), cx.functions) {
        Ok(Value::Number(n)) if n >= 0.0 && n.fract() == 0.0 && n <= LIMIT => n as usize,
        Ok(_) => {
            cx.errors.push(Diagnostic::new(count.span, format!("`count` must be a whole number from 0 to {LIMIT}")));
            return Vec::new();
        }
        Err(d) => {
            cx.errors.push(d);
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for i in 0..n {
        for body in &bodies {
            let copy = component(body, Some(i as f64), cx);
            let mut ids = Vec::new();
            collect_ids(&copy, &mut ids);
            out.push(Member::Child(if ids.is_empty() { copy } else { suffix(&copy, &ids, i) }));
        }
    }
    out
}

fn collect_ids(c: &Component, out: &mut Vec<String>) {
    for m in &c.members {
        match m {
            Member::Id(n, _) => out.push(n.clone()),
            Member::Child(child) | Member::Behavior(_, _, child) => collect_ids(child, out),
            _ => {}
        }
    }
}

fn suffix(c: &Component, ids: &[String], i: usize) -> Component {
    let f = |n: &str| ids.iter().any(|x| x == n).then(|| format!("{n}_{i}"));
    let members = c
        .members
        .iter()
        .map(|m| match m {
            Member::Id(n, s) => Member::Id(f(n).unwrap_or_else(|| n.clone()), *s),
            Member::Property(n, s, e) => Member::Property(n.clone(), *s, e.rename(&f)),
            Member::Set(n, s, e) => Member::Set(n.clone(), *s, e.rename(&f)),
            Member::Handler(n, s, e) => Member::Handler(n.clone(), *s, e.rename(&f)),
            Member::Behavior(n, s, b) => Member::Behavior(n.clone(), *s, suffix(b, ids, i)),
            Member::Child(child) => Member::Child(suffix(child, ids, i)),
        })
        .collect();
    Component { name: c.name.clone(), span: c.span, members }
}
