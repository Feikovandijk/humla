//! A format-neutral block model of an exported note, and the parsers that
//! build it. The plain-text and PDF exports both render from it, so the two
//! read a summary's Markdown the same way.

mod pdf;
mod text;

pub use pdf::render_pdf;
pub use text::render_text;

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading { level: u8, text: Vec<Span> },
    Paragraph(Vec<Span>),
    /// `start` is the first number of an ordered list, `None` for bullets.
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Quote(Vec<Block>),
    Code(String),
    Rule,
    Table { header: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
}

/// A run of inline text in one style. A `"\n"` inside `text` is a hard line
/// break within its paragraph.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

impl Span {
    pub fn plain(text: impl Into<String>) -> Self {
        Span { text: text.into(), ..Default::default() }
    }

    pub fn bold(text: impl Into<String>) -> Self {
        Span { text: text.into(), bold: true, ..Default::default() }
    }

    fn same_style(&self, other: &Span) -> bool {
        self.bold == other.bold && self.italic == other.italic && self.code == other.code
    }
}

/// One section of the exported document, already turned into blocks.
pub struct Section {
    pub name: String,
    pub blocks: Vec<Block>,
}

pub fn spans_text(spans: &[Span]) -> String {
    spans.iter().map(|s| s.text.as_str()).collect()
}

/// Parse CommonMark plus GFM tables, strikethrough and task lists. Raw HTML is
/// dropped, except `<br>`, which models put inside table cells.
pub fn parse_markdown(src: &str) -> Vec<Block> {
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut b = Builder::default();
    for event in Parser::new_ext(src, opts) {
        b.event(event);
    }
    b.finish()
}

/// Blocks for text that is already plain: one paragraph per line, with runs
/// of `- ` lines (what `html_to_text` makes of a list) as a bulleted list.
pub fn plain_lines(src: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut items: Vec<Vec<Block>> = Vec::new();
    for line in src.lines() {
        let line = line.trim_end();
        if let Some(item) = line.strip_prefix("- ") {
            items.push(vec![Block::Paragraph(vec![Span::plain(item)])]);
            continue;
        }
        if !items.is_empty() {
            blocks.push(Block::List { start: None, items: std::mem::take(&mut items) });
        }
        if !line.trim().is_empty() {
            blocks.push(Block::Paragraph(vec![Span::plain(line)]));
        }
    }
    if !items.is_empty() {
        blocks.push(Block::List { start: None, items });
    }
    blocks
}

/// One paragraph per transcript turn, its speaker label (when `labelled`) in
/// bold.
pub fn transcript_turns(transcript: &str, labelled: bool) -> Vec<Block> {
    transcript
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .map(|line| match split_label(line).filter(|_| labelled) {
            Some((label, text)) => {
                Block::Paragraph(vec![Span::bold(format!("{label}: ")), Span::plain(text)])
            }
            None => Block::Paragraph(vec![Span::plain(line)]),
        })
        .collect()
}

/// Split a `Label: text` transcript line. Only a short single-segment prefix
/// counts as a label, so a colon further into spoken text is left alone.
pub fn split_label(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(": ")?;
    let prefix = &line[..idx];
    if !prefix.is_empty() && prefix.chars().count() <= 40 && !prefix.contains(['\n', '\r']) {
        Some((prefix, &line[idx + 2..]))
    } else {
        None
    }
}

enum Frame {
    Blocks(Vec<Block>),
    List { start: Option<u64>, items: Vec<Vec<Block>> },
    Table { header: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>>, row: Vec<Vec<Span>> },
}

enum InlineKind {
    Paragraph,
    Heading(u8),
    Cell,
}

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    inline: Option<(InlineKind, Vec<Span>)>,
    bold: u32,
    italic: u32,
    /// Open links: the destination and the text seen inside so far.
    links: Vec<(String, String)>,
    code_block: Option<String>,
}

impl Builder {
    fn blocks_mut(&mut self) -> &mut Vec<Block> {
        // Content directly inside a list or table outside an item or cell is
        // not something CommonMark produces; give it a home anyway.
        if !matches!(self.stack.last(), Some(Frame::Blocks(_))) {
            self.stack.push(Frame::Blocks(Vec::new()));
        }
        match self.stack.last_mut() {
            Some(Frame::Blocks(b)) => b,
            _ => unreachable!("a blocks frame was just ensured"),
        }
    }

