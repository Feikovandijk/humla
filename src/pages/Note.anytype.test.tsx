import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/app";
import { makeNote } from "../test/fixtures";
import { mockLayoutBox } from "../test/layout";
import { broadcastSettingChange } from "../lib/settingsBus";
import { useRecordingStore } from "../lib/store";
import { useAnytypeSync } from "../lib/anytype";
import type { AnytypeStatus } from "../lib/ipc";
import type { mockTauri } from "../test/tauri";

// Sync to Anytype (ADR 0007) from a note's ⋯ menu: offered only once turned
// on, confirmed against the space before the first push, and pushed from the
// database after the view's queued edits are saved.

beforeAll(() => mockLayoutBox());
afterEach(() => {
  useRecordingStore.setState({ errors: [], flashes: [] });
  useAnytypeSync.setState({ statuses: {}, bulk: null });
});

const UNLINKED: AnytypeStatus = {
  linked: false,
  confirmed: false,
  stale: false,
  syncing: false,
  syncedAt: null,
  lastError: null,
};

const LINKED_CONFIRMED: AnytypeStatus = {
  linked: true,
  confirmed: true,
  stale: false,
  syncing: false,
  syncedAt: 1_700_000_000_000,
  lastError: null,
};

function open(
  handlers: Parameters<typeof mockTauri>[0] = {},
  enabled = true,
  status: AnytypeStatus = UNLINKED,
) {
  const note = makeNote({ id: "n1", title: "Weekly planning" });
  renderApp("/note/n1", {
    notes_list: () => [note],
    notes_get: () => note,
    settings_get: (args) => {
      const key = (args as { key: string }).key;
      if (key === "onboarding_completed") return "true";
      if (key === "anytype_enabled") return String(enabled);
      return null;
    },
    anytype_status: () => status,
    anytype_config_get: () => ({
      enabled: true,
      hasKey: true,
      spaceId: "space-1",
      spaceName: "Product",
      typeKey: "page",
      baseUrl: "",
    }),
    ...handlers,
  });
  return note;
}

const titleBox = () => screen.getByPlaceholderText("New note");

async function openMenu() {
  await userEvent.click(await screen.findByRole("button", { name: /more/i }));
}

async function confirmFirstPush(dontAsk = false) {
  await openMenu();
  await userEvent.click(await screen.findByRole("menuitem", { name: "Sync to Anytype…" }));
  const dialog = await screen.findByRole("dialog", { name: "Sync to Anytype" });
  await waitFor(() =>
    expect(dialog).toHaveTextContent("Send “Weekly planning” to the Product space in Anytype?"),
  );
  if (dontAsk) {
    await userEvent.click(screen.getByRole("checkbox", { name: /don’t ask again/i }));
  }
  await userEvent.click(screen.getByRole("button", { name: "Send" }));
}

describe("⋯ → Sync to Anytype", () => {
  it("is offered only once Anytype sync is turned on", async () => {
    open({}, false);
    await waitFor(() => expect(titleBox()).toHaveValue("Weekly planning"));
    await openMenu();
    expect(await screen.findByRole("menuitem", { name: "Export…" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: /anytype/i })).toBeNull();
    await userEvent.keyboard("{Escape}");

    // Turned on in Settings while the note stays open behind the dialog.
    act(() => broadcastSettingChange("anytype_enabled", "true"));
    await openMenu();
    expect(await screen.findByRole("menuitem", { name: "Sync to Anytype…" })).toBeInTheDocument();
  });

  it("names the space before anything is pushed, and Cancel pushes nothing", async () => {
    const send = vi.fn();
    open({ anytype_sync_note: send });
    await openMenu();
    await userEvent.click(await screen.findByRole("menuitem", { name: "Sync to Anytype…" }));

    const dialog = await screen.findByRole("dialog", { name: "Sync to Anytype" });
    await waitFor(() =>
      expect(dialog).toHaveTextContent("Send “Weekly planning” to the Product space in Anytype?"),
    );
    expect(dialog).toHaveTextContent("Audio is never sent.");
    expect(dialog).toHaveTextContent("replaced the next time you sync this note");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("saves the note's queued edits before the backend reads it to push", async () => {
    // The backend pushes the note as the database holds it, so an edit still
    // in the autosave queue has to land first.
    const calls: string[] = [];
    let saved!: () => void;
    const send = vi.fn(() => {
      calls.push("send");
      return { kind: "synced", recreated: false };
    });
    open({
      notes_update: (args) => {
        const { patch } = args as { patch: Record<string, unknown> };
        if (!("title" in patch)) return undefined;
        calls.push("save");
        return new Promise<void>((resolve) => (saved = resolve));
      },
      anytype_sync_note: send,
    });
    await waitFor(() => expect(titleBox()).toHaveValue("Weekly planning"));
    fireEvent.change(titleBox(), { target: { value: "Weekly planning, final" } });

    await confirmFirstPush();
    await waitFor(() => expect(calls).toEqual(["save"]));
    expect(send).not.toHaveBeenCalled();
    await act(async () => saved());
    await waitFor(() => expect(calls).toEqual(["save", "send"]));
    expect(send).toHaveBeenCalledWith({ noteId: "n1", confirmed: false });
    expect(await screen.findByText("Synced to Anytype")).toBeInTheDocument();
  });

  it("records 'don't ask again' on the push it belongs to", async () => {
    const send = vi.fn(() => ({ kind: "synced", recreated: false }));
    open({ anytype_sync_note: send });
    await confirmFirstPush(true);
    await waitFor(() => expect(send).toHaveBeenCalledWith({ noteId: "n1", confirmed: true }));
  });

  it("a confirmed note updates without asking again", async () => {
    const send = vi.fn(() => ({ kind: "synced", recreated: false }));
    open({ anytype_sync_note: send }, true, LINKED_CONFIRMED);
    await openMenu();
    await userEvent.click(await screen.findByRole("menuitem", { name: "Update in Anytype…" }));
    await waitFor(() => expect(send).toHaveBeenCalledWith({ noteId: "n1", confirmed: false }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(await screen.findByText("Synced to Anytype")).toBeInTheDocument();
  });

  it("says when nothing changed rather than pushing again", async () => {
    open({ anytype_sync_note: () => ({ kind: "unchanged" }) }, true, LINKED_CONFIRMED);
    await openMenu();
    await userEvent.click(await screen.findByRole("menuitem", { name: "Update in Anytype…" }));
    expect(await screen.findByText("Already up to date in Anytype")).toBeInTheDocument();
  });

  it("reports a refused push under its own heading, with the way to Settings", async () => {
    open({
      anytype_sync_note: () => {
        throw "Add your Anytype API key in Settings → General → Integrations first.";
      },
    });
    await confirmFirstPush();
    expect(await screen.findByText("Couldn’t sync to Anytype")).toBeInTheDocument();
    expect(screen.getByText(/API key/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Settings" })).toBeInTheDocument();
  });

  it("shows a push in flight and refuses to start a second one", async () => {
    const send = vi.fn(() => ({ kind: "synced", recreated: false }));
    open({ anytype_sync_note: send }, true, { ...LINKED_CONFIRMED, syncing: true });
    await openMenu();
    const item = await screen.findByRole("menuitem", { name: "Syncing to Anytype…" });
    expect(item).toHaveAttribute("aria-disabled", "true");
    await userEvent.keyboard("{Escape}");
    expect(send).not.toHaveBeenCalled();
  });
});
