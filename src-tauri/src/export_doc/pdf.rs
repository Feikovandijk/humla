//! Blocks → a paginated A4 PDF set in Hanken Grotesk, Humla's own typeface.
//!
//! Layout happens first, into per-page lists of drawing ops, so the footer can
//! say "page n of m"; krilla then draws them. Lines are measured with the same
//! rustybuzz shaping krilla draws with, so a measured line is the drawn line.

use super::{Block, Section, Span};
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect};
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::Fill;
use krilla::surface::Surface;
use krilla::text::{Font, Tag, TextDirection};
use krilla::Document;

static ROMAN: &[u8] = include_bytes!("../../fonts/HankenGrotesk-Variable.ttf");
static ITALIC: &[u8] = include_bytes!("../../fonts/HankenGrotesk-Italic-Variable.ttf");

const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN_X: f32 = 64.0;
const TOP: f32 = 64.0;
const BOTTOM: f32 = PAGE_H - 76.0;
const CONTENT_W: f32 = PAGE_W - 2.0 * MARGIN_X;

type Rgb = (u8, u8, u8);
const INK: Rgb = (28, 28, 26);
const MUTED: Rgb = (112, 110, 104);
const HAIRLINE: Rgb = (218, 216, 210);
const QUOTE_BAR: Rgb = (205, 202, 194);
const CODE_BG: Rgb = (244, 243, 239);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    Regular,
    Semibold,
    Bold,
    Italic,
    SemiboldItalic,
}

const FACES: [(Face, bool, f32); 5] = [
    (Face::Regular, false, 400.0),
    (Face::Semibold, false, 600.0),
    (Face::Bold, false, 700.0),
    (Face::Italic, true, 400.0),
    (Face::SemiboldItalic, true, 600.0),
];

struct Fonts {
    faces: Vec<(Face, Font, rustybuzz::Face<'static>)>,
}

impl Fonts {
    fn new() -> Result<Self, String> {
        let mut faces = Vec::new();
        for (face, italic, weight) in FACES {
            let data = if italic { ITALIC } else { ROMAN };
            let font = Font::new_variable(data.into(), 0, &[(Tag::new(b"wght"), weight)])
                .ok_or("load export font")?;
            let mut shaper = rustybuzz::Face::from_slice(data, 0).ok_or("load export font")?;
            shaper.set_variation(rustybuzz::ttf_parser::Tag::from_bytes(b"wght"), weight);
            faces.push((face, font, shaper));
        }
        Ok(Fonts { faces })
    }

    fn get(&self, face: Face) -> &(Face, Font, rustybuzz::Face<'static>) {
        self.faces.iter().find(|(f, ..)| *f == face).expect("every face is loaded")
    }

