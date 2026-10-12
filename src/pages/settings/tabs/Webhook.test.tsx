import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockTauri } from "../../../test/tauri";
import { WebhookSection } from "./Webhook";
import { WebhookButton } from "../../../components/WebhookButton";
import { broadcastSettingChange } from "../../../lib/settingsBus";

describe("webhook settings and delivery feedback", () => {
  it("only reveals the note button after settings are successfully saved, and hides it on disable", async () => {
    const setConfig = vi.fn();
    mockTauri({ webhook_set_config: setConfig });
    render(<><WebhookSection /><WebhookButton send={async () => 204} /></>);
    const user = userEvent.setup();
    expect(screen.queryByRole("button", { name: "Send to webhook" })).toBeNull();
    await user.click(await screen.findByRole("switch", { name: "Enable webhook" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Webhook URL" }), { target: { value: "https://example.com/hook" } });
    await user.click(screen.getByRole("button", { name: "Add header" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Header 1 name" }), { target: { value: "Authorization" } });
    fireEvent.change(screen.getByLabelText("Header 1 value"), { target: { value: "Bearer test-token" } });
    expect(screen.getByLabelText("Header 1 value")).toHaveAttribute("type", "password");
    expect(screen.queryByRole("button", { name: "Send to webhook" })).toBeNull();
    await user.click(screen.getByRole("button", { name: "Save webhook" }));
    expect(setConfig).toHaveBeenCalledWith({ config: {
      enabled: true, url: "https://example.com/hook", headers: [{ name: "Authorization", value: "Bearer test-token" }],
    } });
    expect(await screen.findByRole("button", { name: "Send to webhook" })).toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Enable webhook" }));
    await user.click(screen.getByRole("button", { name: "Save webhook" }));
    await waitFor(() => expect(screen.queryByRole("button", { name: "Send to webhook" })).toBeNull());
  });

  it("keeps failed settings unsaved and supports removing a header", async () => {
    mockTauri({ webhook_set_config: () => { throw "Enter a valid webhook URL."; } });
    render(<><WebhookSection /><WebhookButton send={async () => 204} /></>);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("switch", { name: "Enable webhook" }));
    await user.click(screen.getByRole("button", { name: "Add header" }));
    await user.click(screen.getByRole("button", { name: "Remove header 1" }));
    expect(screen.queryByLabelText("Header 1 name")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Save webhook" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Enter a valid webhook URL.");
    expect(screen.queryByText("Webhook settings saved")).toBeNull();
    expect(screen.queryByRole("button", { name: "Send to webhook" })).toBeNull();
  });

  it("only sends after confirmation, prevents duplicate clicks, and reports success or failure", async () => {
    mockTauri();
    let finish!: (value: number) => void;
    const send = vi.fn().mockImplementationOnce(() => new Promise<number>((resolve) => { finish = resolve; }))
      .mockRejectedValueOnce("Webhook returned HTTP 401. Check authentication headers.");
    render(<WebhookButton send={send} />);
    await act(async () => {});
    act(() => broadcastSettingChange("webhook_enabled", "true"));
    const button = await screen.findByRole("button", { name: "Send to webhook" });
    const user = userEvent.setup();
    await user.click(button);
    expect(screen.getByRole("dialog", { name: "Send to webhook?" })).toHaveTextContent("Do you want to send this note and transcript to the configured webhook?");
    expect(send).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "No" })).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "No" }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(button).toHaveFocus();
    await user.click(button);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(send).not.toHaveBeenCalled();
    fireEvent.click(button);
    const yes = screen.getByRole("button", { name: "Yes" });
    fireEvent.click(yes);
    fireEvent.click(yes);
    fireEvent.click(button);
    expect(send).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: "Sending to webhook…" })).toBeDisabled();
    await act(async () => finish(204));
    expect(await screen.findByRole("status")).toHaveTextContent("Sent to webhook (HTTP 204)");
    fireEvent.click(screen.getByRole("button", { name: "Send to webhook" }));
    expect(send).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Yes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("HTTP 401");
    expect(screen.getByRole("button", { name: "Send to webhook" })).toBeEnabled();
  });
});
