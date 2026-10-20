import { useEffect, useState } from "react";
import { save } from "@tauri-apps/plugin-dialog";
import { ipc, type ExportFormat, type RecordingAudio, type Note } from "../lib/ipc";
import { useRecordingStore } from "../lib/store";
import { Modal } from "./settings/components/Modal";
import { Segmented } from "./settings/components/Segmented";
import { Toggle } from "./settings/components/Toggle";
import { Btn } from "./settings/components/Btn";

// Export a note's summary / transcript / typed notes as ONE combined
// Markdown, plain-text or PDF file (issue #18). The kit modal supplies only the
// chrome; this component owns the body. Confirming opens the native save
// panel (dialog plugin) and hands the chosen path to `export_note`.

// Slugify a note title into a filename stem. Unicode-letter/number aware so
// non-ASCII titles still produce a readable name; empty → "note".
const FORMATS: Record<ExportFormat, { label: string; filter: string; extensions: string[] }> = {
  markdown: { label: "Markdown", filter: "Markdown", extensions: ["md", "markdown"] },
  txt: { label: "Plain text", filter: "Plain text", extensions: ["txt"] },
  pdf: { label: "PDF", filter: "PDF document", extensions: ["pdf"] },
};

function slug(title: string): string {
  const s = title
    .toLowerCase()
    .trim()
    .replace(/[^\p{L}\p{N}]+/gu, "-")
    .replace(/^-+|-+$/g, "");
  return s || "note";
}

// Cheap "does the typed body have any text" check for the checkbox
// enable/disable state. The authoritative HTML→text conversion happens
// backend; this only needs to know whether the section is worth offering.
function hasText(html: string): boolean {
  return html.replace(/<[^>]*>/g, "").replace(/&nbsp;/g, " ").trim() !== "";
}

function Check({
  checked,
  onChange,
  disabled,
  label,
  hint,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label: string;
  hint?: string;
}) {
  return (
    <label
      className={
        "flex items-center gap-2.5 py-1.5 text-sm " +
        (disabled ? "opacity-40 cursor-not-allowed" : "cursor-pointer")
      }
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="h-4 w-4 accent-[var(--color-accent)]"
      />
      <span>{label}</span>
      {hint && <span className="text-[var(--color-text-muted)] text-xs">{hint}</span>}
    </label>
  );
}

