import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/app";
import { makeNote } from "../test/fixtures";
import { mockLayoutBox } from "../test/layout";
import { broadcastSettingChange } from "../lib/settingsBus";
import { useRecordingStore } from "../lib/store";
import { useWebhookSends } from "../lib/webhook";
import type { mockTauri } from "../test/tauri";

// Send to webhook (#213) from a note's ⋯ menu: offered only once turned on,
// confirmed against the receiver's host, and sent from the database after the
// view's queued edits are saved.

beforeAll(() => mockLayoutBox());
afterEach(() => {
  useRecordingStore.setState({ errors: [], flashes: [] });
  useWebhookSends.setState({ sending: {} });
});

function open(handlers: Parameters<typeof mockTauri>[0] = {}, enabled = true) {
  const note = makeNote({ id: "n1", title: "Weekly planning" });
  renderApp("/note/n1", {
    notes_list: () => [note],
    notes_get: () => note,
    settings_get: (args) => {
      const key = (args as { key: string }).key;
      if (key === "onboarding_completed") return "true";
      if (key === "webhook_enabled") return String(enabled);
      return null;
    },
    webhook_destination: () => "hooks.example.com",
    ...handlers,
  });
  return note;
}

const titleBox = () => screen.getByPlaceholderText("New note");

async function openMenu() {
  await userEvent.click(await screen.findByRole("button", { name: /more/i }));
}

async function confirmSend() {
  await openMenu();
  await userEvent.click(await screen.findByRole("menuitem", { name: "Send to webhook…" }));
  const dialog = await screen.findByRole("dialog", { name: "Send to webhook" });
  await waitFor(() => expect(dialog).toHaveTextContent("hooks.example.com"));
  await userEvent.click(screen.getByRole("button", { name: "Send" }));
}

describe("⋯ → Send to webhook", () => {
  it("is offered only once the webhook is turned on", async () => {
    open({}, false);
    await waitFor(() => expect(titleBox()).toHaveValue("Weekly planning"));
    await openMenu();
    expect(await screen.findByRole("menuitem", { name: "Export…" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitem", { name: /webhook/i })).toBeNull();
    await userEvent.keyboard("{Escape}");

    // Turned on in Settings while the note stays open behind the dialog.
    act(() => broadcastSettingChange("webhook_enabled", "true"));
    await openMenu();
    expect(await screen.findByRole("menuitem", { name: "Send to webhook…" })).toBeInTheDocument();
  });

  it("names the receiver before anything is sent, and Cancel sends nothing", async () => {
    const send = vi.fn();
    open({ webhook_send_note: send });
    await openMenu();
    await userEvent.click(await screen.findByRole("menuitem", { name: "Send to webhook…" }));

    const dialog = await screen.findByRole("dialog", { name: "Send to webhook" });
    await waitFor(() =>
      expect(dialog).toHaveTextContent("Send “Weekly planning” to hooks.example.com?"),
    );
    expect(dialog).toHaveTextContent("Audio is never sent.");
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(send).not.toHaveBeenCalled();
  });

  it("saves the note's queued edits before the backend reads it to send", async () => {
    // The backend sends the note as the database holds it, so an edit still
    // in the autosave queue has to land first.
    const calls: string[] = [];
    let saved!: () => void;
    const send = vi.fn(() => {
      calls.push("send");
    });
    open({
      notes_update: (args) => {
        const { patch } = args as { patch: Record<string, unknown> };
        if (!("title" in patch)) return undefined;
        calls.push("save");
        return new Promise<void>((resolve) => (saved = resolve));
      },
      webhook_send_note: send,
    });
    await waitFor(() => expect(titleBox()).toHaveValue("Weekly planning"));
    fireEvent.change(titleBox(), { target: { value: "Weekly planning, final" } });

    await confirmSend();
    await waitFor(() => expect(calls).toEqual(["save"]));
    expect(send).not.toHaveBeenCalled();
    await act(async () => saved());
    await waitFor(() => expect(calls).toEqual(["save", "send"]));
    expect(send).toHaveBeenCalledWith({ noteId: "n1" });
    expect(await screen.findByText("Sent to hooks.example.com")).toBeInTheDocument();
  });

  it("can't send the same note twice at once", async () => {
    let finish!: () => void;
    const send = vi.fn(() => new Promise<void>((resolve) => (finish = resolve)));
    open({ webhook_send_note: send });
    await confirmSend();
    await waitFor(() => expect(send).toHaveBeenCalledTimes(1));

    await openMenu();
    const item = await screen.findByRole("menuitem", { name: "Sending to webhook…" });
    expect(item).toHaveAttribute("aria-disabled", "true");
    await userEvent.keyboard("{Escape}");
    await act(async () => finish());
    await openMenu();
    expect(await screen.findByRole("menuitem", { name: "Send to webhook…" })).not.toHaveAttribute(
      "aria-disabled",
    );
    expect(send).toHaveBeenCalledTimes(1);
  });

  it("reports a refused send under its own heading, with the way to Settings", async () => {
    open({
      webhook_send_note: () => {
        throw "The webhook returned HTTP 401. Check its authentication headers in Settings.";
      },
    });
    await confirmSend();
    expect(await screen.findByText("Couldn’t send to webhook")).toBeInTheDocument();
    expect(screen.getByText(/HTTP 401/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Open Settings" })).toBeInTheDocument();
  });

  it("says what is missing when no URL is saved, and offers nothing to send", async () => {
    const send = vi.fn();
    open({
      webhook_destination: () => {
        throw "Add a webhook URL in Settings → General → Integrations first.";
      },
      webhook_send_note: send,
    });
    await openMenu();
    await userEvent.click(await screen.findByRole("menuitem", { name: "Send to webhook…" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Add a webhook URL");
    expect(screen.queryByRole("button", { name: "Send" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Open Settings" }));
    await waitFor(() =>
      expect(screen.getByTestId("location")).toHaveTextContent("/settings?tab=general"),
    );
    expect(send).not.toHaveBeenCalled();
  });
});