    fn push_text(&mut self, text: &str, code: bool) {
        if let Some(buf) = self.code_block.as_mut() {
            buf.push_str(text);
            return;
        }
        for (_, seen) in &mut self.links {
            seen.push_str(text);
        }
        let span = Span {
            text: text.to_string(),
            bold: self.bold > 0,
            italic: self.italic > 0,
            code,
        };
        let (_, spans) = self.inline.get_or_insert((InlineKind::Paragraph, Vec::new()));
        match spans.last_mut() {
            Some(last) if last.same_style(&span) => last.text.push_str(&span.text),
            _ => spans.push(span),
        }
    }

    fn flush_inline(&mut self) {
        let Some((kind, mut spans)) = self.inline.take() else { return };
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        spans.retain(|s| !s.text.is_empty());
        match kind {
            InlineKind::Cell => {
                if let Some(Frame::Table { row, .. }) = self.stack.last_mut() {
                    row.push(spans);
                }
            }
            InlineKind::Heading(level) if !spans.is_empty() => {
                self.blocks_mut().push(Block::Heading { level, text: spans })
            }
            InlineKind::Paragraph if !spans.is_empty() => {
                self.blocks_mut().push(Block::Paragraph(spans))
            }
            _ => {}
        }
    }

    fn pop_blocks(&mut self) -> Vec<Block> {
        match self.stack.pop() {
            Some(Frame::Blocks(b)) => b,
            Some(other) => {
                self.stack.push(other);
                Vec::new()
            }
            None => Vec::new(),
        }
    }

