use std::cell::{RefCell, RefMut};
use std::path::Path;
use std::rc::Rc;

use acanthus_core::{Signal, Vec2};
use cosmic_text::{
    Align, Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping, Style, UnderlineStyle,
    Weight, Wrap,
};

/// Shared font database. Cloning shares it; loading a font bumps [`Fonts::generation`] so text re-shapes.
#[derive(Clone)]
pub struct Fonts {
    system: Rc<RefCell<FontSystem>>,
    generation: Signal<u64>,
}

impl Fonts {
    /// Creates a database with the system fonts.
    pub fn new() -> Self {
        Fonts { system: Rc::new(RefCell::new(FontSystem::new())), generation: Signal::new(0) }
    }

    /// Mutable access to the underlying font system.
    pub fn system(&self) -> RefMut<'_, FontSystem> {
        self.system.borrow_mut()
    }

    /// A signal that changes whenever the set of fonts changes.
    pub fn generation(&self) -> Signal<u64> {
        self.generation.clone()
    }

    /// Loads a font file and returns the family names it added; empty on failure.
    pub fn load_file(&self, path: &Path) -> Vec<String> {
        let families = {
            let mut system = self.system();
            let db = system.db_mut();
            let before = db.len();
            if let Err(e) = db.load_font_file(path) {
                tracing::warn!("cannot load font {}: {e}", path.display());
                return Vec::new();
            }
            let mut names: Vec<String> = db
                .faces()
                .skip(before)
                .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
                .collect();
            names.dedup();
            names
        };
        if !families.is_empty() {
            self.generation.update(|g| *g += 1);
        }
        families
    }

    /// True when a family with this name (ASCII case-insensitive) is installed.
    pub fn has_family(&self, name: &str) -> bool {
        let system = self.system();
        system.db().faces().any(|face| face.families.iter().any(|(family, _)| family.eq_ignore_ascii_case(name)))
    }
}

impl Default for Fonts {
    fn default() -> Self {
        Fonts::new()
    }
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HAlign {
    /// Flush left.
    #[default]
    Left,
    /// Centred.
    Center,
    /// Flush right.
    Right,
    /// Every wrapped line except the last of a paragraph spreads its spaces to reach both edges, QML `Text.AlignJustify`.
    Justify,
}

impl HAlign {
    pub(crate) fn offset(self, free: f32) -> f32 {
        match self {
            HAlign::Left | HAlign::Justify => 0.0,
            HAlign::Center => free / 2.0,
            HAlign::Right => free,
        }
    }
}

/// Vertical text alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VAlign {
    /// At the top.
    #[default]
    Top,
    /// Centred.
    Center,
    /// At the bottom.
    Bottom,
}

impl VAlign {
    pub(crate) fn offset(self, free: f32) -> f32 {
        match self {
            VAlign::Top => 0.0,
            VAlign::Center => free / 2.0,
            VAlign::Bottom => free,
        }
    }
}

/// Where truncated text gets its ellipsis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Elide {
    /// No eliding.
    #[default]
    None,
    /// At the start.
    Left,
    /// In the middle.
    Middle,
    /// At the end.
    Right,
}

/// Letter case applied to displayed text, QML `font.capitalization`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Capitalization {
    /// The text as written.
    #[default]
    MixedCase,
    /// Every letter in upper case.
    AllUppercase,
    /// Every letter in lower case.
    AllLowercase,
    /// The first letter of every word in upper case, the rest as written.
    Capitalize,
    /// Lower-case letters shown as capitals at 70 % of the font size, as Qt draws them.
    SmallCaps,
}

impl Capitalization {
    /// `text` with the case applied.
    pub fn apply(self, text: &str) -> String {
        match self {
            Capitalization::MixedCase => text.to_owned(),
            Capitalization::AllUppercase | Capitalization::SmallCaps => text.to_uppercase(),
            Capitalization::AllLowercase => text.to_lowercase(),
            Capitalization::Capitalize => {
                let mut out = String::with_capacity(text.len());
                let mut word_start = true;
                for c in text.chars() {
                    if word_start && c.is_alphabetic() {
                        out.extend(c.to_uppercase());
                    } else {
                        out.push(c);
                    }
                    word_start = c.is_whitespace();
                }
                out
            }
        }
    }
}

