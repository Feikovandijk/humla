// A batch import runs one file at a time through the single capture slot.

import { beforeEach, describe, expect, it, vi } from "vitest";

const { importAudio } = vi.hoisted(() => ({ importAudio: vi.fn() }));
vi.mock("./ipc", async (importOriginal) => {
  const orig = await importOriginal<typeof import("./ipc")>();
  return { ...orig, ipc: { ...orig.ipc, importAudio } };
});

import { importFiles, useImportQueue } from "./importQueue";
import { useRecordingStore } from "./store";
import type { Note } from "./ipc";

const item = (path: string) => ({ path, language: "en", expectedSpeakers: null });
const setPhase = (phase: "idle" | "importing") =>
  useRecordingStore.setState({ status: { noteId: phase === "idle" ? null : "n", phase } as never });
const flush = () => new Promise((r) => setTimeout(r, 0));

describe("importFiles", () => {
  beforeEach(() => {
    importAudio.mockReset();
    importAudio.mockImplementation(async (path: string) => ({ id: path }) as unknown as Note);
    useImportQueue.setState({ pending: [] });
    setPhase("idle");
  });

  it("starts the first file and queues the rest", async () => {
    const note = await importFiles([item("a"), item("b"), item("c")]);
    expect(note?.id).toBe("a");
    expect(importAudio).toHaveBeenCalledTimes(1);
    expect(useImportQueue.getState().pending.map((i) => i.path)).toEqual(["b", "c"]);
  });

  it("queues everything while the slot is busy", async () => {
    setPhase("importing");
    expect(await importFiles([item("a")])).toBeNull();
    expect(importAudio).not.toHaveBeenCalled();
    expect(useImportQueue.getState().pending).toHaveLength(1);
  });

  it("starts the next file when the slot goes idle", async () => {
    await importFiles([item("a"), item("b")]);
    setPhase("importing");
    setPhase("idle");
    await flush();
    expect(importAudio).toHaveBeenLastCalledWith("b", "en", null);
    expect(useImportQueue.getState().pending).toEqual([]);
  });

  it("requeues a file the backend refused for a busy slot", async () => {
    useImportQueue.setState({ pending: [item("b")] });
    importAudio.mockRejectedValueOnce(new Error("already recording or importing"));
    setPhase("importing");
    setPhase("idle");
    await flush();
    expect(useImportQueue.getState().pending.map((i) => i.path)).toEqual(["b"]);
  });

  it("skips a failing file and moves on", async () => {
    useImportQueue.setState({ pending: [item("bad"), item("good")] });
    importAudio.mockRejectedValueOnce(new Error("File not found: bad"));
    setPhase("importing");
    setPhase("idle");
    await flush();
    await flush();
    expect(importAudio).toHaveBeenLastCalledWith("good", "en", null);
  });
});
