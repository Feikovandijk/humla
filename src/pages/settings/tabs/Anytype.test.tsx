import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockTauri } from "../../../test/tauri";
import { IntegrationsSection } from "./Integrations";
import { DEFAULTS, type EditableKey } from "../types";

// The Anytype rows (ADR 0007) in Settings → General → Integrations.

function settings(anytype: boolean) {
  return { ...DEFAULTS, anytype_enabled: String(anytype) } as Record<EditableKey, string>;
}

const CONFIG = {
  enabled: true,
  hasKey: true,
  spaceId: "space-1",
  spaceName: "Product",
  typeKey: "page",
  baseUrl: "",
};

const SPACES = [
  { id: "space-1", name: "Product" },
  { id: "space-2", name: "Personal" },
];

function renderRows(anytype: boolean, handlers: Parameters<typeof mockTauri>[0] = {}) {
  const update = vi.fn();
  mockTauri({
    mcp_server_path: () => "/Applications/Humla.app/Contents/MacOS/humla-mcp",
    anytype_config_get: () => CONFIG,
    anytype_list_types: () => [
      { key: "page", name: "Page" },
      { key: "note", name: "Note" },
    ],
    ...handlers,
  });
  render(<IntegrationsSection s={settings(anytype)} update={update} />);
  return { update };
}

describe("the Anytype switch", () => {
  it("is off out of the box, and reads no configuration until it is on", async () => {
    const getConfig = vi.fn(() => CONFIG);
    renderRows(false, { anytype_config_get: getConfig });
    expect(screen.getByRole("switch", { name: "Enable Anytype sync" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.queryByLabelText("Anytype API key")).toBeNull();
    // Give an eager load every chance to happen before asserting it didn't.
    await new Promise((r) => setTimeout(r, 20));
    expect(getConfig).not.toHaveBeenCalled();
  });

  it("persists the choice at once, as the string the backend reads", async () => {
    const { update } = renderRows(false);
    await userEvent.click(screen.getByRole("switch", { name: "Enable Anytype sync" }));
    expect(update).toHaveBeenCalledWith("anytype_enabled", "true");
  });
});

describe("the Anytype configuration", () => {
  it("shows the saved space and a write-only key field", async () => {
    renderRows(true);
    const key = await screen.findByLabelText("Anytype API key");
    expect(key).toHaveValue("");
    expect(key).toHaveAttribute("type", "password");
    expect(key).toHaveAttribute("placeholder", "Saved");
    expect(screen.getByText("Synced to “Product”.")).toBeInTheDocument();
    // The saved type is offered even before the type list loads.
    expect(screen.getByRole("combobox", { name: "Anytype object type" })).toHaveTextContent("Page");
  });

  it("tests the connection with the typed key and offers the spaces it finds", async () => {
    const check = vi.fn(() => SPACES);
    renderRows(true, { anytype_check: check });
    fireEvent.change(await screen.findByLabelText("Anytype API key"), {
      target: { value: "any-key" },
    });
    await userEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(check).toHaveBeenCalledWith({ apiKey: "any-key", baseUrl: null }));

    await userEvent.click(screen.getByRole("combobox", { name: "Anytype space" }));
    const listbox = await screen.findByRole("listbox");
    await userEvent.click(within(listbox).getByRole("option", { name: "Personal" }));
    expect(screen.getByRole("combobox", { name: "Anytype space" })).toHaveTextContent("Personal");
  });

  it("saves the typed key once and never shows it again", async () => {
    const setConfig = vi.fn();
    renderRows(true, { anytype_config_set: setConfig });
    fireEvent.change(await screen.findByLabelText("Anytype API key"), {
      target: { value: "any-key" },
    });
    await userEvent.click(screen.getByRole("button", { name: "Save Anytype settings" }));
    await waitFor(() =>
      expect(setConfig).toHaveBeenCalledWith({
        input: {
          apiKey: "any-key",
          spaceId: "space-1",
          spaceName: "Product",
          typeKey: "page",
          baseUrl: "",
        },
      }),
    );
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    // What was just typed is a saved value now, and is not shown again.
    expect(screen.getByLabelText("Anytype API key")).toHaveValue("");
  });

  it("keeps the stored key when the field is left empty", async () => {
    const setConfig = vi.fn();
    renderRows(true, { anytype_config_set: setConfig });
    await userEvent.click(await screen.findByRole("button", { name: "Save Anytype settings" }));
    await waitFor(() => expect(setConfig).toHaveBeenCalledWith({
      input: { apiKey: null, spaceId: "space-1", spaceName: "Product", typeKey: "page", baseUrl: "" },
    }));
  });

  it("shows the backend's refusal and doesn't claim it saved", async () => {
    renderRows(true, {
      anytype_config_set: () => {
        throw "The Anytype API address must be 127.0.0.1, localhost or ::1.";
      },
    });
    await userEvent.click(await screen.findByRole("button", { name: "Save Anytype settings" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("must be 127.0.0.1");
    expect(screen.queryByText("Saved")).toBeNull();
  });

  it("warns that a space change leaves the old objects where they are", async () => {
    renderRows(true, { anytype_check: () => SPACES });
    await userEvent.click(await screen.findByRole("button", { name: "Test connection" }));
    await userEvent.click(screen.getByRole("combobox", { name: "Anytype space" }));
    const listbox = await screen.findByRole("listbox");
    await userEvent.click(within(listbox).getByRole("option", { name: "Personal" }));
    await userEvent.click(screen.getByRole("button", { name: "Save Anytype settings" }));
    // The notice names the space the objects STAY in, not the new one.
    expect(
      await screen.findByText(/Notes already synced stay in “Product”/),
    ).toBeInTheDocument();
  });

  it("sends every note on request", async () => {
    const syncAll = vi.fn(() => 12);
    renderRows(true, { anytype_sync_all: syncAll });
    await userEvent.click(await screen.findByRole("button", { name: "Send all notes to Anytype" }));
    await waitFor(() => expect(syncAll).toHaveBeenCalled());
  });
});