fn small_caps(text: &str) -> Vec<(String, bool)> {
    let mut runs: Vec<(String, bool)> = Vec::new();
    for c in text.chars() {
        let lower = c.is_lowercase();
        if runs.last().is_none_or(|(_, l)| *l != lower) {
            runs.push((String::new(), lower));
        }
        let run = &mut runs.last_mut().expect("pushed above").0;
        if lower { run.extend(c.to_uppercase()) } else { run.push(c) }
    }
    runs
}

fn small_caps_size(size: f32) -> f32 {
    if size.fract() == 0.0 { ((size * 7.0 + 5.0) / 10.0).floor() } else { size * 0.7 }
}

/// How lines break when text exceeds its width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WrapMode {
    /// No wrapping.
    #[default]
    NoWrap,
    /// At word boundaries.
    Word,
    /// At any character.
    Anywhere,
    /// At word boundaries, falling back to any character for overlong words.
    WordOrAnywhere,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextParams {
    pub size: f32,
    pub weight: u16,
    pub axes: u8,
    pub styled: bool,
    pub max_width: Option<f32>,
    pub wrap: WrapMode,
    pub elide: Elide,
    pub max_lines: usize,
    pub line_height: Option<f32>,
    pub letter_spacing: f32,
    pub capitalization: Capitalization,
    pub justify: bool,
    pub generation: u64,
}

pub(crate) struct ShapedText {
    text: String,
    family: String,
    params: TextParams,
    pub buffer: Buffer,
    pub size: Vec2,
    pub lines: usize,
    pub rtl: bool,
    pub links: Vec<String>,
}

impl ShapedText {
    pub fn new(fonts: &mut FontSystem, text: &Signal<String>, family: &Signal<String>, params: TextParams) -> Self {
        let mut shaped = ShapedText {
            text: String::new(),
            family: String::new(),
            params,
            buffer: Buffer::new_empty(metrics(params.size, params.line_height)),
            size: Vec2::ZERO,
            lines: 0,
            rtl: false,
            links: Vec::new(),
        };
        shaped.text = text.get();
        shaped.family = family.get();
        shaped.reshape(fonts);
        shaped
    }

    pub fn update(
        &mut self,
        fonts: &mut FontSystem,
        text: &Signal<String>,
        family: &Signal<String>,
        params: TextParams,
    ) {
        let text_same = text.with(|t| *t == self.text);
        let family_same = family.with(|f| *f == self.family);
        if text_same && family_same && params == self.params {
            return;
        }
        if !text_same {
            self.text = text.get();
        }
        if !family_same {
            self.family = family.get();
        }
        self.params = params;
        self.reshape(fonts);
    }

    pub fn link_at(&self, point: Vec2, h_align: HAlign, v_align: VAlign, size: Vec2) -> Option<&str> {
        let y = point.y - v_align.offset(size.y - self.size.y);
        let run = self
            .buffer
            .layout_runs()
            .take(self.lines)
            .find(|run| y >= run.line_top && y < run.line_top + run.line_height)?;
        let x = point.x - h_align.offset(size.x - run.line_w);
        let glyph = run.glyphs.iter().find(|g| x >= g.x && x < g.x + g.w)?;
        glyph.metadata.checked_sub(1).and_then(|i| self.links.get(i)).map(String::as_str)
    }

