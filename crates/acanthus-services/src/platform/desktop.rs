use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DesktopAction {
    pub id: String,
    pub name: String,
    pub exec: String,
    pub icon: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DesktopEntry {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    pub generic_name: String,
    pub comment: String,
    pub icon: String,
    pub exec: String,
    pub try_exec: String,
    pub working_dir: String,
    pub terminal: bool,
    pub no_display: bool,
    pub hidden: bool,
    pub startup_wm_class: String,
    pub categories: Vec<String>,
    pub keywords: Vec<String>,
    pub mime_types: Vec<String>,
    pub only_show_in: Vec<String>,
    pub not_show_in: Vec<String>,
    pub actions: Vec<DesktopAction>,
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(';') => out.push(';'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&';') => {
                current.push(';');
                chars.next();
            }
            ';' => {
                let item = unescape(&std::mem::take(&mut current));
                if !item.is_empty() {
                    items.push(item);
                }
            }
            other => current.push(other),
        }
    }
    let tail = unescape(&current);
    if !tail.is_empty() {
        items.push(tail);
    }
    items
}

#[derive(Default)]
struct Group {
    values: HashMap<String, String>,
}

impl Group {
    fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    fn localized(&self, key: &str, locales: &[String]) -> String {
        for locale in locales {
            if let Some(v) = self.values.get(&format!("{key}[{locale}]")) {
                return unescape(v);
            }
        }
        self.get(key).map(unescape).unwrap_or_default()
    }

    fn flag(&self, key: &str) -> bool {
        self.get(key) == Some("true")
    }
}

fn parse_groups(text: &str) -> Vec<(String, Group)> {
    let mut groups: Vec<(String, Group)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            groups.push((name.to_owned(), Group::default()));
            continue;
        }
        if let (Some((_, group)), Some((key, value))) = (groups.last_mut(), line.split_once('=')) {
            group.values.entry(key.trim().to_owned()).or_insert_with(|| value.trim().to_owned());
        }
    }
    groups
}

pub fn expand_locales(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for locale in raw {
        let base = locale.split('.').next().unwrap_or("");
        let no_modifier = base.split('@').next().unwrap_or("");
        let lang = no_modifier.split('_').next().unwrap_or("");
        for candidate in [base, no_modifier, lang] {
            if !candidate.is_empty() && candidate != "C" && candidate != "POSIX" && !out.iter().any(|o| o == candidate)
            {
                out.push(candidate.to_owned());
            }
        }
    }
    out
}

pub fn locales_from_env(env: impl Fn(&str) -> Option<String>) -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = env(key).filter(|v| !v.is_empty()) {
            raw.push(value);
        }
    }
    if let Some(list) = env("LANGUAGE") {
        raw.extend(list.split(':').filter(|s| !s.is_empty()).map(str::to_owned));
    }
    expand_locales(&raw)
}

pub fn parse_entry(id: &str, path: &Path, text: &str, locales: &[String]) -> Option<DesktopEntry> {
    let groups = parse_groups(text);
    let main = &groups.iter().find(|(name, _)| name == "Desktop Entry")?.1;
    if main.get("Type").is_some_and(|t| t != "Application") {
        return None;
    }
    let list = |key: &str| main.get(key).map(split_list).unwrap_or_default();
    let action_ids = list("Actions");
    let actions = action_ids
        .iter()
        .filter_map(|action_id| {
            let group = &groups.iter().find(|(name, _)| *name == format!("Desktop Action {action_id}"))?.1;
            Some(DesktopAction {
                id: action_id.clone(),
                name: group.localized("Name", locales),
                exec: group.get("Exec").map(unescape).unwrap_or_default(),
                icon: group.get("Icon").map(unescape).unwrap_or_default(),
            })
        })
        .collect();
    let exec = main.get("Exec").map(unescape).unwrap_or_default();
    let name = main.localized("Name", locales);
    if exec.is_empty() && name.is_empty() {
        return None;
    }
    Some(DesktopEntry {
        id: id.to_owned(),
        path: path.to_path_buf(),
        name,
        generic_name: main.localized("GenericName", locales),
        comment: main.localized("Comment", locales),
        icon: main.get("Icon").map(unescape).unwrap_or_default(),
        exec,
        try_exec: main.get("TryExec").map(unescape).unwrap_or_default(),
        working_dir: main.get("Path").map(unescape).unwrap_or_default(),
        terminal: main.flag("Terminal"),
        no_display: main.flag("NoDisplay"),
        hidden: main.flag("Hidden"),
        startup_wm_class: main.get("StartupWMClass").map(unescape).unwrap_or_default(),
        categories: list("Categories"),
        keywords: main
            .get("Keywords")
            .map(|_| main.localized("Keywords", locales))
            .map(|k| split_list(&k))
            .unwrap_or_default(),
        mime_types: list("MimeType"),
        only_show_in: list("OnlyShowIn"),
        not_show_in: list("NotShowIn"),
        actions,
    })
}

