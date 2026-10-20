import { useEffect, useMemo, useState } from "react";
import { Row } from "./Section";
import { ipc, type Note } from "../../../lib/ipc";
import { useNotesStore } from "../../../lib/store";
import { s as plural } from "./format";
import {
  notesWithSpeaker,
  renameOutcomeMessage,
  renameSpeakerAcrossNotes,
} from "../../../lib/crossNoteRename";

// The `You` case of the cross-note rename (#116 part 2).
//
// Mic chunks on remote calls are labelled with the literal "You", which reads
// wrong in a transcript anyone else opens and splits you from your own name
// everywhere else. This is the same cross-note rename the chip strip offers, with
// its arguments pre-filled.
//
// **Count-gated, and deliberately not a permanent surface.** It renders only when
// a transcript actually says "You", so it never existed for a new install and
// disappears for good once used — the self-retiring shape #122's rebuild row
// uses. A library-level speaker-management view was rejected: at a few dozen
// notes this is a legacy cleanup, and a management screen would be dead weight
// the day after it was used.
//
// It lives in Transcription rather than Chat because the row is about what
// transcripts *say*, which is where someone whose transcript reads "You:" looks.
//
// An automatic migration was rejected and stays rejected: it would mass-rewrite
// user content in one go, and it's irreversible, where the fallback is the macOS
// name and could stamp `admin` across every transcript. Showing the exact string
// first is the mitigation.
//
// This does NOT remove the compatibility path: `You:` keeps arriving from every
// new remote recording, so `chat/tools.rs` still resolves
// it at query time. The row is about readability and convergence, not retrieval.

const YOU_LABEL = "You";

export function RenameYouRow() {
  const notes = useNotesStore((s) => s.notes);
  const replaceTranscript = useNotesStore((s) => s.replaceTranscript);
  const [name, setName] = useState<string | null>(null);
  const [state, setState] = useState<
    | { kind: "idle" }
    | { kind: "running" }
    | { kind: "done"; message: string; retryable: Note[] }
  >({ kind: "idle" });
  /** How many notes the run in progress (or just finished) covered. */
  const [ranAgainst, setRanAgainst] = useState(0);

  // Counted from transcript text, so the number on the button cannot be a lie —
  // `notes.speakers` is only written by `reindex_note` and can be stale.
  const affected = useMemo(() => notesWithSpeaker(notes, YOU_LABEL), [notes]);

  // `user_display_name` → macOS full name. Resolved in Rust so the chain lives
  // in one place.
  useEffect(() => {
    let live = true;
    ipc
      .speakerDefaultName()
      .then((n) => live && setName(n ?? ""))
      .catch(() => live && setName(""));
    return () => {
      live = false;
    };
  }, []);

  // Nothing to repair → no row at all. Also hidden until the prefill resolves,
  // so the field never flickers from empty to a name.
  //
  // `state.kind === "idle"` is what keeps the confirmation visible: the
  // optimistic store update empties `affected` the moment the sweep starts, so
  // retiring on the count alone would unmount the row mid-run and the user would
  // never learn whether it worked. It retires on the next visit instead.
  if (name === null) return null;
  if (affected.length === 0 && state.kind === "idle") return null;

  const target = name.trim();
  // Frozen once a run starts, for the same reason: the live count is on its way
  // to zero and the button must keep saying what it did.
  const count = state.kind === "idle" ? affected.length : ranAgainst;

  async function run(targets: Note[]) {
    setRanAgainst(targets.length);
    setState({ kind: "running" });
    const outcome = await renameSpeakerAcrossNotes({
      notes: targets,
      oldLabel: YOU_LABEL,
      newLabel: target,
      onRewritten: replaceTranscript,
    });
    setState({
      kind: "done",
      message: renameOutcomeMessage(outcome),
      // The PRE-rewrite notes, kept so a retry has something to rewrite. The
      // store's copies already hold the optimistic rename, so they no longer
      // carry the old label and retrying off them would be a silent no-op.
      retryable: targets.filter((n) => outcome.failed.includes(n.id)),
    });
  }

  return (
    <Row label='Speaker labelled "You"'>
      <p className="text-xs text-[color:var(--color-text-muted)]">
        {count} recording{plural(count)} label{count === 1 ? "s" : ""} your side of the call as
        "You". Renaming brings them together with the rest of your notes, so a search for your
        name finds all of them.
      </p>
      <div className="flex items-center gap-2 mt-2">
        <input
          aria-label="Rename You to"
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="min-w-0 flex-1 text-sm px-2.5 py-1.5 rounded-md border border-[var(--color-line-visible)] bg-[var(--color-surface)] focus:border-[var(--color-text-muted)] transition-colors"
          placeholder="Your name"
        />
        <button
          className="nd-btn shrink-0"
          onClick={() => void run(affected)}
          disabled={!target || state.kind === "running"}
        >
          {/* The exact string it will write, so nothing about this is a surprise. */}
          {state.kind === "running"
            ? "Renaming…"
            : `Rename You → ${target || "…"} in ${count} note${plural(count)}`}
        </button>
      </div>
      {state.kind === "done" && (
        <p
          className={
            state.retryable.length > 0
              ? "text-xs text-[color:var(--color-warning-text)] mt-1"
              : "text-xs text-[color:var(--color-success)] mt-1"
          }
        >
          {state.message}
          {state.retryable.length > 0 && (
            <button className="nd-btn ml-2" onClick={() => void run(state.retryable)}>
              Retry {state.retryable.length}
            </button>
          )}
        </p>
      )}
      <p className="text-xs text-[color:var(--color-text-muted)] mt-1">
        Rewrites each transcript and its playback labels. Summaries are left alone, and every
        note keeps a restorable version from before the rename.
      </p>
    </Row>
  );
}
