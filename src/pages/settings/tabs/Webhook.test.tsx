import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockTauri } from "../../../test/tauri";
import { IntegrationsSection } from "./Integrations";
import { DEFAULTS, type EditableKey } from "../types";

// The webhook rows (#213) in Settings → General → Integrations.

function settings(webhook: boolean) {
  return { ...DEFAULTS, webhook_enabled: String(webhook) } as Record<EditableKey, string>;
}

function renderRows(webhook: boolean, handlers: Parameters<typeof mockTauri>[0] = {}) {
  const update = vi.fn();
  mockTauri({ mcp_server_path: () => "/Applications/Humla.app/Contents/MacOS/humla-mcp", ...handlers });
  render(<IntegrationsSection s={settings(webhook)} update={update} />);
  return { update };
}

const SAVED = { url: "https://hooks.example.com/humla", headers: ["Authorization"] };

describe("the webhook switch", () => {
  it("is off out of the box, and reads nothing from the Keychain until it is on", async () => {
    const getEndpoint = vi.fn(() => SAVED);
    renderRows(false, { webhook_get_endpoint: getEndpoint });
    expect(screen.getByRole("switch", { name: "Enable sending to a webhook" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(screen.queryByRole("textbox", { name: "Webhook URL" })).toBeNull();
    // Give an eager load every chance to happen before asserting it didn't.
    await new Promise((r) => setTimeout(r, 20));
    expect(getEndpoint).not.toHaveBeenCalled();
  });

  it("persists the choice at once, as the string the backend reads", async () => {
    const { update } = renderRows(false);
    await userEvent.click(screen.getByRole("switch", { name: "Enable sending to a webhook" }));
    expect(update).toHaveBeenCalledWith("webhook_enabled", "true");
  });
});

describe("the webhook endpoint", () => {
  it("shows the saved URL and header names, and never a header's value", async () => {
    renderRows(true, { webhook_get_endpoint: () => SAVED });
    expect(await screen.findByRole("textbox", { name: "Webhook URL" })).toHaveValue(SAVED.url);
    expect(screen.getByRole("textbox", { name: "Header 1 name" })).toHaveValue("Authorization");
    const value = screen.getByLabelText("Header 1 value");
    expect(value).toHaveValue("");
    expect(value).toHaveAttribute("type", "password");
    expect(value).toHaveAttribute("placeholder", "Saved");
  });

  it("keeps an untouched saved value, sends typed ones, and drops blank rows", async () => {
    const setEndpoint = vi.fn();
    renderRows(true, { webhook_get_endpoint: () => SAVED, webhook_set_endpoint: setEndpoint });
    const url = await screen.findByRole("textbox", { name: "Webhook URL" });
    fireEvent.change(url, { target: { value: " https://hooks.example.com/humla/v2 " } });
    await userEvent.click(screen.getByRole("button", { name: "Add header" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Header 2 name" }), {
      target: { value: "X-Team" },
    });
    fireEvent.change(screen.getByLabelText("Header 2 value"), { target: { value: "design" } });
    await userEvent.click(screen.getByRole("button", { name: "Add header" }));

    await userEvent.click(screen.getByRole("button", { name: "Save webhook" }));
    expect(setEndpoint).toHaveBeenCalledWith({
      endpoint: {
        url: "https://hooks.example.com/humla/v2",
        headers: [
          { name: "Authorization", value: null },
          { name: "X-Team", value: "design" },
        ],
      },
    });
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    // What was just typed is a saved value now, and is not shown again.
    expect(screen.getByLabelText("Header 2 value")).toHaveValue("");
    expect(screen.queryByLabelText("Header 3 value")).toBeNull();
  });

  it("removes a header", async () => {
    const setEndpoint = vi.fn();
    renderRows(true, { webhook_get_endpoint: () => SAVED, webhook_set_endpoint: setEndpoint });
    await userEvent.click(await screen.findByRole("button", { name: "Remove header 1" }));
    expect(screen.queryByRole("textbox", { name: "Header 1 name" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Save webhook" }));
    expect(setEndpoint).toHaveBeenCalledWith({ endpoint: { url: SAVED.url, headers: [] } });
  });

  it("shows the backend's refusal and doesn't claim it saved", async () => {
    renderRows(true, {
      webhook_set_endpoint: () => {
        throw "Enter a valid webhook URL.";
      },
    });
    await userEvent.click(await screen.findByRole("button", { name: "Save webhook" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Enter a valid webhook URL.");
    expect(screen.queryByText("Saved")).toBeNull();
  });

  it("still offers a fresh save when the saved endpoint can't be read", async () => {
    const setEndpoint = vi.fn();
    renderRows(true, {
      webhook_get_endpoint: () => {
        throw "The saved webhook couldn't be read. Save it again in Settings → General → Integrations.";
      },
      webhook_set_endpoint: setEndpoint,
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("couldn't be read");
    fireEvent.change(screen.getByRole("textbox", { name: "Webhook URL" }), {
      target: { value: "https://hooks.example.com/new" },
    });
    expect(screen.queryByRole("alert")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Save webhook" }));
    await waitFor(() =>
      expect(setEndpoint).toHaveBeenCalledWith({
        endpoint: { url: "https://hooks.example.com/new", headers: [] },
      }),
    );
  });
});