export function ExportModal({
  note,
  open,
  onClose,
  flushEdits,
}: {
  note: Note;
  open: boolean;
  onClose: () => void;
  /** Saves the note's queued edits. `export_note` reads the note from the
   *  database, so it waits for this before writing the file. */
  flushEdits: () => Promise<void>;
}) {
  const pushError = useRecordingStore((s) => s.pushError);

  const hasSummary = note.summary.trim() !== "";
  const hasTranscript = note.transcript.trim() !== "";
  const hasNotes = hasText(note.body);

  const [format, setFormat] = useState<ExportFormat>("markdown");
  // Defaults: Summary + Transcript checked (when they have content); Notes off.
  const [includeSummary, setIncludeSummary] = useState(hasSummary);
  const [includeTranscript, setIncludeTranscript] = useState(hasTranscript);
  const [includeNotes, setIncludeNotes] = useState(false);
  const [includeSpeakerLabels, setIncludeSpeakerLabels] = useState(true);
  const [busy, setBusy] = useState(false);

  const [recordings, setRecordings] = useState<RecordingAudio[]>([]);
  const [recordingsError, setRecordingsError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setRecordings([]);
    setRecordingsError(null);
    ipc.noteRecordingAudio(note.id).then((files) => {
      if (!cancelled) setRecordings(files);
    }).catch((e) => {
      if (!cancelled) setRecordingsError(String(e));
    });
    return () => { cancelled = true; };
  }, [note.id, open]);

  async function exportRecording(recording: RecordingAudio) {
    setBusy(true);
    try {
      const path = await save({
        defaultPath: `${slug(note.title)}-recording-${recording.sessionIndex}-${recording.source}.wav`,
        filters: [{ name: "WAV audio", extensions: ["wav"] }],
      });
      if (path) await ipc.exportRecordingAudio(note.id, recording.sessionId, recording.source, path);
    } catch (e) {
      pushError({ noteId: note.id, message: String(e), kind: "export" });
    } finally {
      setBusy(false);
    }
  }

  const nothingSelected =
    !(includeSummary && hasSummary) &&
    !(includeTranscript && hasTranscript) &&
    !(includeNotes && hasNotes);

  async function onExport() {
    setBusy(true);
    try {
      const { filter, extensions } = FORMATS[format];
      const path = await save({
        defaultPath: `${slug(note.title)}.${extensions[0]}`,
        filters: [{ name: filter, extensions }],
      });
      if (!path) return; // user cancelled the panel
      await flushEdits();
      await ipc.exportNote(note.id, {
        path,
        format,
        includeSummary: includeSummary && hasSummary,
        includeTranscript: includeTranscript && hasTranscript,
        includeNotes: includeNotes && hasNotes,
        includeSpeakerLabels,
      });
      onClose();
    } catch (e) {
      pushError({ noteId: note.id, message: String(e), kind: "export" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal open={open} onClose={onClose} title="Export note">
      <div className="flex flex-col gap-5">
        <div className="flex flex-col gap-1">
          <div className="nd-label">Include</div>
          <Check
            checked={includeSummary && hasSummary}
            disabled={!hasSummary}
            onChange={setIncludeSummary}
            label="Summary"
            hint={hasSummary ? undefined : "empty"}
          />
          <Check
            checked={includeTranscript && hasTranscript}
            disabled={!hasTranscript}
            onChange={setIncludeTranscript}
            label="Transcript"
            hint={hasTranscript ? undefined : "empty"}
          />
          <Check
            checked={includeNotes && hasNotes}
            disabled={!hasNotes}
            onChange={setIncludeNotes}
            label="Notes"
            hint={hasNotes ? undefined : "empty"}
          />
        </div>

        <div className="flex items-center justify-between gap-4">
          <span className="text-sm">Speaker labels in transcript</span>
          <Toggle
            checked={includeSpeakerLabels}
            onChange={setIncludeSpeakerLabels}
            disabled={!(includeTranscript && hasTranscript)}
            label="Include speaker labels in transcript"
          />
        </div>

        <div className="flex items-center justify-between gap-4">
          <span className="text-sm">Format</span>
          <Segmented<ExportFormat>
            label="Export format"
            value={format}
            onChange={setFormat}
            options={(Object.keys(FORMATS) as ExportFormat[]).map((value) => ({
              value,
              label: FORMATS[value].label,
            }))}
          />
        </div>

        {(recordings.length > 0 || recordingsError) && (
          <div className="flex flex-col gap-2 pt-4 border-t border-[var(--color-line)]">
            <div className="nd-label">Raw recordings</div>
            {recordingsError && <p role="alert" className="text-sm">Could not load recordings: {recordingsError}</p>}
            {recordings.map((recording) => {
              const source = recording.source === "mic" ? "microphone" : "system";
              return (
                <div key={`${recording.sessionId}-${recording.source}`} className="flex items-center justify-between gap-4">
                  <span className="text-sm">Recording {recording.sessionIndex} · {recording.source === "mic" ? "Microphone" : "System audio"}</span>
                  <Btn disabled={busy} aria-label={`Export recording ${recording.sessionIndex} ${source} WAV`}
                    onClick={() => void exportRecording(recording)}>Export WAV…</Btn>
                </div>
              );
            })}
          </div>
        )}

        <div className="flex justify-end gap-2 pt-1">
          <Btn onClick={onClose}>Cancel</Btn>
          <Btn onClick={onExport} disabled={nothingSelected || busy}>
            {busy ? "Exporting…" : "Export…"}
          </Btn>
        </div>
      </div>
    </Modal>
  );
}
