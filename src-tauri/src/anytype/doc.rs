//! What a note looks like once it is an Anytype object.
//!
//! The object's name is the note's Title; its body is Markdown assembled from
//! the SAME sections the Markdown export uses (`commands::sections`), so the
//! two surfaces never drift. Two deliberate differences, both because this
//! body is rendered by Anytype rather than saved as a `.md` file:
//!
//!   - no H1 title — the object already shows its own name;
//!   - transcript turns are separated by a blank line — CommonMark folds a
//!     single newline into one paragraph, which would run the whole
//!     conversation together on screen.
//!
//! Everything here is pure: the caller gathers the facts, `build` renders
//! them, and the tests pin the shape without a database or a network.

use crate::commands::{self, ExportFormat, ExportSpec};
use crate::db;

/// What the note row alone can't tell the document: the names its folder and
/// client ids resolve to, and when it was recorded.
pub struct Facts {
    pub folder: Option<String>,
    pub client: Option<String>,
    pub recording: RecordingFacts,
}

#[derive(Default)]
pub struct RecordingFacts {
    /// "12 Oct 2026, 09:00", in the user's local timezone — preformatted by
    /// the caller, so `build` never depends on the machine's clock.
    pub started_label: Option<String>,
    pub duration_ms: Option<u64>,
    pub take_count: usize,
    pub in_progress: bool,
}

/// A note with nothing in it must not be pushed: an object whose three
/// sections all read "empty" is noise in the user's space, not a copy.
pub fn is_empty(note: &db::Note) -> bool {
    note.summary.trim().is_empty()
        && note.transcript.trim().is_empty()
        && crate::html_text::html_to_text(&note.body).trim().is_empty()
}

/// The document body: an optional details block, then every section. The
/// object's name comes from [`commands::display_title`] so an untitled note
/// reads the same here as in the export.
pub fn build(note: &db::Note, facts: &Facts) -> String {
    let mut doc = details(note, facts);
    if !doc.is_empty() {
        doc.push_str("\n\n");
    }
    doc.push_str(&body(&note.summary, &note.transcript, &note.body));
    doc
}

/// The facts a knowledge-base reader wants at a glance, as a bullet list —
/// a plain line per fact would collapse into one paragraph in Markdown.
/// Facts with no value are simply absent.
fn details(note: &db::Note, facts: &Facts) -> String {
    let mut lines = Vec::new();
    if let Some(label) = &facts.recording.started_label {
        let mut line = format!("- **Recorded:** {label}");
        if let Some(ms) = facts.recording.duration_ms {
            line.push_str(&format!(" · {}", duration(ms)));
        }
        if facts.recording.take_count > 1 {
            line.push_str(&format!(" · {} takes", facts.recording.take_count));
        }
        if facts.recording.in_progress {
            line.push_str(" · recording in progress");
        }
        lines.push(line);
    } else if facts.recording.in_progress {
        lines.push("- **Recording:** in progress".to_string());
    }
    if let Some(folder) = facts.folder.as_deref().filter(|f| !f.trim().is_empty()) {
        lines.push(format!("- **Folder:** {}", one_line(folder)));
    }
    if let Some(client) = facts.client.as_deref().filter(|c| !c.trim().is_empty()) {
        lines.push(format!("- **Client:** {}", one_line(client)));
    }
    let speakers = db::transcript_speakers(&note.transcript);
    if !speakers.is_empty() {
        lines.push(format!("- **Speakers:** {}", speakers.join(", ")));
    }
    lines.join("\n")
}

/// A folder or client name is the user's own text; a newline in it would
/// break the list, everything else is theirs to write.
fn one_line(name: &str) -> String {
    name.trim().replace(['\n', '\r'], " ")
}