    pub fn caret_positions(&self, text: &str) -> Vec<f32> {
        let glyphs = self.buffer.layout_runs().next().map(|run| run.glyphs.to_vec()).unwrap_or_default();
        let x_of = |byte: usize| -> f32 {
            for g in &glyphs {
                if byte <= g.start {
                    return g.x;
                }
                if byte < g.end {
                    return g.x + g.w * (byte - g.start) as f32 / (g.end - g.start).max(1) as f32;
                }
            }
            glyphs.last().map_or(0.0, |g| g.x + g.w)
        };
        text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).map(x_of).collect()
    }

    fn reshape(&mut self, fonts: &mut FontSystem) {
        let p = self.params;
        let family = if self.family.is_empty() { Family::SansSerif } else { Family::Name(&self.family) };
        let mut attrs = Attrs::new().family(family).weight(Weight(p.weight));
        if p.axes != 0 {
            attrs = attrs.cache_key_flags(crate::axes::flags_for(p.axes));
        }
        if p.letter_spacing != 0.0 {
            attrs = attrs.letter_spacing(p.letter_spacing / p.size.max(1.0));
        }
        self.buffer.set_wrap(match p.wrap {
            WrapMode::NoWrap => Wrap::None,
            WrapMode::Word => Wrap::Word,
            WrapMode::Anywhere => Wrap::Glyph,
            WrapMode::WordOrAnywhere => Wrap::WordOrGlyph,
        });
        let limit = EllipsizeHeightLimit::Lines(if p.max_lines == 0 { 1 } else { p.max_lines });
        let limit = if p.wrap == WrapMode::NoWrap { EllipsizeHeightLimit::Lines(1) } else { limit };
        self.buffer.set_ellipsize(match p.elide {
            Elide::None => Ellipsize::None,
            Elide::Left => Ellipsize::Start(limit),
            Elide::Middle => Ellipsize::Middle(limit),
            Elide::Right => Ellipsize::End(limit),
        });
        self.buffer.set_metrics_and_size(metrics(p.size, p.line_height), p.max_width, None);
        let align = Some(if p.justify { Align::Justified } else { Align::Left });
        let small = p.capitalization == Capitalization::SmallCaps;
        self.links.clear();
        if p.styled || small {
            let segments = if p.styled {
                parse_styled(&self.text)
            } else {
                vec![Segment {
                    text: self.text.clone(),
                    bold: false,
                    italic: false,
                    underline: false,
                    scale: 1.0,
                    link: None,
                }]
            };
            let mut spans = Vec::new();
            for seg in segments {
                let mut a = attrs.clone();
                if seg.bold {
                    a = a.weight(Weight::BOLD);
                }
                if seg.italic {
                    a = a.style(Style::Italic);
                }
                if seg.underline {
                    a = a.underline(UnderlineStyle::Single);
                }
                if let Some(link) = seg.link {
                    self.links.push(link);
                    a = a.metadata(self.links.len());
                }
                let size = p.size.max(1.0) * seg.scale;
                let line_height = metrics(size, p.line_height).line_height;
                if seg.scale != 1.0 {
                    a = a.metrics(Metrics::new(size, line_height));
                }
                if small {
                    let reduced = Metrics::new(small_caps_size(size), line_height);
                    for (text, lower) in small_caps(&seg.text) {
                        spans.push((text, if lower { a.clone().metrics(reduced) } else { a.clone() }));
                    }
                } else {
                    spans.push((p.capitalization.apply(&seg.text), a));
                }
            }
            self.rtl = right_to_left(&spans.iter().map(|(text, _)| text.as_str()).collect::<String>());
            self.buffer.set_rich_text(
                spans.iter().map(|(text, a)| (text.as_str(), a.clone())),
                &attrs,
                Shaping::Advanced,
                align,
            );
        } else {
            let shown = p.capitalization.apply(&self.text);
            self.rtl = right_to_left(&shown);
            self.buffer.set_text(&shown, &attrs, Shaping::Advanced, align);
        }
        self.buffer.shape_until_scroll(fonts, false);
        let mut w: f32 = 0.0;
        let mut h: f32 = 0.0;
        let mut lines = 0;
        for run in self.buffer.layout_runs() {
            if p.max_lines > 0 && lines >= p.max_lines {
                break;
            }
            lines += 1;
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        self.size = Vec2::new(w.ceil(), h.ceil());
        self.lines = lines;
    }
}

fn right_to_left(text: &str) -> bool {
    unicode_bidi::get_base_direction_full(text) == unicode_bidi::Direction::Rtl
}

