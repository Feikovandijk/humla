import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ipc, type Note } from "../lib/ipc";
import { sendNoteToWebhook } from "../lib/webhook";
import { Modal } from "./settings/components/Modal";
import { Btn } from "./settings/components/Btn";

type Destination = { host: string } | { error: string };

// Send to webhook (#213): the confirmation before a note leaves the Mac.
//
// It names the receiver, because "the configured webhook" doesn't answer
// "where is this going?" for someone who set it up months ago. It shows the
// host only, never the path or query, which often carry a token. It asks every
// time: what goes out is a whole meeting, other people's words included, and a
// misclick costs nothing to cancel but can't be unsent.
export function SendToWebhookModal({
  note,
  open,
  onClose,
  flushEdits,
}: {
  note: Note;
  open: boolean;
  onClose: () => void;
  /** Saves the note's queued edits; the backend sends what the database holds. */
  flushEdits: () => Promise<void>;
}) {
  const navigate = useNavigate();
  const [destination, setDestination] = useState<Destination | null>(null);

  // Asked on every open, so a change made in Settings since the last send is
  // the one named here.
  useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setDestination(null);
    ipc.webhookDestination().then(
      (host) => !cancelled && setDestination({ host }),
      (e) => !cancelled && setDestination({ error: String(e) }),
    );
    return () => {
      cancelled = true;
    };
  }, [open]);

  const title = note.title.trim() || "Untitled";

  return (
    <Modal open={open} onClose={onClose} title="Send to webhook" size="sm">
      <div className="flex flex-col gap-3">
        {destination && "error" in destination ? (
          <p role="alert" className="text-sm text-[var(--color-danger)]">
            {destination.error}
          </p>
        ) : (
          <p className="text-sm">
            Send “{title}” to{" "}
            <span className="font-medium">{destination ? destination.host : "…"}</span>?
          </p>
        )}
        <p className="text-xs text-[var(--color-text-muted)]">
          It receives the notes, summary and transcript, with who spoke. Audio is never sent.
        </p>
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
                void sendNoteToWebhook(note.id, destination.host, flushEdits);
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
