import { expect, it } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { renderApp } from "../test/app";
import { makeNote } from "../test/fixtures";

it("lets the backend check English and Dutch instead of blocking on the old default provider", async () => {
  const note = makeNote({ id: "n1", language: "no" });
  let started = false;
  renderApp("/note/n1", {
    notes_list: () => [note],
    notes_get: () => note,
    provider_key_get: () => null,
    recording_start: () => { started = true; },
  });
  await userEvent.click(await screen.findByRole("button", { name: "Record" }));
  await waitFor(() => expect(started).toBe(true));
});
