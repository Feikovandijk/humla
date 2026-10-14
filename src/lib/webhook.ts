import { create } from "zustand";
import { ipc } from "./ipc";
import { useRecordingStore } from "./store";

// Send to webhook (#213): the one send path, from a note's ⋯ menu.

/** Notes with a send in flight. Module state rather than the toolbar's own: a
 *  send outlives the view that started it, and a note left mid-send and opened
 *  again must still read as sending, or a second press posts it twice. */
export const useWebhookSends = create<{ sending: Record<string, true> }>(() => ({
  sending: {},
}));

function setSending(noteId: string, on: boolean) {
  useWebhookSends.setState(({ sending }) => {
    const next = { ...sending };
    if (on) next[noteId] = true;
    else delete next[noteId];
    return { sending: next };
  });
}

/** Send one note and report the outcome as a toast.
 *
 *  The backend sends the note as the database holds it, so `flushEdits` saves
 *  the view's queued edits first. If that save fails, nothing is sent: the
 *  receiver would get an older copy than the one on screen, and say nothing
 *  about it. */
export async function sendNoteToWebhook(
  noteId: string,
  destination: string,
  flushEdits: () => Promise<void>,
): Promise<void> {
  if (useWebhookSends.getState().sending[noteId]) return;
  setSending(noteId, true);
  const { pushError, pushFlash } = useRecordingStore.getState();
  try {
    await flushEdits().catch((e) => {
      throw new Error(`Your latest edits couldn’t be saved, so the note wasn’t sent. ${String(e)}`);
    });
    await ipc.webhookSendNote(noteId);
    pushFlash(`Sent to ${destination}`);
  } catch (e) {
    // The backend rejects with a plain string; only the flush above is an Error.
    pushError({ noteId, message: e instanceof Error ? e.message : String(e), kind: "webhook" });
  } finally {
    setSending(noteId, false);
  }
}