    fn event(&mut self, event: Event) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.push_text(&t, false),
            Event::Code(t) => self.push_text(&t, true),
            Event::SoftBreak => self.push_text(" ", false),
            Event::HardBreak => self.push_text("\n", false),
            Event::InlineHtml(h) | Event::Html(h) => {
                let tag = h.trim().to_ascii_lowercase();
                if matches!(tag.as_str(), "<br>" | "<br/>" | "<br />") {
                    self.push_text("\n", false);
                }
            }
            Event::TaskListMarker(done) => self.push_text(if done { "[x] " } else { "[ ] " }, false),
            Event::Rule => {
                self.flush_inline();
                self.blocks_mut().push(Block::Rule);
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                self.flush_inline();
                self.inline = Some((InlineKind::Paragraph, Vec::new()));
            }
            Tag::Heading { level, .. } => {
                self.flush_inline();
                self.inline = Some((InlineKind::Heading(heading_level(level)), Vec::new()));
            }
            Tag::BlockQuote(_) | Tag::Item => {
                self.flush_inline();
                self.stack.push(Frame::Blocks(Vec::new()));
            }
            Tag::List(start) => {
                self.flush_inline();
                self.stack.push(Frame::List { start, items: Vec::new() });
            }
            Tag::CodeBlock(_) => {
                self.flush_inline();
                self.code_block = Some(String::new());
            }
            Tag::Table(_) => {
                self.flush_inline();
                self.stack.push(Frame::Table { header: Vec::new(), rows: Vec::new(), row: Vec::new() });
            }
            Tag::TableCell => self.inline = Some((InlineKind::Cell, Vec::new())),
            Tag::Strong => self.bold += 1,
            Tag::Emphasis => self.italic += 1,
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.links.push((dest_url.to_string(), String::new()))
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::TableCell => self.flush_inline(),
            TagEnd::BlockQuote(_) => {
                self.flush_inline();
                let inner = self.pop_blocks();
                self.blocks_mut().push(Block::Quote(inner));
            }
            TagEnd::Item => {
                self.flush_inline();
                let inner = self.pop_blocks();
                if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                    items.push(inner);
                }
            }
            TagEnd::List(_) => {
                self.flush_inline();
                if let Some(Frame::List { start, items }) = self.stack.pop() {
                    self.blocks_mut().push(Block::List { start, items });
                }
            }
            TagEnd::CodeBlock => {
                if let Some(code) = self.code_block.take() {
                    self.blocks_mut().push(Block::Code(code.trim_end_matches('\n').to_string()));
                }
            }
            TagEnd::TableHead => {
                if let Some(Frame::Table { header, row, .. }) = self.stack.last_mut() {
                    *header = std::mem::take(row);
                }
            }
            TagEnd::TableRow => {
                if let Some(Frame::Table { rows, row, .. }) = self.stack.last_mut() {
                    rows.push(std::mem::take(row));
                }
            }
            TagEnd::Table => {
                if let Some(Frame::Table { header, rows, .. }) = self.stack.pop() {
                    self.blocks_mut().push(Block::Table { header, rows });
                }
            }
            TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
            TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
            TagEnd::Link | TagEnd::Image => {
                if let Some((url, seen)) = self.links.pop() {
                    let bare = url.strip_prefix("mailto:").unwrap_or(&url);
                    if !url.is_empty() && !url.starts_with('#') && seen.trim() != bare {
                        let saved = (self.bold, self.italic);
                        (self.bold, self.italic) = (0, 0);
                        self.push_text(&format!(" ({url})"), false);
                        (self.bold, self.italic) = saved;
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.flush_inline();
        let mut out = Vec::new();
        for frame in self.stack {
            if let Frame::Blocks(b) = frame {
                out.extend(b);
            }
        }
        out
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &str) -> Block {
        Block::Paragraph(vec![Span::plain(text)])
    }

    #[test]
    fn headings_paragraphs_and_emphasis() {
        let blocks = parse_markdown("## Decisions\n\nWe **ship** on *Friday*.");
        assert_eq!(
            blocks,
            vec![
                Block::Heading { level: 2, text: vec![Span::plain("Decisions")] },
                Block::Paragraph(vec![
                    Span::plain("We "),
                    Span::bold("ship"),
                    Span::plain(" on "),
                    Span { text: "Friday".into(), italic: true, ..Default::default() },
                    Span::plain("."),
                ]),
            ]
        );
    }

    #[test]
    fn tight_and_nested_lists() {
        let blocks = parse_markdown("- one\n- two\n  - deep\n\n1. first\n2. second");
        assert_eq!(
            blocks,
            vec![
                Block::List {
                    start: None,
                    items: vec![
                        vec![para("one")],
                        vec![
                            para("two"),
                            Block::List { start: None, items: vec![vec![para("deep")]] },
                        ],
                    ],
                },
                Block::List { start: Some(1), items: vec![vec![para("first")], vec![para("second")]] },
            ]
        );
    }

    #[test]
    fn links_keep_their_destination() {
        let blocks = parse_markdown("See [the doc](https://x.no/a) or <https://y.no>.");
        assert_eq!(
            spans_text(match &blocks[0] {
                Block::Paragraph(s) => s,
                b => panic!("{b:?}"),
            }),
            "See the doc (https://x.no/a) or https://y.no."
        );
    }

    #[test]
    fn tables_and_code() {
        let blocks = parse_markdown("| A | B |\n|---|---|\n| 1 | x<br>y |\n\n```\nlet a = 1;\n```");
        assert_eq!(
            blocks,
            vec![
                Block::Table {
                    header: vec![vec![Span::plain("A")], vec![Span::plain("B")]],
                    rows: vec![vec![vec![Span::plain("1")], vec![Span::plain("x\ny")]]],
                },
                Block::Code("let a = 1;".into()),
            ]
        );
    }

    #[test]
    fn plain_lines_groups_dash_items_into_a_list() {
        let blocks = plain_lines("Intro\n- one\n- two\nOutro");
        assert_eq!(
            blocks,
            vec![
                para("Intro"),
                Block::List { start: None, items: vec![vec![para("one")], vec![para("two")]] },
                para("Outro"),
            ]
        );
    }

    #[test]
    fn transcript_turns_bold_the_label() {
        let blocks = transcript_turns("Michael: the plan is: ship it\nno label here", true);
        assert_eq!(
            blocks,
            vec![
                Block::Paragraph(vec![Span::bold("Michael: "), Span::plain("the plan is: ship it")]),
                para("no label here"),
            ]
        );
        // Once labels are stripped, a spoken colon is not mistaken for one.
        assert_eq!(transcript_turns("the plan is: ship it", false), vec![para("the plan is: ship it")]);
    }
}