fn metrics(size: f32, line_height: Option<f32>) -> Metrics {
    let size = size.max(1.0);
    let base = (size * 1.25).round();
    Metrics::new(size, line_height.map_or(base, |m| (base * m.max(0.0)).max(1.0)))
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub scale: f32,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct Format {
    bold: bool,
    italic: bool,
    underline: bool,
    scale: Option<f32>,
    link: Option<String>,
}

fn decode_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{a0}'),
        _ => None,
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(at) = lower[from..].find(name).map(|i| i + from) {
        from = at + name.len();
        let before = lower[..at].chars().next_back();
        let rest = tag[from..].trim_start();
        if before.is_some_and(char::is_whitespace)
            && let Some(value) = rest.strip_prefix('=')
        {
            let value = value.trim_start();
            return Some(match value.chars().next() {
                Some(q @ ('"' | '\'')) => value[1..].split(q).next().unwrap_or("").to_owned(),
                _ => value.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_owned(),
            });
        }
    }
    None
}

#[derive(Clone, Copy)]
enum Marker {
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    Bullet,
    Disc,
    Square,
}

impl Marker {
    fn label(self, n: usize) -> String {
        let alpha = |base: u8| {
            let mut out = Vec::new();
            let mut c = n;
            while c > 0 {
                c -= 1;
                out.insert(0, (base + (c % 26) as u8) as char);
                c /= 26;
            }
            out.into_iter().collect::<String>()
        };
        match self {
            Marker::Decimal => format!("{n}."),
            Marker::LowerAlpha => format!("{}.", alpha(b'a')),
            Marker::UpperAlpha => format!("{}.", alpha(b'A')),
            Marker::LowerRoman => format!("{}.", roman(n)),
            Marker::UpperRoman => format!("{}.", roman(n).to_uppercase()),
            Marker::Bullet => "\u{2022}".to_owned(),
            Marker::Disc => "\u{25e6}".to_owned(),
            Marker::Square => "\u{25a1}".to_owned(),
        }
    }
}

fn roman(mut n: usize) -> String {
    if n >= 5000 {
        return "?".to_owned();
    }
    let table = [
        (1000, "m"),
        (900, "cm"),
        (500, "d"),
        (400, "cd"),
        (100, "c"),
        (90, "xc"),
        (50, "l"),
        (40, "xl"),
        (10, "x"),
        (9, "ix"),
        (5, "v"),
        (4, "iv"),
        (1, "i"),
    ];
    let mut out = String::new();
    for (value, digits) in table {
        while n >= value {
            out.push_str(digits);
            n -= value;
        }
    }
    out
}

const HEADING_SCALES: [f32; 6] = [2.0, 1.5, 1.2, 1.0, 0.8, 0.7];

struct Styler {
    segments: Vec<Segment>,
    current: String,
    formats: Vec<Format>,
    lists: Vec<(Marker, usize)>,
    has_new_line: bool,
    has_space: bool,
    prepend_space: bool,
}

impl Styler {
    fn format(&self) -> Format {
        self.formats.last().cloned().unwrap_or_default()
    }

    fn flush(&mut self) {
        if self.current.is_empty() {
            return;
        }
        let f = self.format();
        self.segments.push(Segment {
            text: std::mem::take(&mut self.current),
            bold: f.bold,
            italic: f.italic,
            underline: f.underline,
            scale: f.scale.unwrap_or(1.0),
            link: f.link,
        });
    }

    fn append(&mut self, text: &str) {
        if self.prepend_space {
            self.current.push(' ');
        }
        self.current.push_str(text);
        self.prepend_space = false;
        self.has_space = false;
        self.has_new_line = false;
    }

    fn line_break_unless_new(&mut self) {
        if !self.has_new_line {
            self.current.push('\n');
        }
    }

