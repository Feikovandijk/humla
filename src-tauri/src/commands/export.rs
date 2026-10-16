//! Note export (issue #18). Assembles a note's selected content — summary,
//! transcript, typed notes — into a single Markdown, plain-text or PDF
//! document and writes it to a user-chosen path.
//!
//! The path is chosen on the frontend via the dialog plugin's native save
//! panel and passed in as `spec.path`. Unlike `open_in_finder`, the write is
//! intentionally NOT constrained to the app data directory — the whole point
//! is to let the user save anywhere they picked in the panel.

use super::err;
use crate::db;
use crate::export_doc::{self, Block};
use crate::html_text::html_to_text;
use crate::AppState;
use serde::Deserialize;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Markdown,
    Txt,
    Pdf,
}

/// What to export and how. Mirrors the frontend `ExportSpec` type. `path` is
/// the destination chosen in the native save panel.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSpec {
    pub path: String,
    pub format: ExportFormat,
    pub include_summary: bool,
    pub include_transcript: bool,
    pub include_notes: bool,
    /// When false, the leading `Label: ` prefix is stripped from each
    /// transcript line so the export reads as plain prose.
    pub include_speaker_labels: bool,
}

/// A section's content, before it is rendered for a format.
enum Content {
    /// The summary, which the model writes as Markdown.
    Markdown(String),
    Transcript(String),
    /// The typed notes, already reduced to plain text.
    Notes(String),
}

impl Content {
    fn raw(&self) -> &str {
        match self {
            Content::Markdown(s) | Content::Transcript(s) | Content::Notes(s) => s,
        }
    }

    fn blocks(&self, labelled: bool) -> Vec<Block> {
        match self {
            Content::Markdown(s) => export_doc::parse_markdown(s),
            Content::Transcript(s) => export_doc::transcript_turns(s, labelled),
            Content::Notes(s) => export_doc::plain_lines(s),
        }
    }
}

fn display_title(title: &str) -> &str {
    if title.trim().is_empty() { "Untitled note" } else { title.trim() }
}

fn sections(summary: &str, transcript: &str, body_html: &str, spec: &ExportSpec) -> Vec<(&'static str, Content)> {
    let mut out = Vec::new();
    if spec.include_summary {
        out.push(("Summary", Content::Markdown(summary.trim().to_string())));
    }
    if spec.include_transcript {
        let body = if spec.include_speaker_labels {
            transcript.trim().to_string()
        } else {
            transcript
                .lines()
                .map(|l| export_doc::split_label(l).map_or(l, |(_, text)| text))
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        };
        out.push(("Transcript", Content::Transcript(body)));
    }
    if spec.include_notes {
        out.push(("Notes", Content::Notes(html_to_text(body_html).trim().to_string())));
    }
    out
}

/// Markdown uses ATX `#`/`##`; plain text underlines the text with `=`
/// (title) or `-` (section) so the structure survives without markup.
fn heading(markdown: bool, level: u8, text: &str) -> String {
    if markdown {
        return format!("{} {}", "#".repeat(level as usize), text);
    }
    let bar = if level <= 1 { '=' } else { '-' };
    let underline: String = std::iter::repeat(bar).take(text.chars().count().max(1)).collect();
    format!("{text}\n{underline}")
}

/// Assemble a Markdown or plain-text export. Pure — no I/O, so the whole
/// content contract is unit-testable without a database or a filesystem.
/// `body_html` is the raw Tiptap HTML. A checked-but-empty section still
/// emits its heading with an explicit empty marker, so the user sees it was
/// included and had no content.
pub(crate) fn build_export_document(
    title: &str,
    summary: &str,
    transcript: &str,
    body_html: &str,
    spec: &ExportSpec,
) -> String {
    let markdown = spec.format == ExportFormat::Markdown;
    let rendered: Vec<String> = sections(summary, transcript, body_html, spec)
        .into_iter()
        .map(|(name, content)| {
            let body = if content.raw().is_empty() {
                if markdown { "_(empty)_" } else { "(empty)" }.to_string()
            } else if markdown || matches!(content, Content::Transcript(_)) {
                content.raw().to_string()
            } else {
                export_doc::render_text(&content.blocks(false))
            };
            format!("{}\n\n{body}", heading(markdown, 2, name))
        })
        .collect();

    let mut doc = heading(markdown, 1, display_title(title));
    if !rendered.is_empty() {
        doc.push_str("\n\n");
        doc.push_str(&rendered.join("\n\n"));
    }
    doc.push('\n');
    doc
}

/// Lay out a PDF export from the same sections the text formats use.
pub(crate) fn build_export_pdf(
    title: &str,
    summary: &str,
    transcript: &str,
    body_html: &str,
    spec: &ExportSpec,
) -> Result<Vec<u8>, String> {
    let sections: Vec<export_doc::Section> = sections(summary, transcript, body_html, spec)
        .into_iter()
        .map(|(name, content)| export_doc::Section {
            name: name.to_string(),
            blocks: content.blocks(spec.include_speaker_labels),
        })
        .collect();
    export_doc::render_pdf(display_title(title), &sections)
}

