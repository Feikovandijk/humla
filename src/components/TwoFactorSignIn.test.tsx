import { describe, it, expect } from "vitest";
import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/app";

// A password sign-in to an account with two-factor authentication answers
// with a code step, on every surface that signs in. Driven through Settings →
// Account; onboarding and the new-workspace sheet render the same form.

const SIGNED_OUT = {
  configured: true,
  logged_in: false,
  base_url: "https://sync.humla.team",
  user: null,
  current_workspace: null,
  workspaces: [],
  billing_enabled: false,
};

const SIGNED_IN = {
  ...SIGNED_OUT,
  logged_in: true,
  user: { id: "u1", email: "m@example.no", name: "Michael", verified: true },
};

/** A server whose sign-in asks for a code, then answers it with `verify`. */
function challengingServer(verify: (args: unknown) => unknown) {
  let signedIn = false;
  const verified: unknown[] = [];
  const handlers = {
    cloud_status: () => (signedIn ? SIGNED_IN : SIGNED_OUT),
    cloud_login: () => ({ step: "two_factor" }),
    cloud_verify_two_factor: (args: unknown) => {
      verified.push(args);
      const status = verify(args);
      signedIn = true;
      return status;
    },
  };
  return { handlers, verified };
}

async function signInWithPassword(handlers: Record<string, (args: unknown) => unknown>) {
  renderApp("/settings?tab=account", handlers);
  const dialog = await screen.findByRole("dialog", { name: /settings/i });
  await userEvent.type(within(dialog).getByPlaceholderText(/you@example.com/i), "m@example.no");
  await userEvent.type(within(dialog).getByPlaceholderText("••••••••"), "hunter22");
  await userEvent.click(within(dialog).getByRole("button", { name: /^sign in$/i }));
  return dialog;
}

describe("two-factor sign-in", () => {
  it("asks for the authenticator code and finishes signing in with it", async () => {
    const server = challengingServer(() => SIGNED_IN);
    const dialog = await signInWithPassword(server.handlers);

    const code = await within(dialog).findByLabelText(/authentication code/i);
    await userEvent.type(code, "123 456");
    await userEvent.click(within(dialog).getByRole("button", { name: /^verify$/i }));

    expect(server.verified).toEqual([{ code: "123456", recoveryCode: null }]);
    expect(await within(dialog).findByText("Michael")).toBeInTheDocument();
  });

  it("takes a recovery code instead", async () => {
    const server = challengingServer(() => SIGNED_IN);
    const dialog = await signInWithPassword(server.handlers);

    await userEvent.click(
      await within(dialog).findByRole("button", { name: /use a recovery code instead/i }),
    );
    await userEvent.type(within(dialog).getByLabelText(/recovery code/i), "ABCDE-12345");
    await userEvent.click(within(dialog).getByRole("button", { name: /^verify$/i }));

    expect(server.verified).toEqual([{ code: null, recoveryCode: "ABCDE-12345" }]);
  });

  it("stays on the code step when the code is wrong", async () => {
    const server = challengingServer(() => {
      throw { reason: "invalid_code", message: "That code isn't right." };
    });
    const dialog = await signInWithPassword(server.handlers);

    await userEvent.type(await within(dialog).findByLabelText(/authentication code/i), "000000");
    await userEvent.click(within(dialog).getByRole("button", { name: /^verify$/i }));

    expect(await within(dialog).findByText("That code isn't right.")).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/authentication code/i)).toBeInTheDocument();
  });

  it("goes back to the password when the sign-in has expired", async () => {
    const server = challengingServer(() => {
      throw {
        reason: "challenge_expired",
        message: "That sign-in has expired. Enter your password again.",
      };
    });
    const dialog = await signInWithPassword(server.handlers);

    await userEvent.type(await within(dialog).findByLabelText(/authentication code/i), "123456");
    await userEvent.click(within(dialog).getByRole("button", { name: /^verify$/i }));

    expect(
      await within(dialog).findByText("That sign-in has expired. Enter your password again."),
    ).toBeInTheDocument();
    expect(within(dialog).getByPlaceholderText("••••••••")).toBeInTheDocument();
    expect(within(dialog).queryByLabelText(/authentication code/i)).not.toBeInTheDocument();
  });

  it("asks a user whose session ended to sign in again, email filled in", async () => {
    renderApp("/settings?tab=account", {
      cloud_status: () => ({ ...SIGNED_OUT, signed_out_email: "m@example.no" }),
    });
    const dialog = await screen.findByRole("dialog", { name: /settings/i });

    expect(await within(dialog).findByText(/session has ended/i)).toBeInTheDocument();
    expect(within(dialog).getByPlaceholderText(/you@example.com/i)).toHaveValue("m@example.no");
  });
});