pub fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_token = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_token = true;
            }
            '\\' if quoted => match chars.next() {
                Some(next @ ('"' | '`' | '$' | '\\')) => current.push(next),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            c if c.is_whitespace() && !quoted => {
                if in_token {
                    args.push(std::mem::take(&mut current));
                    in_token = false;
                }
            }
            other => {
                current.push(other);
                in_token = true;
            }
        }
    }
    if in_token {
        args.push(current);
    }
    args
}

impl DesktopEntry {
    pub fn command(&self, files: &[String]) -> Vec<String> {
        expand_exec(&self.exec, self, files)
    }

    pub fn action_command(&self, action_id: &str) -> Option<Vec<String>> {
        let action = self.actions.iter().find(|a| a.id == action_id)?;
        Some(expand_exec(&action.exec, self, &[]))
    }

    pub fn is_visible_in(&self, current_desktops: &[String]) -> bool {
        if self.hidden {
            return false;
        }
        let matches = |list: &[String]| list.iter().any(|d| current_desktops.iter().any(|c| c.eq_ignore_ascii_case(d)));
        if !self.only_show_in.is_empty() && !matches(&self.only_show_in) {
            return false;
        }
        self.not_show_in.is_empty() || !matches(&self.not_show_in)
    }

    pub fn launch_command(&self, files: &[String], terminal: Option<&str>) -> Vec<String> {
        let command = self.command(files);
        match (self.terminal, terminal) {
            (true, Some(term)) if !term.is_empty() => {
                let mut wrapped = split_exec(term);
                wrapped.push("-e".into());
                wrapped.extend(command);
                wrapped
            }
            _ => command,
        }
    }
}

fn expand_exec(exec: &str, entry: &DesktopEntry, files: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for arg in split_exec(exec) {
        match arg.as_str() {
            "%f" | "%u" => {
                if let Some(first) = files.first() {
                    out.push(first.clone());
                }
                continue;
            }
            "%F" | "%U" => {
                out.extend(files.iter().cloned());
                continue;
            }
            "%i" => {
                if !entry.icon.is_empty() {
                    out.push("--icon".into());
                    out.push(entry.icon.clone());
                }
                continue;
            }
            _ => {}
        }
        let mut expanded = String::new();
        let mut chars = arg.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                expanded.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => expanded.push('%'),
                Some('c') => expanded.push_str(&entry.name),
                Some('k') => expanded.push_str(&entry.path.to_string_lossy()),
                Some('f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'v' | 'm') => {}
                Some(other) => {
                    expanded.push('%');
                    expanded.push(other);
                }
                None => expanded.push('%'),
            }
        }
        if !expanded.is_empty() {
            out.push(expanded);
        }
    }
    out
}

pub fn application_dirs(env: impl Fn(&str) -> Option<String>) -> Vec<PathBuf> {
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    let data_home = env("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".local/share")));
    let data_dirs =
        env("XDG_DATA_DIRS").filter(|v| !v.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned());
    let mut dirs: Vec<PathBuf> = data_home.into_iter().map(|d| d.join("applications")).collect();
    dirs.extend(data_dirs.split(':').filter(|d| !d.is_empty()).map(|d| Path::new(d).join("applications")));
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

fn collect_files(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, out);
        } else if path.extension().is_some_and(|e| e == "desktop")
            && let Ok(relative) = path.strip_prefix(base)
        {
            out.push((relative.to_string_lossy().replace('/', "-"), path));
        }
    }
}

