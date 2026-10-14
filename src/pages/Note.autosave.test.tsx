import { beforeAll, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router-dom";
import App from "../App";
import { mockTauri } from "../test/tauri";
import { makeNote } from "../test/fixtures";
import { mockLayoutBox } from "../test/layout";

beforeAll(() => mockLayoutBox());

// `/note/:id` re-renders the same Note component when the id changes, so the
// autosave queue outlives a switch between notes. These pin that an edit is
// saved to the note it was typed into, whichever note is on screen when its
// batch goes out.
function GoTo({ path }: { path: string }) {
  const navigate = useNavigate();
  return <button onClick={() => navigate(path)}>go to {path}</button>;
}

function renderNotes() {
  const a = makeNote({ id: "a", title: "Planning" });
  const b = makeNote({ id: "b", title: "Retro" });
  const save = vi.fn();
  mockTauri({
    notes_list: () => [a, b],
    notes_get: (args) => ((args as { id: string }).id === "a" ? a : b),
    notes_update: save,
  });
  render(
    <MemoryRouter initialEntries={["/note/a"]}>
      <App />
      <GoTo path="/note/b" />
    </MemoryRouter>,
  );
  return { save };
}

const titleBox = () => screen.getByPlaceholderText("New note");

describe("note autosave", () => {
  it("sends quick successive edits as one save of the latest value", async () => {
    const { save } = renderNotes();
    await waitFor(() => expect(titleBox()).toHaveValue("Planning"));
    fireEvent.change(titleBox(), { target: { value: "Planning v2" } });
    fireEvent.change(titleBox(), { target: { value: "Planning v3" } });

    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save).toHaveBeenCalledWith({ id: "a", patch: expect.objectContaining({ title: "Planning v3" }) });
    // Nothing else is queued behind it.
    await act(() => new Promise((r) => setTimeout(r, 400)));
    expect(save).toHaveBeenCalledTimes(1);
  });
});