    fn width(&self, face: Face, size: f32, text: &str) -> f32 {
        let (_, _, shaper) = self.get(face);
        let mut buffer = rustybuzz::UnicodeBuffer::new();
        buffer.push_str(text);
        buffer.guess_segment_properties();
        let shaped = rustybuzz::shape(shaper, &[], buffer);
        let units: i32 = shaped.glyph_positions().iter().map(|p| p.x_advance).sum();
        units as f32 * size / shaper.units_per_em() as f32
    }
}

#[derive(Clone, Copy)]
struct TextStyle {
    size: f32,
    line_height: f32,
    color: Rgb,
    /// Headings are semibold, and bold inside one is bold.
    heading: bool,
}

const TITLE: TextStyle = TextStyle { size: 20.0, line_height: 26.0, color: INK, heading: true };
const SECTION: TextStyle = TextStyle { size: 14.0, line_height: 19.0, color: INK, heading: true };
const BODY: TextStyle = TextStyle { size: 10.5, line_height: 15.5, color: INK, heading: false };
const CODE: TextStyle = TextStyle { size: 9.5, line_height: 14.0, color: INK, heading: false };
const FOOTER: TextStyle = TextStyle { size: 8.5, line_height: 12.0, color: MUTED, heading: false };

fn face_for(span: &Span, heading: bool) -> Face {
    match (span.italic, span.bold, heading) {
        (false, false, false) => Face::Regular,
        (false, true, true) => Face::Bold,
        (false, _, _) => Face::Semibold,
        (true, false, false) => Face::Italic,
        (true, _, _) => Face::SemiboldItalic,
    }
}

enum Op {
    Text { x: f32, y: f32, face: Face, size: f32, color: Rgb, text: String },
    Rect { x: f32, y: f32, w: f32, h: f32, color: Rgb },
    Dot { cx: f32, cy: f32, r: f32, color: Rgb },
}

enum Marker {
    Dot { cx: f32, r: f32 },
    Number { right: f32, text: String },
}

/// What is drawn beside every line of a block: quote bars, a code background,
/// a colour that overrides the style's.
#[derive(Clone, Default)]
struct Decor {
    bars: Vec<f32>,
    background: Option<(f32, f32)>,
    color: Option<Rgb>,
}

type Piece = (Face, String);

enum Token {
    Word(Vec<Piece>),
    Break,
}

fn push_piece(pieces: &mut Vec<Piece>, face: Face, text: &str) {
    match pieces.last_mut() {
        Some((f, s)) if *f == face => s.push_str(text),
        _ => pieces.push((face, text.to_string())),
    }
}

/// Split spans into words. Preformatted text keeps its spaces, so each of its
/// lines is one word.
fn tokenize(spans: &[Span], heading: bool, preformatted: bool) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut word: Vec<Piece> = Vec::new();
    for span in spans {
        let face = face_for(span, heading);
        for c in span.text.chars() {
            if c == '\n' {
                if !word.is_empty() {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                }
                tokens.push(Token::Break);
            } else if c.is_whitespace() && !preformatted {
                if !word.is_empty() {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                }
            } else {
                push_piece(&mut word, face, c.encode_utf8(&mut [0; 4]));
            }
        }
    }
    if !word.is_empty() {
        tokens.push(Token::Word(word));
    }
    tokens
}

struct Layout<'f> {
    fonts: &'f Fonts,
    pages: Vec<Vec<Op>>,
    y: f32,
    marker: Option<Marker>,
}

impl<'f> Layout<'f> {
    fn new(fonts: &'f Fonts) -> Self {
        Layout { fonts, pages: vec![Vec::new()], y: TOP, marker: None }
    }

    fn ops(&mut self) -> &mut Vec<Op> {
        self.pages.last_mut().expect("there is always a page")
    }

    fn at_top(&self) -> bool {
        self.y <= TOP + 0.01
    }

    /// Start a new page unless `h` more points fit on this one.
    fn ensure(&mut self, h: f32) {
        if self.y + h > BOTTOM && !self.at_top() {
            self.pages.push(Vec::new());
            self.y = TOP;
        }
    }

    /// Vertical space, which a page break swallows.
    fn space(&mut self, h: f32) {
        if !self.at_top() {
            self.y += h;
        }
    }