    fn open(&mut self, tag: &str, name: &str) {
        let mut f = self.format();
        let push = match name {
            "b" | "strong" => {
                f.bold = true;
                true
            }
            "i" => {
                f.italic = true;
                true
            }
            "u" => {
                f.underline = true;
                true
            }
            "s" | "del" | "pre" | "font" => true,
            "br" => {
                self.current.push('\n');
                self.has_space = true;
                self.prepend_space = false;
                false
            }
            "p" => {
                self.line_break_unless_new();
                self.has_space = true;
                self.prepend_space = false;
                false
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.line_break_unless_new();
                self.has_space = true;
                self.prepend_space = false;
                f.scale = Some(HEADING_SCALES[usize::from(name.as_bytes()[1] - b'1')]);
                f.bold = true;
                true
            }
            "ul" | "ol" => {
                let kind = attribute(tag, "type");
                let marker = match (name, kind.as_deref()) {
                    ("ol", Some("a")) => Marker::LowerAlpha,
                    ("ol", Some("A")) => Marker::UpperAlpha,
                    ("ol", Some("i")) => Marker::LowerRoman,
                    ("ol", Some("I")) => Marker::UpperRoman,
                    ("ol", _) => Marker::Decimal,
                    (_, Some(k)) if k.eq_ignore_ascii_case("disc") => Marker::Disc,
                    (_, Some(k)) if k.eq_ignore_ascii_case("square") => Marker::Square,
                    _ => Marker::Bullet,
                };
                self.lists.push((marker, 0));
                false
            }
            "li" => {
                if !self.has_new_line {
                    self.current.push('\n');
                }
                let depth = self.lists.len();
                if let Some((marker, count)) = self.lists.last_mut() {
                    *count += 1;
                    let label = marker.label(*count);
                    self.current.extend(std::iter::repeat_n('\u{a0}', 6 * depth));
                    self.current.push_str(&label);
                    self.current.push_str("\u{a0}\u{a0}");
                }
                false
            }
            "a" => {
                let href = attribute(tag, "href");
                let valid = href.is_some() || attribute(tag, "title").is_some();
                if let Some(href) = href {
                    f.link = Some(href);
                    f.underline = true;
                }
                valid
            }
            _ => false,
        };
        if push {
            self.flush();
            self.formats.push(f);
        }
    }

    fn close(&mut self, name: &str) {
        self.has_new_line = false;
        let pop = match name {
            "b" | "i" | "a" | "u" | "font" | "strong" | "del" | "s" => true,
            "p" => {
                self.current.push('\n');
                self.has_new_line = true;
                self.has_space = true;
                false
            }
            "pre" => {
                self.current.push('\n');
                self.has_new_line = true;
                self.has_space = true;
                true
            }
            "ul" | "ol" => {
                if self.lists.pop().is_some() && self.lists.is_empty() {
                    self.current.push('\n');
                }
                false
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                self.current.push('\n');
                self.has_new_line = true;
                self.has_space = true;
                true
            }
            _ => false,
        };
        if pop && !self.formats.is_empty() {
            self.flush();
            self.formats.pop();
        }
    }
}

pub(crate) fn parse_styled(input: &str) -> Vec<Segment> {
    let mut s = Styler {
        segments: Vec::new(),
        current: String::new(),
        formats: Vec::new(),
        lists: Vec::new(),
        has_new_line: true,
        has_space: true,
        prepend_space: false,
    };
    let mut rest = input;
    let mut word = String::new();
    while let Some(ch) = rest.chars().next() {
        let tail = &rest[ch.len_utf8()..];
        if ch == '<'
            && let Some(end) = tail.find('>')
        {
            let tag = tail[..end].trim();
            let closing = tag.starts_with('/');
            let name = tag
                .trim_start_matches('/')
                .trim_end_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()) {
                if !word.is_empty() {
                    s.append(&std::mem::take(&mut word));
                } else if s.prepend_space {
                    s.current.push(' ');
                    s.prepend_space = false;
                    s.has_space = true;
                }
                if closing {
                    s.close(&name)
                } else {
                    s.open(tag, &name)
                }
                rest = &tail[end + 1..];
                continue;
            }
        } else if ch == '&'
            && let Some(end) = tail.find(';').filter(|e| *e <= 6)
            && let Some(decoded) = decode_entity(&tail[..end])
        {
            s.append(&std::mem::take(&mut word));
            s.current.push(decoded);
            rest = &tail[end + 1..];
            continue;
        } else if ch.is_whitespace() {
            if !word.is_empty() {
                s.append(&std::mem::take(&mut word));
            }
            s.prepend_space = !s.has_space;
            s.has_new_line = false;
            rest = tail.trim_start();
            continue;
        }
        word.push(ch);
        rest = tail;
    }
    if !word.is_empty() {
        s.append(&word);
    }
    s.flush();
    s.segments
}

