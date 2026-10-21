import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ipc, type Note } from "../lib/ipc";
import { syncNoteToAnytype } from "../lib/anytype";
import { Modal } from "./settings/components/Modal";
import { Btn } from "./settings/components/Btn";

type Destination = { space: string } | { error: string };

// Sync to Anytype (ADR 0007): the confirmation before a note is pushed the
// FIRST time. It names the space, because "the configured space" doesn't
// answer "where is this going?" for someone who set it up months ago, and it
// says the one rule that can surprise: edits made in Anytype are replaced the
// next time this note syncs — Humla never reads anything back. Once the user
// ticks "Don't ask again", updates go straight through.
export function SyncToAnytypeModal({
  note,
  open,
  onClose,
  flushEdits,
}: {
  note: Note;
  open: boolean;
  onClose: () => void;
  /** Saves the note's queued edits; the backend pushes what the database holds. */
  flushEdits: () => Promise<void>;
}) {
  const navigate = useNavigate();
  const [destination, setDestination] = useState<Destination | null>(null);
  const [dontAsk, setDontAsk] = useState(false);

  // Asked on every open, so a change made in Settings since the last sync is
  // the one named here.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setDestination(null);
    setDontAsk(false);
    ipc.anytypeConfigGet().then(
      (cfg) => {
        if (cancelled) return;
        setDestination(
          cfg.spaceName.trim() !== ""
            ? { space: cfg.spaceName }
            : { error: "Choose an Anytype space in Settings → General → Integrations first." },
        );
      },
      (e) => !cancelled && setDestination({ error: String(e) }),
    );
    return () => {
      cancelled = true;
    };
  }, [open]);

  const title = note.title.trim() || "Untitled";

  return (
    <Modal open={open} onClose={onClose} title="Sync to Anytype" size="sm">
      <div className="flex flex-col gap-3">
        {destination && "error" in destination ? (
          <p role="alert" className="text-sm text-[var(--color-danger)]">
            {destination.error}
          </p>
        ) : (
          <p className="text-sm">
            Send “{title}” to the{" "}
            <span className="font-medium">{destination ? destination.space : "…"}</span> space
            in Anytype?
          </p>
        )}
        <p className="text-xs text-[var(--color-text-muted)]">
          It gets the notes, summary and transcript, with who spoke, as one object. Audio is
          never sent. Anything you edit in Anytype is replaced the next time you sync this
          note.
        </p>
        <label className="flex items-center gap-2 text-xs text-[var(--color-text-muted)]">
          <input
            type="checkbox"
            checked={dontAsk}
            onChange={(e) => setDontAsk(e.target.checked)}
          />
          Don’t ask again for this note
        </label>
        <div className="flex justify-end gap-2 pt-2">
          <Btn onClick={onClose}>Cancel</Btn>
          {destination && "error" in destination ? (
            <Btn
              onClick={() => {
                onClose();
                navigate("/settings?tab=general");
              }}
            >
              Open Settings
            </Btn>
          ) : (
            <Btn
              disabled={!destination}
              onClick={() => {
                if (!destination) return;
                onClose();
                void syncNoteToAnytype(note.id, dontAsk, flushEdits);
              }}
            >
              Send
            </Btn>
          )}
        </div>
      </div>
    </Modal>
  );
}
