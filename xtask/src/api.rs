use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use quote::ToTokens;
use syn::{
    Attribute, Fields, ImplItem, Item, ItemEnum, ItemImpl, ItemStruct, ItemTrait, ItemUse, TraitItem, UseTree,
    Visibility,
};

pub fn list(crate_dir: &Path) -> Result<String, String> {
    let name = crate_dir.file_name().and_then(|n| n.to_str()).ok_or("bad crate directory")?.replace('-', "_");
    let root = crate_dir.join("src/lib.rs");
    let mut surface = Surface::default();
    surface.read_module(&root, &crate_dir.join("src"), vec![name.clone()], true)?;
    Ok(surface.render())
}

#[derive(Default)]
struct Surface {
    items: Vec<(Vec<String>, bool, Item)>,
    exported: BTreeMap<String, BTreeSet<String>>,
    globs: BTreeSet<String>,
}

impl Surface {
    fn read_module(&mut self, file: &Path, dir: &Path, path: Vec<String>, public: bool) -> Result<(), String> {
        let text = fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
        let parsed = syn::parse_file(&text).map_err(|e| format!("{}: {e}", file.display()))?;
        self.walk(parsed.items, dir, path, public)
    }

    fn walk(&mut self, items: Vec<Item>, dir: &Path, path: Vec<String>, public: bool) -> Result<(), String> {
        for item in items {
            if is_test(&item) {
                continue;
            }
            match item {
                Item::Mod(module) => {
                    let name = module.ident.to_string();
                    let visible = public && is_pub(&module.vis);
                    let mut child = path.clone();
                    child.push(name.clone());
                    match module.content {
                        Some((_, inner)) => self.walk(inner, &dir.join(&name), child, visible)?,
                        None => {
                            let flat = dir.join(format!("{name}.rs"));
                            let nested = dir.join(&name).join("mod.rs");
                            let file = if flat.exists() { flat } else { nested };
                            self.read_module(&file, &dir.join(&name), child, visible)?;
                        }
                    }
                }
                Item::Use(import) if is_pub(&import.vis) && public => self.note_use(&import, &path),
                other if public => self.items.push((path.clone(), true, other)),
                other => self.items.push((path.clone(), false, other)),
            }
        }
        Ok(())
    }

    fn note_use(&mut self, import: &ItemUse, path: &[String]) {
        let module = path.join("::");
        collect_use(&import.tree, &module, &mut self.exported, &mut self.globs);
    }