pub fn scan(dirs: &[PathBuf], locales: &[String]) -> Vec<DesktopEntry> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut entries = Vec::new();
    for dir in dirs {
        let mut files = Vec::new();
        collect_files(dir, dir, &mut files);
        files.sort();
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            if let Some(entry) = parse_entry(&id, &path, &String::from_utf8_lossy(&bytes), locales) {
                entries.push(entry);
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIREFOX: &str = "# comment
[Desktop Entry]
Version=1.0
Type=Application
Name=Firefox
Name[de]=Feuerfuchs
Name[de_AT]=Feuerfuchs AT
GenericName=Web Browser
Comment=Browse the Web
Exec=firefox %u
Icon=firefox
Terminal=false
StartupWMClass=firefox
Categories=Network;WebBrowser;
Keywords=web;browser;Internet;
MimeType=text/html;x-scheme-handler/http;
Actions=new-window;private-window;

[Desktop Action new-window]
Name=Open a New Window
Exec=firefox --new-window %u

[Desktop Action private-window]
Name=Open a New Private Window
Name[de]=Neues privates Fenster
Exec=firefox --private-window %u
";

    fn parse(text: &str, locales: &[&str]) -> Option<DesktopEntry> {
        let locales: Vec<String> = locales.iter().map(|s| (*s).to_owned()).collect();
        parse_entry("firefox.desktop", Path::new("/x/firefox.desktop"), text, &locales)
    }

    #[test]
    fn parses_fields_lists_and_actions() {
        let entry = parse(FIREFOX, &[]).unwrap();
        assert_eq!(
            (entry.name.as_str(), entry.generic_name.as_str(), entry.icon.as_str()),
            ("Firefox", "Web Browser", "firefox")
        );
        assert_eq!(entry.categories, ["Network", "WebBrowser"]);
        assert_eq!(entry.keywords, ["web", "browser", "Internet"]);
        assert_eq!(entry.mime_types.len(), 2);
        assert_eq!(entry.startup_wm_class, "firefox");
        assert!(!entry.terminal && !entry.no_display);
        assert_eq!(entry.actions.len(), 2);
        assert_eq!(entry.actions[1].name, "Open a New Private Window");
        assert_eq!(entry.action_command("new-window").unwrap(), ["firefox", "--new-window"]);
    }

    #[test]
    fn localized_names_prefer_the_most_specific_locale() {
        assert_eq!(parse(FIREFOX, &["de_AT", "de"]).unwrap().name, "Feuerfuchs AT");
        assert_eq!(parse(FIREFOX, &["de"]).unwrap().name, "Feuerfuchs");
        assert_eq!(parse(FIREFOX, &["fr"]).unwrap().name, "Firefox");
        assert_eq!(parse(FIREFOX, &["de"]).unwrap().actions[1].name, "Neues privates Fenster");
        assert_eq!(expand_locales(&["de_AT.UTF-8".into(), "C".into()]), ["de_AT", "de"]);
        assert_eq!(expand_locales(&["sr_RS@latin".into()]), ["sr_RS@latin", "sr_RS", "sr"].map(String::from));
    }

    #[test]
    fn rejects_non_applications_and_empty_entries() {
        assert!(parse("[Desktop Entry]\nType=Link\nName=x\nURL=http://x", &[]).is_none());
        assert!(parse("[Other]\nName=x", &[]).is_none());
        assert!(parse("[Desktop Entry]\nType=Application", &[]).is_none());
    }

    #[test]
    fn exec_field_codes_expand_per_the_spec() {
        let entry = parse(FIREFOX, &[]).unwrap();
        assert_eq!(entry.command(&[]), ["firefox"]);
        assert_eq!(entry.command(&["https://a".into()]), ["firefox", "https://a"]);
        let multi = DesktopEntry {
            exec: "viewer %F --title \"%c\" --icon-name=%i 100%% %k".into(),
            name: "My App".into(),
            icon: "icon".into(),
            path: "/p.desktop".into(),
            ..Default::default()
        };
        assert_eq!(
            multi.command(&["a.png".into(), "b.png".into()]),
            ["viewer", "a.png", "b.png", "--title", "My App", "--icon-name=%i", "100%", "/p.desktop"].map(String::from)
        );
        let icon = DesktopEntry { exec: "app %i".into(), icon: "ico".into(), ..Default::default() };
        assert_eq!(icon.command(&[]), ["app", "--icon", "ico"]);
    }

    #[test]
    fn quoting_in_exec_lines() {
        assert_eq!(split_exec(r#"sh -c "echo \"hi there\" $HOME""#), ["sh", "-c", "echo \"hi there\" $HOME"]);
        assert_eq!(split_exec("  a   b  "), ["a", "b"]);
        assert_eq!(split_exec("\"\" x"), ["", "x"]);
        assert_eq!(split_exec(r"env VAR=1 prog\ with"), ["env", "VAR=1", "prog\\", "with"]);
    }

    #[test]
    fn visibility_follows_show_in_lists_and_hidden() {
        let desktops = vec!["Hyprland".to_owned()];
        let base = DesktopEntry::default();
        assert!(base.is_visible_in(&desktops));
        assert!(!DesktopEntry { only_show_in: vec!["GNOME".into()], ..base.clone() }.is_visible_in(&desktops));
        assert!(DesktopEntry { only_show_in: vec!["hyprland".into()], ..base.clone() }.is_visible_in(&desktops));
        assert!(!DesktopEntry { not_show_in: vec!["Hyprland".into()], ..base.clone() }.is_visible_in(&desktops));
        assert!(!DesktopEntry { hidden: true, ..base }.is_visible_in(&desktops));
    }

    #[test]
    fn terminal_apps_are_wrapped_in_the_terminal() {
        let entry = DesktopEntry { exec: "htop".into(), terminal: true, ..Default::default() };
        assert_eq!(entry.launch_command(&[], Some("kitty")), ["kitty", "-e", "htop"]);
        assert_eq!(entry.launch_command(&[], None), ["htop"]);
        let gui = DesktopEntry { exec: "gimp".into(), ..Default::default() };
        assert_eq!(gui.launch_command(&[], Some("kitty")), ["gimp"]);
    }

    #[test]
    fn scan_uses_the_first_directory_for_duplicate_ids_and_nested_names() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("app.desktop"), "[Desktop Entry]\nType=Application\nName=From A\nExec=a").unwrap();
        std::fs::write(b.path().join("app.desktop"), "[Desktop Entry]\nType=Application\nName=From B\nExec=b").unwrap();
        std::fs::create_dir_all(b.path().join("kde")).unwrap();
        std::fs::write(b.path().join("kde/tool.desktop"), "[Desktop Entry]\nType=Application\nName=Tool\nExec=t")
            .unwrap();
        std::fs::write(b.path().join("broken.desktop"), "garbage").unwrap();
        let entries = scan(&[a.path().to_path_buf(), b.path().to_path_buf()], &[]);
        let names: Vec<(&str, &str)> = entries.iter().map(|e| (e.id.as_str(), e.name.as_str())).collect();
        assert_eq!(names, [("app.desktop", "From A"), ("kde-tool.desktop", "Tool")]);
    }

    #[test]
    fn directory_list_follows_xdg() {
        let env = |k: &str| match k {
            "HOME" => Some("/home/u".to_owned()),
            "XDG_DATA_DIRS" => Some("/a:/b".to_owned()),
            _ => None,
        };
        let dirs = application_dirs(env);
        assert_eq!(dirs[0], PathBuf::from("/home/u/.local/share/applications"));
        assert_eq!(&dirs[1..3], [PathBuf::from("/a/applications"), PathBuf::from("/b/applications")]);
        assert_eq!(dirs.len(), 3);
    }

    #[test]
    fn real_system_entries_parse_when_present() {
        let dirs = application_dirs(|k| std::env::var(k).ok());
        let entries = scan(&dirs, &locales_from_env(|k| std::env::var(k).ok()));
        for entry in &entries {
            assert!(!entry.id.is_empty());
            assert!(!entry.name.is_empty() || !entry.exec.is_empty());
        }
    }
}