#[cfg(test)]
mod styled_tests {
    use super::*;

    fn seg(text: &str, bold: bool, italic: bool) -> Segment {
        Segment { text: text.to_owned(), bold, italic, underline: false, scale: 1.0, link: None }
    }

    fn text(input: &str) -> String {
        parse_styled(input).into_iter().map(|s| s.text).collect()
    }

    #[test]
    fn plain_text_is_one_segment() {
        assert_eq!(parse_styled("hello world"), vec![seg("hello world", false, false)]);
    }

    #[test]
    fn bold_and_italic_nest() {
        assert_eq!(
            parse_styled("a <b>b<i>c</i></b> d"),
            vec![seg("a ", false, false), seg("b", true, false), seg("c", true, true), seg(" d", false, false)]
        );
    }

    #[test]
    fn unknown_tags_are_stripped_and_br_breaks_lines() {
        assert_eq!(parse_styled("<span>word</span><br/>next"), vec![seg("word\nnext", false, false)]);
    }

    #[test]
    fn entities_decode_and_stray_brackets_stay() {
        assert_eq!(parse_styled("1 &lt; 2 &amp; 3 > 2 <3 x"), vec![seg("1 < 2 & 3 > 2 <3 x", false, false)]);
    }

    #[test]
    fn whitespace_collapses_like_qt() {
        assert_eq!(text("  a \n\t b  <b> c</b>  "), "a b c");
        assert_eq!(text("<p>one</p>\n<p>two</p>"), "one\n\ntwo\n");
        assert_eq!(text("<p>one</p><p>two</p>"), "one\ntwo\n");
    }

    #[test]
    fn links_are_underlined_and_keep_their_href() {
        let segments =
            parse_styled("see <a href=\"https://x.y/a?b=1\" title='t'>the docs</a> or <a href=next>more</a>");
        assert_eq!(segments.len(), 4);
        assert_eq!(segments[1].text, "the docs");
        assert_eq!(segments[1].link.as_deref(), Some("https://x.y/a?b=1"));
        assert!(segments[1].underline && !segments[2].underline);
        assert_eq!(segments[3].link.as_deref(), Some("next"));
    }

    #[test]
    fn headings_scale_and_embolden_on_their_own_line() {
        let segments = parse_styled("intro<h1>Title</h1>body<h6>small</h6>");
        assert_eq!(text("intro<h1>Title</h1>body<h6>small</h6>"), "intro\nTitle\nbody\nsmall\n");
        let title = segments.iter().find(|s| s.text.starts_with("Title")).unwrap();
        assert!(title.bold && title.scale == 2.0);
        assert_eq!(segments.iter().find(|s| s.text.starts_with("small")).unwrap().scale, 0.7);
        assert_eq!(segments.iter().find(|s| s.text.contains("body")).unwrap().scale, 1.0);
        let h3 = parse_styled("<h3>a</h3>");
        assert_eq!(h3[0].scale, 1.2);
    }

