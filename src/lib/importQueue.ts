import { create } from "zustand";
import { ipc, type Note } from "./ipc";
import { useNotesStore, useRecordingStore } from "./store";

// Imports share the backend's single capture slot with recording, so a batch
// runs one file at a time: the rest wait here and start as the slot goes idle.

export type QueuedImport = {
  path: string;
  language: string;
  expectedSpeakers: number | null;
};

export const useImportQueue = create<{ pending: QueuedImport[] }>(() => ({ pending: [] }));

// The backend's answer when the slot is taken (commands.rs `import_audio`).
const SLOT_BUSY = "already recording or importing";

let starting = false;

function slotBusy(): boolean {
  return starting || useRecordingStore.getState().status.phase !== "idle";
}

function basename(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

async function start(item: QueuedImport): Promise<Note> {
  starting = true;
  try {
    const note = await ipc.importAudio(item.path, item.language, item.expectedSpeakers);
    useNotesStore.getState().upsertLocal(note);
    return note;
  } finally {
    starting = false;
  }
}

/** Starts the next queued file if the slot is free. */
export async function pumpImportQueue(): Promise<void> {
  if (slotBusy()) return;
  const [next, ...rest] = useImportQueue.getState().pending;
  if (!next) return;
  useImportQueue.setState({ pending: rest });
  try {
    await start(next);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (msg.includes(SLOT_BUSY)) {
      // Something else took the slot first; its idle event pumps us again.
      useImportQueue.setState((s) => ({ pending: [next, ...s.pending] }));
      return;
    }
    useRecordingStore.getState().pushFlash(`Couldn't import ${basename(next.path)}: ${msg}`);
    void pumpImportQueue();
  }
}

/**
 * Imports a batch. With the slot free the first file starts now and its note
 * is returned (its error throws, for the dialog to show); everything else is
 * queued. Returns null when the whole batch was queued.
 */
export async function importFiles(items: QueuedImport[]): Promise<Note | null> {
  if (items.length === 0) return null;
  const queue = (more: QueuedImport[]) =>
    useImportQueue.setState((s) => ({ pending: [...s.pending, ...more] }));
  if (slotBusy() || useImportQueue.getState().pending.length > 0) {
    queue(items);
    return null;
  }
  const [first, ...rest] = items;
  try {
    const note = await start(first);
    queue(rest);
    return note;
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    if (!msg.includes(SLOT_BUSY)) throw e;
    queue(items);
    return null;
  }
}

useRecordingStore.subscribe((s, prev) => {
  if (s.status.phase === "idle" && prev.status.phase !== "idle") void pumpImportQueue();
});