    fn wrap(&self, tokens: Vec<Token>, size: f32, width: f32) -> Vec<Vec<Piece>> {
        let w = |face: Face, text: &str| self.fonts.width(face, size, text);
        let mut lines = Vec::new();
        let mut line: Vec<Piece> = Vec::new();
        let mut line_w = 0.0;
        for token in tokens {
            let pieces = match token {
                Token::Break => {
                    lines.push(std::mem::take(&mut line));
                    line_w = 0.0;
                    continue;
                }
                Token::Word(pieces) => pieces,
            };
            let word_w: f32 = pieces.iter().map(|(f, t)| w(*f, t)).sum();
            if let Some((space_face, _)) = line.last() {
                let space_face = *space_face;
                let space_w = w(space_face, " ");
                if line_w + space_w + word_w <= width {
                    push_piece(&mut line, space_face, " ");
                    for (face, text) in &pieces {
                        push_piece(&mut line, *face, text);
                    }
                    line_w += space_w + word_w;
                    continue;
                }
                lines.push(std::mem::take(&mut line));
                line_w = 0.0;
            }
            if word_w <= width {
                line = pieces;
                line_w = word_w;
                continue;
            }
            // A word wider than the line (a long URL) breaks between characters.
            for (face, text) in pieces {
                for c in text.chars() {
                    let s = c.encode_utf8(&mut [0; 4]).to_string();
                    let cw = w(face, &s);
                    if line_w + cw > width && !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                        line_w = 0.0;
                    }
                    push_piece(&mut line, face, &s);
                    line_w += cw;
                }
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    fn draw_pieces(&mut self, pieces: Vec<Piece>, x: f32, baseline: f32, size: f32, color: Rgb) {
        let mut cx = x;
        for (face, text) in pieces {
            let w = self.fonts.width(face, size, &text);
            self.ops().push(Op::Text { x: cx, y: baseline, face, size, color, text });
            cx += w;
        }
    }

    fn baseline(top: f32, style: &TextStyle) -> f32 {
        top + style.line_height * 0.5 + style.size * 0.36
    }

    fn line(&mut self, pieces: Vec<Piece>, x: f32, style: &TextStyle, decor: &Decor) {
        self.ensure(style.line_height);
        self.decorate(style.line_height, decor);
        let baseline = Self::baseline(self.y, style);
        match self.marker.take() {
            Some(Marker::Dot { cx, r }) => {
                let cy = baseline - style.size * 0.32;
                self.ops().push(Op::Dot { cx, cy, r, color: INK });
            }
            Some(Marker::Number { right, text }) => {
                let w = self.fonts.width(Face::Regular, style.size, &text);
                self.draw_pieces(vec![(Face::Regular, text)], right - w, baseline, style.size, INK);
            }
            None => {}
        }
        let color = decor.color.unwrap_or(style.color);
        self.draw_pieces(pieces, x, baseline, style.size, color);
        self.y += style.line_height;
    }

    fn decorate(&mut self, h: f32, decor: &Decor) {
        let y = self.y;
        if let Some((x, w)) = decor.background {
            self.ops().push(Op::Rect { x, y, w, h, color: CODE_BG });
        }
        for &x in &decor.bars {
            self.ops().push(Op::Rect { x, y, w: 2.0, h, color: QUOTE_BAR });
        }
    }

    fn paragraph(&mut self, spans: &[Span], x: f32, w: f32, style: &TextStyle, decor: &Decor, pre: bool) {
        let tokens = tokenize(spans, style.heading, pre);
        let lines = self.wrap(tokens, style.size, w);
        // A paragraph of up to three lines is never split across pages.
        if lines.len() <= 3 {
            self.ensure(lines.len() as f32 * style.line_height);
        }
        for line in lines {
            self.line(line, x, style, decor);
        }
    }

    fn hairline(&mut self, x: f32, y: f32, w: f32) {
        self.ops().push(Op::Rect { x, y, w, h: 0.6, color: HAIRLINE });
    }

    fn blocks(&mut self, blocks: &[Block], x: f32, w: f32, decor: &Decor, tight: bool, depth: usize) {
        for block in blocks {
            self.block(block, x, w, decor, tight, depth);
        }
    }

    fn block(&mut self, block: &Block, x: f32, w: f32, decor: &Decor, tight: bool, depth: usize) {
        match block {
            Block::Heading { level, text } => {
                let size = match level {
                    1 => 12.5,
                    2 => 11.5,
                    _ => BODY.size,
                };
                let style = TextStyle { size, line_height: size * 1.4, ..BODY };
                let style = TextStyle { heading: true, ..style };
                self.space(if tight { 3.0 } else { 7.0 });
                // Keep a heading with the first lines under it.
                self.ensure(style.line_height + BODY.line_height * 2.0);
                self.paragraph(text, x, w, &style, decor, false);
                self.space(3.0);
            }
            Block::Paragraph(spans) => {
                self.paragraph(spans, x, w, &BODY, decor, false);
                self.space(if tight { 2.5 } else { 7.0 });
            }
            Block::List { start, items } => {
                let indent = match start {
                    None => 14.0,
                    Some(n) => {
                        let last = format!("{}.", n + items.len().saturating_sub(1) as u64);
                        self.fonts.width(Face::Regular, BODY.size, &last) + 6.0
                    }
                };
                for (i, item) in items.iter().enumerate() {
                    self.marker = Some(match start {
                        None => Marker::Dot { cx: x + 4.0, r: if depth == 0 { 1.8 } else { 1.4 } },
                        Some(n) => Marker::Number { right: x + indent - 6.0, text: format!("{}.", n + i as u64) },
                    });
                    self.blocks(item, x + indent, w - indent, decor, true, depth + 1);
                }
                self.marker = None;
                if !tight {
                    self.space(4.5);
                }
            }
            Block::Quote(inner) => {
                let mut quoted = decor.clone();
                quoted.bars.push(x + 1.0);
                quoted.color = Some(MUTED);
                self.blocks(inner, x + 14.0, w - 14.0, &quoted, tight, depth);
            }
            Block::Code(code) => {
                let boxed = Decor { background: Some((x, w)), ..decor.clone() };
                self.ensure(CODE.line_height + 8.0);
                self.decorate(4.0, &boxed);
                self.y += 4.0;
                self.paragraph(&[Span::plain(code)], x + 8.0, w - 16.0, &CODE, &boxed, true);
                self.decorate(4.0, &boxed);
                self.y += 4.0;
                self.space(if tight { 2.5 } else { 7.0 });
            }
            Block::Rule => {
                self.ensure(14.0);
                let y = self.y + 6.0;
                self.hairline(x, y, w);
                self.y += 14.0;
            }
            Block::Table { header, rows } => {
                self.table(header, rows, x, w);
                self.space(if tight { 2.5 } else { 7.0 });
            }
        }
    }

    fn table(&mut self, header: &[Vec<Span>], rows: &[Vec<Vec<Span>>], x: f32, w: f32) {
        const PAD: f32 = 6.0;
        let cols = std::iter::once(header).chain(rows.iter().map(Vec::as_slice)).map(<[_]>::len).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let natural = |c: usize| {
            std::iter::once(header)
                .chain(rows.iter().map(Vec::as_slice))
                .filter_map(|row| row.get(c))
                .map(|cell| {
                    let text: String = cell.iter().map(|s| s.text.as_str()).collect();
                    text.lines().map(|l| self.fonts.width(Face::Semibold, BODY.size, l)).fold(0.0, f32::max)
                })
                .fold(0.0, f32::max)
                + 2.0 * PAD
        };
        let natural: Vec<f32> = (0..cols).map(natural).collect();
        let total: f32 = natural.iter().sum();
        let widths: Vec<f32> = if total <= w {
            natural.iter().map(|n| n + (w - total) / cols as f32).collect()
        } else {
            let floor = (w / cols as f32).min(60.0);
            let shrinkable: f32 = natural.iter().map(|n| (n - floor).max(0.0)).sum();
            let excess = total - w;
            natural.iter().map(|n| n - (n - floor).max(0.0) / shrinkable * excess).collect()
        };

        let draw_row = |l: &mut Self, cells: &[Vec<Span>], bold: bool| {
            let wrapped: Vec<Vec<Vec<Piece>>> = (0..cols)
                .map(|c| {
                    let spans: Vec<Span> = cells
                        .get(c)
                        .map(|cell| cell.iter().map(|s| Span { bold: s.bold || bold, ..s.clone() }).collect())
                        .unwrap_or_default();
                    l.wrap(tokenize(&spans, false, false), BODY.size, widths[c] - 2.0 * PAD)
                })
                .collect();
            let lines = wrapped.iter().map(Vec::len).max().unwrap_or(0).max(1);
            let h = lines as f32 * BODY.line_height + 2.0 * 4.0;
            l.ensure(h);
            let mut cx = x;
            for (c, cell) in wrapped.into_iter().enumerate() {
                for (i, pieces) in cell.into_iter().enumerate() {
                    let baseline = Self::baseline(l.y + 4.0 + i as f32 * BODY.line_height, &BODY);
                    l.draw_pieces(pieces, cx + PAD, baseline, BODY.size, INK);
                }
                cx += widths[c];
            }
            l.y += h;
            let y = l.y;
            l.hairline(x, y, w);
        };
        draw_row(self, header, true);
        for row in rows {
            draw_row(self, row, false);
        }
    }
}

fn fill(color: Rgb) -> Option<Fill> {
    Some(Fill {
        paint: rgb::Color::new(color.0, color.1, color.2).into(),
        opacity: NormalizedF32::ONE,
        rule: Default::default(),
    })
}

fn draw(surface: &mut Surface, fonts: &Fonts, op: &Op) {
    match op {
        Op::Text { x, y, face, size, color, text } => {
            if text.trim().is_empty() {
                return;
            }
            surface.set_fill(fill(*color));
            let font = fonts.get(*face).1.clone();
            surface.draw_text(Point::from_xy(*x, *y), font, *size, text, false, TextDirection::Auto);
        }
        Op::Rect { x, y, w, h, color } => {
            let mut path = PathBuilder::new();
            let Some(rect) = Rect::from_xywh(*x, *y, *w, *h) else { return };
            path.push_rect(rect);
            if let Some(path) = path.finish() {
                surface.set_fill(fill(*color));
                surface.draw_path(&path);
            }
        }
        Op::Dot { cx, cy, r, color } => {
            // A circle from four cubic quarter-arcs.
            let k = 0.552_284_8 * r;
            let (cx, cy, r) = (*cx, *cy, *r);
            let mut path = PathBuilder::new();
            path.move_to(cx + r, cy);
            path.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
            path.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
            path.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
            path.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
            path.close();
            if let Some(path) = path.finish() {
                surface.set_fill(fill(*color));
                surface.draw_path(&path);
            }
        }
    }
}

/// Shorten `text` with an ellipsis until it fits `max` points.
fn fit(fonts: &Fonts, face: Face, size: f32, text: &str, max: f32) -> String {
    if fonts.width(face, size, text) <= max {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate = format!("{}…", chars.iter().collect::<String>().trim_end());
        if fonts.width(face, size, &candidate) <= max {
            return candidate;
        }
    }
    String::new()
}

fn layout(fonts: &Fonts, title: &str, sections: &[Section]) -> Vec<Vec<Op>> {
    let mut l = Layout::new(fonts);
    let plain = Decor::default();
    l.paragraph(&[Span::bold(title)], MARGIN_X, CONTENT_W, &TITLE, &plain, false);

    for section in sections {
        l.space(20.0);
        l.ensure(SECTION.line_height + 10.0 + BODY.line_height * 2.0);
        l.paragraph(&[Span::plain(&section.name)], MARGIN_X, CONTENT_W, &SECTION, &plain, false);
        let y = l.y + 3.0;
        l.hairline(MARGIN_X, y, CONTENT_W);
        l.y += 12.0;
        if section.blocks.is_empty() {
            let empty = Span { text: "(empty)".into(), italic: true, ..Default::default() };
            let muted = Decor { color: Some(MUTED), ..Decor::default() };
            l.paragraph(&[empty], MARGIN_X, CONTENT_W, &BODY, &muted, false);
        } else {
            l.blocks(&section.blocks, MARGIN_X, CONTENT_W, &plain, false, 0);
        }
    }
    l.pages
}

pub fn render_pdf(title: &str, sections: &[Section]) -> Result<Vec<u8>, String> {
    let fonts = Fonts::new()?;
    let pages = layout(&fonts, title, sections);
    let mut doc = Document::new();
    doc.set_metadata(Metadata::new().title(title.to_string()).creator("Transcriberr".to_string()));
    let count = pages.len();
    for (i, ops) in pages.iter().enumerate() {
        let settings = PageSettings::from_wh(PAGE_W, PAGE_H).ok_or("page size")?;
        let mut page = doc.start_page_with(settings);
        let mut surface = page.surface();
        for op in ops {
            draw(&mut surface, &fonts, op);
        }
        let baseline = PAGE_H - 40.0;
        let number = format!("{} / {count}", i + 1);
        let number_w = fonts.width(Face::Regular, FOOTER.size, &number);
        let running = fit(&fonts, Face::Regular, FOOTER.size, title, CONTENT_W - number_w - 24.0);
        let footer = [
            Op::Text { x: MARGIN_X, y: baseline, face: Face::Regular, size: FOOTER.size, color: MUTED, text: running },
            Op::Text {
                x: PAGE_W - MARGIN_X - number_w,
                y: baseline,
                face: Face::Regular,
                size: FOOTER.size,
                color: MUTED,
                text: number,
            },
        ];
        for op in &footer {
            draw(&mut surface, &fonts, op);
        }
        surface.finish();
        page.finish();
    }
    doc.finish().map_err(|e| format!("render PDF: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::super::{parse_markdown, transcript_turns};
    use super::*;

    fn fonts() -> Fonts {
        Fonts::new().unwrap()
    }

    #[test]
    fn wrapping_never_exceeds_the_width() {
        let f = fonts();
        let l = Layout::new(&f);
        let text = "Vi ble enige om å lansere den nye eksporten på fredag, etter at Hege har sett over PDF-oppsettet og teksteksporten. https://example.com/a/very/long/path/that/cannot/fit/on/one/line/at/all/whatsoever";
        let lines = l.wrap(tokenize(&[Span::plain(text)], false, false), BODY.size, 200.0);
        assert!(lines.len() > 3, "{}", lines.len());
        for line in &lines {
            let w: f32 = line.iter().map(|(face, t)| f.width(*face, BODY.size, t)).sum();
            assert!(w <= 200.0 + 0.01, "line {w}pt wide: {line:?}");
        }
        let rejoined: String = lines.iter().map(|l| l.iter().map(|(_, t)| t.as_str()).collect::<String>()).collect::<Vec<_>>().join(" ");
        assert_eq!(rejoined.replace(' ', ""), text.replace(' ', ""));
    }

    #[test]
    fn a_long_document_paginates() {
        let transcript: String = (0..300).map(|i| format!("Speaker {}: Dette er replikk nummer {i}.\n", i % 3)).collect();
        let sections = vec![
            Section { name: "Summary".into(), blocks: parse_markdown("## Beslutninger\n\n- **Eksport**: ferdig\n\n| A | B |\n|---|---|\n| 1 | 2 |") },
            Section { name: "Transcript".into(), blocks: transcript_turns(&transcript, true) },
        ];
        let pages = layout(&fonts(), "Ukentlig synk", &sections);
        assert!(pages.len() > 3, "{} pages", pages.len());
        for ops in &pages {
            for op in ops {
                if let Op::Text { y, .. } = op {
                    assert!(*y <= BOTTOM, "text below the bottom margin at {y}");
                }
            }
        }
        let pdf = render_pdf("Ukentlig synk", &sections).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }

    /// Writes a sample to `$HUMLA_EXPORT_SAMPLE` for looking at by eye.
    #[test]
    fn writes_a_sample_when_asked() {
        let Ok(path) = std::env::var("HUMLA_EXPORT_SAMPLE") else { return };
        let summary = "## Sammendrag\n\nTeamet gikk gjennom **eksportfunksjonen** og ble enige om å legge til *PDF* som eget format. Se [notatet](https://humla.team/notes) for detaljer.\n\n### Beslutninger\n\n- Ren tekst skal ikke inneholde Markdown-tegn\n- PDF bruker Hanken Grotesk\n  - med sidetall i bunnteksten\n  - og en tittel i toppen\n- Lanseres i `0.68`\n\n### Oppfølging\n\n1. Michael tester på en lang møtenotat\n2. Hege ser over typografien\n\n| Oppgave | Eier | Frist |\n|---|---|---|\n| Teksteksport | Michael | Fredag |\n| PDF-oppsett | Hege | Neste uke, etter gjennomgang med teamet |\n\n> Et sitat fra møtet som strekker seg over mer enn én linje for å vise at sitatstreken følger teksten nedover.\n\n```\nlet pdf = render_pdf(title, &sections)?;\n```\n\n---\n\nÆØÅ æøå — «sitat» – ferdig.";
        let transcript: String = (0..60)
            .map(|i| match i % 3 {
                0 => format!("Michael: Replikk {i}, der jeg forklarer hvordan eksporten skal fungere når notatet er langt.\n"),
                1 => format!("Hege: Svar {i}. Høres bra ut.\n"),
                _ => format!("Speaker 3: Et lengre innlegg {i} som går over flere linjer for å vise hvordan bryting av tekst ser ut i transkripsjonen.\n"),
            })
            .collect();
        let sections = vec![
            Section { name: "Summary".into(), blocks: parse_markdown(summary) },
            Section { name: "Transcript".into(), blocks: transcript_turns(&transcript, true) },
            Section { name: "Notes".into(), blocks: Vec::new() },
        ];
        std::fs::write(path, render_pdf("Produktmøte: eksport av notater", &sections).unwrap()).unwrap();
    }
}