    #[test]
    fn lists_number_and_indent_like_qt() {
        let pad = |n: usize| "\u{a0}".repeat(n);
        assert_eq!(
            text("<ul><li>a</li><li>b</li></ul>after"),
            format!("{}\u{2022}{}a\n{}\u{2022}{}b\nafter", pad(6), pad(2), pad(6), pad(2))
        );
        assert_eq!(
            text("<ol type=\"I\"><li>x<li>y<ol type=a><li>z</ol></ol>"),
            format!("{}I.{}x\n{}II.{}y\n{}a.{}z", pad(6), pad(2), pad(6), pad(2), pad(12), pad(2)) + "\n"
        );
        assert_eq!(text("<ul type=square><li>s</ul>"), format!("{}\u{25a1}{}s\n", pad(6), pad(2)));
        assert_eq!(roman(1994), "mcmxciv");
        assert_eq!(roman(4000), "mmmm");
        assert_eq!(Marker::UpperAlpha.label(28), "AB.");
    }
}

#[cfg(test)]
mod bidi_tests {
    use super::*;

    fn shaped(text: &str) -> ShapedText {
        let params = TextParams {
            size: 16.0,
            weight: 400,
            axes: 0,
            styled: false,
            max_width: None,
            wrap: WrapMode::NoWrap,
            elide: Elide::None,
            max_lines: 0,
            line_height: None,
            letter_spacing: 0.0,
            capitalization: Capitalization::MixedCase,
            justify: false,
            generation: 0,
        };
        let fonts = Fonts::new();
        let mut system = fonts.system();
        ShapedText::new(&mut system, &Signal::new(text.to_owned()), &Signal::new(String::new()), params)
    }

    fn visual_starts(shaped: &ShapedText) -> Vec<usize> {
        let run = shaped.buffer.layout_runs().next().unwrap();
        let mut glyphs: Vec<_> = run.glyphs.iter().map(|g| (g.x, g.start)).collect();
        glyphs.sort_by(|a, b| a.0.total_cmp(&b.0));
        glyphs.into_iter().map(|(_, start)| start).collect()
    }

    #[test]
    fn base_direction_comes_from_the_first_strong_character() {
        assert!(!shaped("abc שלום").rtl);
        assert!(shaped("שלום abc").rtl);
        assert!(shaped("12, שלום").rtl);
        assert!(shaped("\u{2067}abc\u{2069}\nمرحبا").rtl);
        assert!(!shaped("12 34").rtl);
    }

    #[test]
    fn mixed_runs_are_reordered_visually() {
        let ltr = shaped("ab אבג cd");
        let starts = visual_starts(&ltr);
        let hebrew: Vec<usize> = starts.iter().copied().filter(|s| (3..9).contains(s)).collect();
        assert_eq!(hebrew, vec![7, 5, 3]);
        assert_eq!(starts.first(), Some(&0));
        assert_eq!(starts.last(), Some(&(ltr.text.len() - 1)));

        let rtl = shaped("אבג ab");
        let starts = visual_starts(&rtl);
        assert_eq!(&starts[..2], &[7, 8]);
        assert_eq!(starts.last(), Some(&0));
        assert!(
            rtl.buffer.layout_runs().all(|run| run.glyphs.iter().all(|g| g.x >= -0.5 && g.x + g.w <= run.line_w + 0.5))
        );
    }
}

#[cfg(test)]
mod capitalization_tests {
    use super::Capitalization::*;

    #[test]
    fn cases_apply_like_qml() {
        assert_eq!(MixedCase.apply("Hello wORLD"), "Hello wORLD");
        assert_eq!(AllUppercase.apply("straße"), "STRASSE");
        assert_eq!(AllLowercase.apply("Hello WORLD"), "hello world");
        assert_eq!(Capitalize.apply("hello  wORLD, ünder 3d"), "Hello  WORLD, Ünder 3d");
    }

    #[test]
    fn small_caps_split_lower_case_runs_like_qt() {
        assert_eq!(
            super::small_caps("Hello, wORLD"),
            vec![
                ("H".to_owned(), false),
                ("ELLO".to_owned(), true),
                (", ".to_owned(), false),
                ("W".to_owned(), true),
                ("ORLD".to_owned(), false)
            ]
        );
        assert_eq!(super::small_caps_size(16.0), 11.0);
        assert!((super::small_caps_size(10.5) - 7.35).abs() < 1e-5);
    }
}
