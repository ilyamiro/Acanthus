use std::cell::{RefCell, RefMut};
use std::path::Path;
use std::rc::Rc;

use acanthus_core::{Signal, Vec2};
use cosmic_text::{
    Attrs, Buffer, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics, Shaping, Style, Weight, Wrap,
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
}

impl HAlign {
    pub(crate) fn offset(self, free: f32) -> f32 {
        match self {
            HAlign::Left => 0.0,
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

/// Letter case applied to displayed text, QML `font.capitalization` without `SmallCaps`.
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
}

impl Capitalization {
    /// `text` with the case applied.
    pub fn apply(self, text: &str) -> String {
        match self {
            Capitalization::MixedCase => text.to_owned(),
            Capitalization::AllUppercase => text.to_uppercase(),
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
    pub generation: u64,
}

pub(crate) struct ShapedText {
    text: String,
    family: String,
    params: TextParams,
    pub buffer: Buffer,
    pub size: Vec2,
    pub lines: usize,
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
        if p.styled {
            let segments: Vec<Segment> = parse_styled(&self.text)
                .into_iter()
                .map(|seg| Segment { text: p.capitalization.apply(&seg.text), ..seg })
                .collect();
            let spans = segments.iter().map(|seg| {
                let mut a = attrs.clone();
                if seg.bold {
                    a = a.weight(Weight::BOLD);
                }
                if seg.italic {
                    a = a.style(Style::Italic);
                }
                (seg.text.as_str(), a)
            });
            self.buffer.set_rich_text(spans, &attrs, Shaping::Advanced, None);
        } else {
            let shown = p.capitalization.apply(&self.text);
            self.buffer.set_text(&shown, &attrs, Shaping::Advanced, None);
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

fn metrics(size: f32, line_height: Option<f32>) -> Metrics {
    let size = size.max(1.0);
    let base = (size * 1.25).round();
    Metrics::new(size, line_height.map_or(base, |m| (base * m.max(0.0)).max(1.0)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Segment {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
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

pub(crate) fn parse_styled(input: &str) -> Vec<Segment> {
    let mut segments: Vec<Segment> = Vec::new();
    let (mut bold, mut italic) = (0u32, 0u32);
    let mut current = String::new();
    let flush = |current: &mut String, segments: &mut Vec<Segment>, bold: u32, italic: u32| {
        if !current.is_empty() {
            segments.push(Segment { text: std::mem::take(current), bold: bold > 0, italic: italic > 0 });
        }
    };
    let mut rest = input;
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
            let known = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric());
            if known {
                if matches!(name.as_str(), "b" | "strong" | "i" | "em") {
                    flush(&mut current, &mut segments, bold, italic);
                }
                match (name.as_str(), closing) {
                    ("b" | "strong", false) => bold += 1,
                    ("b" | "strong", true) => bold = bold.saturating_sub(1),
                    ("i" | "em", false) => italic += 1,
                    ("i" | "em", true) => italic = italic.saturating_sub(1),
                    ("br", _) => current.push('\n'),
                    _ => {}
                }
                rest = &tail[end + 1..];
                continue;
            }
        } else if ch == '&'
            && let Some(end) = tail.find(';').filter(|e| *e <= 6)
            && let Some(decoded) = decode_entity(&tail[..end])
        {
            current.push(decoded);
            rest = &tail[end + 1..];
            continue;
        }
        current.push(ch);
        rest = tail;
    }
    flush(&mut current, &mut segments, bold, italic);
    segments
}

#[cfg(test)]
mod styled_tests {
    use super::*;

    fn seg(text: &str, bold: bool, italic: bool) -> Segment {
        Segment { text: text.to_owned(), bold, italic }
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
    fn other_tags_are_stripped_and_br_breaks_lines() {
        assert_eq!(parse_styled("<a href=\"https://x.y\">link</a><br/>next"), vec![seg("link\nnext", false, false)]);
    }

    #[test]
    fn entities_decode_and_stray_brackets_stay() {
        assert_eq!(parse_styled("1 &lt; 2 &amp; 3 > 2 <3 x"), vec![seg("1 < 2 & 3 > 2 <3 x", false, false)]);
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
}
