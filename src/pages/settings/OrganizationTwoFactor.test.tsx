import { describe, it, expect } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../../test/app";
import type { CloudRole, TwoFactorStatus } from "../../lib/cloud";

// An owner or admin can require two-factor authentication of every member,
// once they have it themselves, and sees who has it before they do.

const ON: TwoFactorStatus = { available: true, enabled: true, recovery_codes_left: 10, required_by: [] };
const OFF: TwoFactorStatus = { ...ON, enabled: false, recovery_codes_left: 0 };

const MEMBERS = [
  { id: "u1", email: "m@example.no", name: "Michael", role: "owner" },
  { id: "u2", email: "ola@example.no", name: "Ola", role: "member" },
];

function status(role: CloudRole, two_factor: TwoFactorStatus, require_2fa = false) {
  const ws = { id: "w1", name: "Acme", role, plan_status: "active", require_2fa };
  return {
    configured: true,
    logged_in: true,
    base_url: "https://sync.humla.team",
    user: { id: "u1", email: "m@example.no", name: "Michael", verified: true },
    current_workspace: ws,
    workspaces: [ws],
    billing_enabled: false,
    two_factor,
  };
}

async function openAccount(handlers: Record<string, (args: unknown) => unknown>) {
  renderApp("/settings?tab=account", {
    cloud_workspace_members: () => MEMBERS,
    cloud_two_factor_members: () => ({ u1: true, u2: false }),
    ...handlers,
  });
  return await screen.findByRole("dialog", { name: /settings/i });
}

describe("Organization → require two-factor authentication", () => {
  it("lets an admin who has it require it, after saying who doesn't", async () => {
    let current = status("admin", ON);
    const requests: unknown[] = [];
    const dialog = await openAccount({
      cloud_status: () => current,
      cloud_set_workspace_require_2fa: (args) => {
        requests.push(args);
        current = status("admin", ON, true);
        return null;
      },
    });

    const toggle = await within(dialog).findByRole("switch", {
      name: /require two-factor authentication/i,
    });
    expect(await within(dialog).findByText(/1 member doesn't have it yet/i)).toBeInTheDocument();
    await userEvent.click(toggle);

    expect(requests).toEqual([{ workspaceId: "w1", require: true }]);
    await screen.findByRole("switch", { name: /require two-factor authentication/i, checked: true });
  });

  it("says so when it couldn't check who has it, rather than implying everyone does", async () => {
    const dialog = await openAccount({
      cloud_status: () => status("owner", ON),
      cloud_two_factor_members: () => {
        throw { reason: "", message: "offline" };
      },
    });
    expect(
      await within(dialog).findByText(/couldn't check which members have it/i),
    ).toBeInTheDocument();
  });

  it("waits until you have it yourself", async () => {
    const dialog = await openAccount({ cloud_status: () => status("owner", OFF) });
    const toggle = await within(dialog).findByRole("switch", {
      name: /require two-factor authentication/i,
    });
    expect(toggle).toBeDisabled();
    expect(within(dialog).getByText(/for your own account first/i)).toBeInTheDocument();
  });

  it("marks which members have it", async () => {
    const dialog = await openAccount({ cloud_status: () => status("owner", ON) });
    const heading = await within(dialog).findByRole("heading", { name: /^members/i });
    const roster = heading.closest("section") as HTMLElement;
    const row = (email: string) => within(roster).getByText(email).closest(".rounded-md") as HTMLElement;

    expect(await within(roster).findByText(/no 2fa/i)).toBeInTheDocument();
    expect(within(row("ola@example.no")).getByText(/no 2fa/i)).toBeInTheDocument();
    expect(within(row("m@example.no")).getByText(/^2fa$/i)).toBeInTheDocument();
  });

  it("isn't offered to a member", async () => {
    const dialog = await openAccount({ cloud_status: () => status("member", ON, true) });
    await within(dialog).findByText("Ola");
    expect(within(dialog).queryByRole("switch", { name: /require two-factor/i })).not.toBeInTheDocument();
  });

  it("says why the selected workspace can't be opened yet", async () => {
    const dialog = await openAccount({
      cloud_status: () => ({
        ...status("member", OFF),
        current_workspace: null,
        workspaces: [],
        locked_workspaces: [{ id: "w1", name: "Acme" }],
        locked_current_workspace: { id: "w1", name: "Acme" },
      }),
    });
    expect(
      await within(dialog).findByText(/acme requires two-factor authentication before you can open it/i),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText(/personal \(local-only\) mode/i)).not.toBeInTheDocument();
  });
});
