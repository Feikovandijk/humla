import { beforeAll, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import { renderApp } from "../test/app";
import { makeNote } from "../test/fixtures";
import { mockLayoutBox } from "../test/layout";
import { useNotesStore } from "../lib/store";

beforeAll(() => mockLayoutBox());

describe("sending the current note", () => {
  it("sends transcript edits while their autosave is still in flight", async () => {
    const note = makeNote({ id: "n1", title: "Meeting", transcript: "You: Original" });
    const send = vi.fn(() => 204);
    let finishSave!: () => void;
    const save = vi.fn(() => new Promise<void>((resolve) => { finishSave = resolve; }));
    renderApp("/note/n1", {
      notes_list: () => [note], notes_get: () => note, notes_update: save,
      settings_get: (args) => ["onboarding_completed", "webhook_enabled"].includes((args as { key: string }).key) ? "true" : null,
      webhook_send_note: send,
    });
    fireEvent.click(await screen.findByRole("button", { name: /^transcript$/i }));
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Transcript" }), { target: { value: "Alex: Just corrected" } });
    await waitFor(() => expect(save).toHaveBeenCalledWith({ id: "n1", patch: expect.objectContaining({ transcript: "Alex: Just corrected" }) }));
    fireEvent.click(screen.getByRole("button", { name: "Send to webhook" }));
    fireEvent.click(screen.getByRole("button", { name: "Yes" }));
    await waitFor(() => expect(send).toHaveBeenCalledWith({ noteId: "n1", draft: expect.objectContaining({ transcript: "Alex: Just corrected" }) }));
    await act(async () => finishSave());
  });

  it("includes a just-edited title before autosave and the latest rebuilt transcript", async () => {
    const note = makeNote({ id: "n1", title: "Stored title", body: "<p>My notes</p>", transcript: "You: Original", summary: "- Decision" });
    const send = vi.fn(() => 200);
    renderApp("/note/n1", {
      notes_list: () => [note], notes_get: () => note,
      settings_get: (args) => ["onboarding_completed", "webhook_enabled"].includes((args as { key: string }).key) ? "true" : null,
      webhook_send_note: send,
    });
    await screen.findByRole("button", { name: "Send to webhook" });
    await waitFor(() => expect(screen.getByPlaceholderText("New note")).toHaveValue("Stored title"));
    note.transcript = "Alex: Corrected speaker and words";
    act(() => useNotesStore.getState().replaceTranscript("n1", note.transcript));
    fireEvent.change(screen.getByPlaceholderText("New note"), { target: { value: "Just typed" } });
    fireEvent.click(screen.getByRole("button", { name: "Send to webhook" }));
    fireEvent.click(screen.getByRole("button", { name: "Yes" }));
    await waitFor(() => expect(send).toHaveBeenCalledWith({ noteId: "n1", draft: expect.objectContaining({
      title: "Just typed", body: "<p>My notes</p>", transcript: note.transcript, summary: "- Decision",
    }) }));
  });
});