/// Export a note to the path chosen in the native save panel.
#[tauri::command]
pub fn export_note(app: AppHandle, note_id: String, spec: ExportSpec) -> Result<(), String> {
    let state: tauri::State<AppState> = app.state();
    let note = {
        let conn = state.db.lock();
        db::get_note(&conn, &note_id).map_err(err)?
    };
    let bytes = match spec.format {
        ExportFormat::Pdf => {
            build_export_pdf(&note.title, &note.summary, &note.transcript, &note.body, &spec)?
        }
        ExportFormat::Markdown | ExportFormat::Txt => {
            build_export_document(&note.title, &note.summary, &note.transcript, &note.body, &spec)
                .into_bytes()
        }
    };
    std::fs::write(&spec.path, bytes).map_err(|e| format!("write export file: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(format: ExportFormat) -> ExportSpec {
        ExportSpec {
            path: "/tmp/x".into(),
            format,
            include_summary: true,
            include_transcript: true,
            include_notes: false,
            include_speaker_labels: true,
        }
    }

    #[test]
    fn markdown_has_title_and_section_headers() {
        let s = spec(ExportFormat::Markdown);
        let out = build_export_document(
            "Weekly sync",
            "- Ship v1",
            "You: hello\nSpeaker 1: hi",
            "",
            &s,
        );
        assert!(out.starts_with("# Weekly sync\n"), "title heading: {out:?}");
        assert!(out.contains("## Summary\n\n- Ship v1"));
        assert!(out.contains("## Transcript\n\nYou: hello\nSpeaker 1: hi"));
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn plain_text_underlines_headings() {
        let s = spec(ExportFormat::Txt);
        let out = build_export_document("Sync", "Done", "You: hey", "", &s);
        assert!(out.starts_with("Sync\n====\n"), "title underline: {out:?}");
        assert!(out.contains("Summary\n-------\n\nDone"));
    }

    #[test]
    fn plain_text_carries_no_markdown() {
        let mut s = spec(ExportFormat::Txt);
        s.include_notes = true;
        let out = build_export_document(
            "Sync",
            "## Beslutninger\n\n- **Eier:** Hege\n- Frist *fredag*",
            "You: hey",
            "<ul><li>typed</li></ul>",
            &s,
        );
        assert!(
            out.contains("Summary\n-------\n\nBeslutninger\n\n• Eier: Hege\n• Frist fredag\n\n"),
            "{out:?}"
        );
        assert!(out.contains("Transcript\n----------\n\nYou: hey"), "{out:?}");
        assert!(out.contains("Notes\n-----\n\n• typed\n"), "{out:?}");
        assert!(!out.contains(['#', '*']), "{out:?}");
    }

    #[test]
    fn pdf_export_is_a_pdf() {
        let mut s = spec(ExportFormat::Pdf);
        s.include_notes = true;
        let pdf = build_export_pdf("Sync", "- **Ship** v1", "You: hey", "<p>typed</p>", &s).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
    }

    #[test]
    fn labels_stripped_when_toggle_off() {
        let mut s = spec(ExportFormat::Markdown);
        s.include_summary = false;
        s.include_speaker_labels = false;
        let out = build_export_document(
            "T",
            "",
            "Speaker 1: hello there\nMichael: how are you",
            "",
            &s,
        );
        assert!(out.contains("## Transcript\n\nhello there\nhow are you"), "{out:?}");
        assert!(!out.contains("Speaker 1:"));
        assert!(!out.contains("Michael:"));
    }

    #[test]
    fn labels_kept_when_toggle_on() {
        let mut s = spec(ExportFormat::Markdown);
        s.include_summary = false;
        s.include_speaker_labels = true;
        let out = build_export_document("T", "", "Speaker 1: hello", "", &s);
        assert!(out.contains("Speaker 1: hello"));
    }

    #[test]
    fn mid_sentence_colon_is_preserved_when_stripping() {
        // A spoken colon far into the line is not a label — leave the line intact
        // when it has no short leading `Label: ` segment... but our heuristic
        // strips the first short prefix. Guard the realistic case: a genuine
        // label followed by text containing a colon keeps the tail's colon.
        let mut s = spec(ExportFormat::Markdown);
        s.include_summary = false;
        s.include_speaker_labels = false;
        let out = build_export_document("T", "", "You: the plan is: ship it", "", &s);
        assert!(out.contains("the plan is: ship it"), "{out:?}");
    }

    #[test]
    fn notes_converted_from_html() {
        let mut s = spec(ExportFormat::Markdown);
        s.include_summary = false;
        s.include_transcript = false;
        s.include_notes = true;
        let out = build_export_document(
            "T",
            "",
            "",
            "<p>First para</p><ul><li>one</li><li>two</li></ul>",
            &s,
        );
        assert!(out.contains("## Notes"));
        assert!(out.contains("First para"));
        assert!(out.contains("- one"));
        assert!(out.contains("- two"));
        assert!(!out.contains("<p>"), "html should be stripped: {out:?}");
    }

    #[test]
    fn empty_checked_section_shows_placeholder() {
        let mut s = spec(ExportFormat::Markdown);
        s.include_transcript = false;
        // summary included but empty
        let out = build_export_document("T", "   ", "", "", &s);
        assert!(out.contains("## Summary\n\n_(empty)_"), "{out:?}");
    }

    #[test]
    fn empty_title_falls_back() {
        let s = spec(ExportFormat::Markdown);
        let out = build_export_document("", "s", "t", "", &s);
        assert!(out.starts_with("# Untitled note"));
    }

    #[test]
    fn nothing_selected_yields_just_the_title() {
        let mut s = spec(ExportFormat::Markdown);
        s.include_summary = false;
        s.include_transcript = false;
        s.include_notes = false;
        let out = build_export_document("Solo", "s", "t", "b", &s);
        assert_eq!(out, "# Solo\n");
    }

    #[test]
    fn format_deserializes_from_lowercase() {
        let f: ExportFormat = serde_json::from_str("\"markdown\"").unwrap();
        assert_eq!(f, ExportFormat::Markdown);
        let f: ExportFormat = serde_json::from_str("\"txt\"").unwrap();
        assert_eq!(f, ExportFormat::Txt);
    }
}