/// Summary, transcript and notes, in the export's order and under its
/// headings — with the transcript paragraph-joined for the renderer (see the
/// module docs).
fn body(summary: &str, transcript: &str, body_html: &str) -> String {
    let spec = ExportSpec {
        path: String::new(),
        format: ExportFormat::Markdown,
        include_summary: true,
        include_transcript: true,
        include_notes: true,
        include_speaker_labels: true,
    };
    commands::sections(summary, transcript, body_html, &spec)
        .into_iter()
        .map(|(name, content)| {
            let rendered = match &content {
                commands::Content::Transcript(t) if !t.is_empty() => t
                    .lines()
                    .map(str::trim_end)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
                _ if content.raw().is_empty() => "_(empty)_".to_string(),
                _ => content.raw().to_string(),
            };
            format!("{}\n\n{rendered}", commands::heading(true, 2, name))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// "1h 20m" / "42 min" / "45 s" — compact, since it shares a line with the date.
fn duration(ms: u64) -> String {
    let total_min = ms / 60_000;
    if total_min >= 60 {
        let (h, m) = (total_min / 60, total_min % 60);
        if m == 0 { format!("{h}h") } else { format!("{h}h {m}m") }
    } else if total_min >= 1 {
        format!("{total_min} min")
    } else {
        format!("{} s", ms / 1000)
    }
}

/// FNV-1a over the inputs that decide what Anytype should show: where the
/// object lives, its type and name, and the body. Stable across Rust versions
/// by construction, so a hash computed before an app update still matches
/// after — `DefaultHasher` guarantees no such thing.
pub fn hash(space_id: &str, type_key: &str, name: &str, body: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in [space_id, type_key, name, body] {
        for b in part.as_bytes() {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note() -> db::Note {
        db::Note {
            id: "n1".into(),
            title: "Weekly planning".into(),
            body: "<p>Confirm the launch date.</p>".into(),
            transcript: "You: Are we ready?\nAlex: Yes, the 24th.".into(),
            summary: "- Launch confirmed for the 24th.".into(),
            audio_path: None,
            summary_preset: "meeting".into(),
            folder_id: Some("f1".into()),
            client_id: None,
            language: "en".into(),
            summary_provider: String::new(),
            expected_speakers: None,
            detected_language: None,
            created_at: 0,
            updated_at: 0,
            owner: String::new(),
            workspace_id: String::new(),
            private: false,
            deleted_at: None,
        }
    }

    fn facts() -> Facts {
        Facts {
            folder: Some("Product".into()),
            client: None,
            recording: RecordingFacts {
                started_label: Some("12 Oct 2026, 09:00".into()),
                duration_ms: Some(80 * 60_000),
                take_count: 4,
                in_progress: false,
            },
        }
    }

    #[test]
    fn the_document_carries_the_details_then_every_section() {
        let doc = build(&note(), &facts());
        assert_eq!(
            doc,
            "- **Recorded:** 12 Oct 2026, 09:00 · 1h 20m · 4 takes\n\
             - **Folder:** Product\n\
             - **Speakers:** You, Alex\n\
             \n\
             ## Summary\n\
             \n\
             - Launch confirmed for the 24th.\n\
             \n\
             ## Transcript\n\
             \n\
             You: Are we ready?\n\
             \n\
             Alex: Yes, the 24th.\n\
             \n\
             ## Notes\n\
             \n\
             Confirm the launch date."
        );
    }

    #[test]
    fn a_transcript_turn_never_shares_a_paragraph() {
        // CommonMark folds single newlines; each turn gets its own paragraph
        // so Anytype can't run the conversation together on screen.
        let doc = build(&note(), &facts());
        assert!(doc.contains("You: Are we ready?\n\nAlex: Yes, the 24th."));
    }

    #[test]
    fn facts_without_values_are_absent_and_never_null() {
        let facts = Facts {
            folder: None,
            client: Some(" ".into()),
            recording: RecordingFacts::default(),
        };
        let doc = build(&note(), &facts);
        assert!(!doc.contains("**Recorded:**"));
        assert!(!doc.contains("**Folder:**"));
        assert!(!doc.contains("**Client:**"));
        // The transcript still names its speakers — that fact survives.
        assert!(doc.starts_with("- **Speakers:** You, Alex\n\n## Summary"));
    }

    #[test]
    fn empty_sections_read_as_empty_rather_than_vanishing() {
        let mut note = note();
        note.summary = "  ".into();
        let doc = build(&note, &facts());
        assert!(doc.contains("## Summary\n\n_(empty)_"));
        assert!(doc.contains("## Transcript\n\nYou: Are we ready?"));
    }

    #[test]
    fn a_recording_in_progress_says_so_without_a_duration() {
        let facts = Facts {
            folder: None,
            client: None,
            recording: RecordingFacts {
                started_label: Some("12 Oct 2026, 09:00".into()),
                duration_ms: None,
                take_count: 0,
                in_progress: true,
            },
        };
        let doc = build(&note(), &facts);
        assert!(doc.contains("- **Recorded:** 12 Oct 2026, 09:00 · recording in progress"));
    }

    #[test]
    fn a_multi_line_folder_name_cannot_break_the_list() {
        let facts = Facts {
            folder: Some("Line one\nLine two".into()),
            client: None,
            recording: RecordingFacts::default(),
        };
        let doc = build(&note(), &facts);
        assert!(doc.contains("- **Folder:** Line one Line two\n"));
    }

    #[test]
    fn nothing_means_nothing_to_sync() {
        let mut note = note();
        note.title = String::new();
        note.body = String::new();
        note.transcript = String::new();
        note.summary = String::new();
        assert!(is_empty(&note));
        note.body = "<p>x</p>".into();
        assert!(!is_empty(&note));
    }

    #[test]
    fn the_hash_notices_every_input_and_is_stable() {
        let base = hash("space", "page", "Title", "body");
        assert_eq!(base, hash("space", "page", "Title", "body"));
        assert_ne!(base, hash("other", "page", "Title", "body"));
        assert_ne!(base, hash("space", "note", "Title", "body"));
        assert_ne!(base, hash("space", "page", "Title 2", "body"));
        assert_ne!(base, hash("space", "page", "Title", "body "));
    }
}