    fn render(&self) -> String {
        let mut public_types: BTreeMap<String, String> = BTreeMap::new();
        let mut lines = BTreeSet::new();
        for (path, visible, item) in &self.items {
            let Some(name) = item_name(item) else { continue };
            let module = path.join("::");
            let places: Vec<String> = if *visible {
                vec![module.clone()]
            } else {
                let mut found: Vec<String> =
                    self.exported.get(&name).map(|m| m.iter().cloned().collect()).unwrap_or_default();
                if self.globs.contains(&module) {
                    found.push(module.clone());
                }
                found
            };
            if places.is_empty() || !is_pub_item(item) {
                continue;
            }
            public_types.entry(name.clone()).or_insert_with(|| places[0].clone());
            for place in places {
                for line in describe(item, &format!("{place}::{name}")) {
                    lines.insert(line);
                }
            }
        }
        for (_, _, item) in &self.items {
            if let Item::Impl(imp) = item {
                let Some(base) = impl_base(imp) else { continue };
                if let Some(place) = public_types.get(&base) {
                    for line in describe_impl(imp, &format!("{place}::{base}")) {
                        lines.insert(line);
                    }
                }
            }
        }
        let mut out = String::new();
        for line in lines {
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

fn collect_use(
    tree: &UseTree,
    module: &str,
    exported: &mut BTreeMap<String, BTreeSet<String>>,
    globs: &mut BTreeSet<String>,
) {
    match tree {
        UseTree::Path(p) => {
            if p.ident == "std" || p.ident == "core" {
                return;
            }
            collect_use(&p.tree, module, exported, globs);
            if matches!(&*p.tree, UseTree::Glob(_))
                && !matches!(p.ident.to_string().as_str(), "self" | "super" | "crate")
            {
                globs.insert(format!("{}::{}", module.split("::").next().unwrap_or(""), p.ident));
            }
        }
        UseTree::Name(n) => {
            exported.entry(n.ident.to_string()).or_default().insert(module.to_owned());
        }
        UseTree::Rename(r) => {
            exported.entry(r.ident.to_string()).or_default().insert(module.to_owned());
        }
        UseTree::Group(g) => {
            for t in &g.items {
                collect_use(t, module, exported, globs);
            }
        }
        UseTree::Glob(_) => {}
    }
}

fn is_test(item: &Item) -> bool {
    let attrs: &[Attribute] = match item {
        Item::Mod(m) => &m.attrs,
        Item::Fn(f) => &f.attrs,
        Item::Impl(i) => &i.attrs,
        Item::Use(u) => &u.attrs,
        _ => return false,
    };
    attrs.iter().any(|a| {
        let text = a.to_token_stream().to_string();
        text.contains("cfg (test)") || text.contains("cfg (any (test")
    })
}

fn is_pub(vis: &Visibility) -> bool {
    matches!(vis, Visibility::Public(_))
}

fn is_pub_item(item: &Item) -> bool {
    match item {
        Item::Struct(i) => is_pub(&i.vis),
        Item::Enum(i) => is_pub(&i.vis),
        Item::Trait(i) => is_pub(&i.vis),
        Item::Fn(i) => is_pub(&i.vis),
        Item::Const(i) => is_pub(&i.vis),
        Item::Static(i) => is_pub(&i.vis),
        Item::Type(i) => is_pub(&i.vis),
        Item::Macro(m) => m.attrs.iter().any(|a| a.path().is_ident("macro_export")),
        _ => false,
    }
}

fn item_name(item: &Item) -> Option<String> {
    Some(match item {
        Item::Struct(i) => i.ident.to_string(),
        Item::Enum(i) => i.ident.to_string(),
        Item::Trait(i) => i.ident.to_string(),
        Item::Fn(i) => i.sig.ident.to_string(),
        Item::Const(i) => i.ident.to_string(),
        Item::Static(i) => i.ident.to_string(),
        Item::Type(i) => i.ident.to_string(),
        Item::Macro(m) => m.ident.as_ref()?.to_string(),
        _ => return None,
    })
}

fn flat(tokens: impl ToTokens) -> String {
    let text = tokens.to_token_stream().to_string();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn derives(attrs: &[Attribute]) -> String {
    let mut found = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("derive") {
            let _ = attr.parse_nested_meta(|meta| {
                found.push(flat(&meta.path));
                Ok(())
            });
        }
        if attr.path().is_ident("non_exhaustive") {
            found.push("non_exhaustive".to_owned());
        }
    }
    found.sort();
    if found.is_empty() { String::new() } else { format!(" [{}]", found.join(", ")) }
}

fn describe(item: &Item, path: &str) -> Vec<String> {
    match item {
        Item::Struct(i) => describe_struct(i, path),
        Item::Enum(i) => describe_enum(i, path),
        Item::Trait(i) => describe_trait(i, path),
        Item::Fn(i) => vec![format!("{path}: fn {}", flat(&i.sig).trim_start_matches("fn "))],
        Item::Const(i) => vec![format!("{path}: const {}: {}", i.ident, flat(&i.ty))],
        Item::Static(i) => vec![format!("{path}: static {}: {}", i.ident, flat(&i.ty))],
        Item::Type(i) => vec![format!("{path}: type {}{} = {}", i.ident, flat(&i.generics), flat(&i.ty))],
        Item::Macro(_) => vec![format!("{path}: macro")],
        _ => Vec::new(),
    }
}

fn describe_struct(item: &ItemStruct, path: &str) -> Vec<String> {
    let mut out = vec![format!("{path}: struct{}{}", derives(&item.attrs), flat(&item.generics))];
    match &item.fields {
        Fields::Named(named) => {
            for field in &named.named {
                if is_pub(&field.vis) {
                    let name = field.ident.as_ref().map(ToString::to_string).unwrap_or_default();
                    out.push(format!("{path}.{name}: {}", flat(&field.ty)));
                }
            }
        }
        Fields::Unnamed(unnamed) => {
            for (i, field) in unnamed.unnamed.iter().enumerate() {
                if is_pub(&field.vis) {
                    out.push(format!("{path}.{i}: {}", flat(&field.ty)));
                }
            }
        }
        Fields::Unit => {}
    }
    out
}

fn describe_enum(item: &ItemEnum, path: &str) -> Vec<String> {
    let mut out = vec![format!("{path}: enum{}{}", derives(&item.attrs), flat(&item.generics))];
    for variant in &item.variants {
        out.push(format!("{path}::{}: variant {}", variant.ident, flat(&variant.fields)));
    }
    out
}

fn describe_trait(item: &ItemTrait, path: &str) -> Vec<String> {
    let mut out = vec![format!("{path}: trait{}", flat(&item.generics))];
    for member in &item.items {
        match member {
            TraitItem::Fn(f) => {
                let required = if f.default.is_some() { "provided" } else { "required" };
                out.push(format!("{path}::{}: {required} {}", f.sig.ident, flat(&f.sig)));
            }
            TraitItem::Const(c) => out.push(format!("{path}::{}: const {}", c.ident, flat(&c.ty))),
            TraitItem::Type(t) => out.push(format!("{path}::{}: type", t.ident)),
            _ => {}
        }
    }
    out
}

fn impl_base(imp: &ItemImpl) -> Option<String> {
    match &*imp.self_ty {
        syn::Type::Path(p) => p.path.segments.last().map(|s| s.ident.to_string()),
        _ => None,
    }
}

fn describe_impl(imp: &ItemImpl, base: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some((_, trait_path, _)) = &imp.trait_ {
        out.push(format!("impl {} for {}", flat(trait_path), base));
        return out;
    }
    for member in &imp.items {
        match member {
            ImplItem::Fn(f) if is_pub(&f.vis) => out.push(format!("{base}::{}: {}", f.sig.ident, flat(&f.sig))),
            ImplItem::Const(c) if is_pub(&c.vis) => out.push(format!("{base}::{}: const {}", c.ident, flat(&c.ty))),
            _ => {}
        }
    }
    out
}

pub fn crate_dirs(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(root.join("crates"))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join("src/lib.rs").exists())
        .collect();
    dirs.sort();
    Ok(dirs)
}

pub fn run(root: &Path, check: bool) -> Result<String, String> {
    let out_dir = root.join("docs/api");
    fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    let mut stale = Vec::new();
    let mut written = 0;
    for dir in crate_dirs(root)? {
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_owned();
        let listing = list(&dir)?;
        let target = out_dir.join(format!("{name}.txt"));
        if check {
            if fs::read_to_string(&target).ok().as_deref() != Some(listing.as_str()) {
                stale.push(name);
            }
        } else {
            fs::write(&target, listing).map_err(|e| e.to_string())?;
            written += 1;
        }
    }
    if !stale.is_empty() {
        return Err(format!(
            "public API changed without updating docs/api: {} (run cargo xtask api and note it in CHANGELOG.md)",
            stale.join(", ")
        ));
    }
    Ok(if check { "public API lists are current".to_owned() } else { format!("wrote {written} API lists to docs/api") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crate_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("demo-crate");
        fs::create_dir_all(root.join("src")).unwrap();
        for (name, text) in files {
            let path = root.join("src").join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        dir
    }

    fn listing(files: &[(&str, &str)]) -> String {
        let dir = crate_with(files);
        list(&dir.path().join("demo-crate")).unwrap()
    }

    #[test]
    fn lists_public_items_and_skips_private_ones() {
        let out = listing(&[(
            "lib.rs",
            "pub struct A { pub x: f32, y: f32 }\nstruct B;\npub fn go(a: &A) -> f32 { 0.0 }\nfn hidden() {}\npub(crate) fn also() {}\nimpl A { pub fn new() -> Self { A { x: 0.0, y: 0.0 } } fn secret(&self) {} }",
        )]);
        assert!(out.contains("demo_crate::A: struct"));
        assert!(out.contains("demo_crate::A.x: f32"));
        assert!(!out.contains("A.y"));
        assert!(out.contains("demo_crate::go: fn go (a : & A) -> f32"));
        assert!(!out.contains("hidden") && !out.contains("also") && !out.contains("secret"));
        assert!(out.contains("A::new: fn new () -> Self"));
        assert!(!out.contains("::B"));
    }

    #[test]
    fn private_modules_surface_only_through_reexports() {
        let out = listing(&[
            ("lib.rs", "mod inner;\nmod unused;\npub use inner::Shown;\npub mod open;"),
            ("inner.rs", "pub struct Shown;\npub struct NotShown;"),
            ("unused.rs", "pub struct Lost;"),
            ("open.rs", "pub fn direct() {}"),
        ]);
        assert!(out.contains("demo_crate::Shown: struct"));
        assert!(!out.contains("NotShown") && !out.contains("Lost"));
        assert!(out.contains("demo_crate::open::direct"));
    }

    #[test]
    fn derives_variants_and_trait_methods_are_part_of_the_list() {
        let out = listing(&[(
            "lib.rs",
            "#[derive(Debug, Clone)]\npub enum E { A, B(u8) }\npub trait T { fn need(&self); fn have(&self) {} }\nimpl T for E { fn need(&self) {} }\n#[cfg(test)]\nmod tests { pub fn t() {} }",
        )]);
        assert!(out.contains("demo_crate::E: enum [Clone, Debug]"));
        assert!(out.contains("demo_crate::E::B: variant"));
        assert!(out.contains("demo_crate::T::need: required"));
        assert!(out.contains("demo_crate::T::have: provided"));
        assert!(out.contains("impl T for demo_crate::E"));
        assert!(!out.contains("tests"));
    }

    #[test]
    fn a_changed_signature_changes_the_listing() {
        let before = listing(&[("lib.rs", "pub fn f(a: u8) {}")]);
        let after = listing(&[("lib.rs", "pub fn f(a: u16) {}")]);
        assert_ne!(before, after);
    }

    #[test]
    fn check_reports_the_stale_crates() {
        let dir = tempfile::tempdir().unwrap();
        let krate = dir.path().join("crates/demo-crate/src");
        fs::create_dir_all(&krate).unwrap();
        fs::write(krate.join("lib.rs"), "pub fn f() {}").unwrap();
        assert!(run(dir.path(), true).unwrap_err().contains("demo-crate"));
        run(dir.path(), false).unwrap();
        assert!(run(dir.path(), true).is_ok());
        fs::write(krate.join("lib.rs"), "pub fn f(a: u8) {}").unwrap();
        assert!(run(dir.path(), true).is_err());
    }
}
