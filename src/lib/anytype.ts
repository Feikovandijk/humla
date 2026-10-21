import { useEffect } from "react";
import { create } from "zustand";
import {
  ipc,
  onAnytypeBulk,
  onAnytypeSync,
  type AnytypeBulkEvent,
  type AnytypeStatus,
  type AnytypeSyncEvent,
} from "./ipc";
import { useRecordingStore } from "./store";

// Anytype sync (ADR 0007): the one push path, from a note's ⋯ menu or the
// bulk button in Settings.
//
// Per-note status lives here rather than in the toolbar: a push outlives the
// view that started it, and a note left mid-push and opened again must still
// read as syncing — the backend also refuses a second push for a queued note.
// Statuses arrive from `anytype_sync` events and are fetched once per note on
// first need (`useAnytypeStatus`), so the menu can decide between the
// confirmation modal and a direct update without a round-trip every render.

export const useAnytypeSync = create<{
  statuses: Record<string, AnytypeStatus>;
  /** A bulk run in flight: `{ done, total }`, or null when none is running. */
  bulk: { done: number; total: number } | null;
}>(() => ({ statuses: {}, bulk: null }));

let wired = false;

/** Install the event listeners exactly once per session. Idempotent, so
 *  every consumer (note toolbar, Settings) can call it. */
function wireEvents() {
  if (wired) return;
  wired = true;
  void onAnytypeSync((e: AnytypeSyncEvent) => {
    useAnytypeSync.setState((s) => {
      const prev = s.statuses[e.noteId];
      const done = e.state === "synced" || e.state === "unchanged";
      const next: AnytypeStatus = {
        linked: prev?.linked ?? done,
        confirmed: prev?.confirmed ?? done,
        stale: e.state === "failed" ? (prev?.stale ?? true) : false,
        syncing: e.state === "syncing",
        syncedAt: done ? Date.now() : (prev?.syncedAt ?? null),
        lastError: e.state === "failed" ? (e.message ?? "Sync failed") : null,
      };
      return { statuses: { ...s.statuses, [e.noteId]: next } };
    });
  });
  void onAnytypeBulk((e: AnytypeBulkEvent) => {
    if (e.finished || e.cancelled) {
      useAnytypeSync.setState({ bulk: null });
      if (e.finished && !e.cancelled && e.total > 0) {
        useRecordingStore.getState().pushFlash(`Synced ${e.total} notes to Anytype`);
      }
    } else {
      useAnytypeSync.setState({ bulk: { done: e.done, total: e.total } });
    }
  });
}

/** Wire the sync events; call from any component that reads this store. */
export function useAnytypeEvents() {
  useEffect(() => {
    wireEvents();
  }, []);
}

/** One note's status, fetched once and then kept current by events. Null
 *  until the first read resolves — callers shouldn't promise anything on the
 *  strength of a value they don't have yet. */
export function useAnytypeStatus(noteId: string | null): AnytypeStatus | null {
  useAnytypeEvents();
  const status = useAnytypeSync((s) => (noteId ? (s.statuses[noteId] ?? null) : null));
  useEffect(() => {
    if (!noteId || useAnytypeSync.getState().statuses[noteId]) return;
    ipc
      .anytypeStatus(noteId)
      .then((fetched) => {
        useAnytypeSync.setState((s) => ({ statuses: { ...s.statuses, [noteId]: fetched } }));
      })
      .catch(() => {
        /* the menu item simply offers the plain label */
      });
  }, [noteId]);
  return status;
}

/** The bulk run in flight, if any — including one started before this view
 *  mounted (the Settings dialog closing doesn't stop the queue). */
export function useAnytypeBulk(): { done: number; total: number } | null {
  useAnytypeEvents();
  const bulk = useAnytypeSync((s) => s.bulk);
  useEffect(() => {
    ipc
      .anytypeSyncState()
      .then((p) => {
        if (p && !p.cancelled) {
          useAnytypeSync.setState({ bulk: { done: p.done, total: p.total } });
        }
      })
      .catch(() => {});
  }, []);
  return bulk;
}

/** Push one note and report the outcome as a toast.
 *
 *  The backend pushes the note as the database holds it, so `flushEdits` saves
 *  the view's queued edits first. If that save fails, nothing is pushed: the
 *  Anytype copy would get an older note than the one on screen, and say
 *  nothing about it. */
export async function syncNoteToAnytype(
  noteId: string,
  confirmed: boolean | null,
  flushEdits: () => Promise<void>,
): Promise<void> {
  const { pushError, pushFlash } = useRecordingStore.getState();
  try {
    await flushEdits().catch((e) => {
      throw new Error(`Your latest edits couldn’t be saved, so the note wasn’t synced. ${String(e)}`);
    });
    const outcome = await ipc.anytypeSyncNote(noteId, confirmed);
    pushFlash(
      outcome.kind === "synced"
        ? outcome.recreated
          ? "Re-created in Anytype — the old copy was gone"
          : "Synced to Anytype"
        : "Already up to date in Anytype",
    );
  } catch (e) {
    // The backend rejects with a plain string; only the flush above is an Error.
    pushError({ noteId, message: e instanceof Error ? e.message : String(e), kind: "anytype" });
  }
}
