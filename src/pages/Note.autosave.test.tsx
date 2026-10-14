import { beforeAll, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter, useNavigate } from "react-router-dom";
import App from "../App";
import { mockTauri } from "../test/tauri";
import { makeNote } from "../test/fixtures";
import { mockLayoutBox } from "../test/layout";

beforeAll(() => mockLayoutBox());

// The note view's autosave queue (`flushSave`): edits within 300 ms go out as
// one save, and leaving the note before the timer fires still saves them, to
// the note they were typed into.

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

async function settled(save: ReturnType<typeof vi.fn>) {
  await waitFor(() => expect(titleBox()).toHaveValue("Planning"));
  // Let the editor's mount-time write go out, so what follows is only what the
  // test types.
  await act(() => new Promise((r) => setTimeout(r, 400)));
  save.mockClear();
}

describe("note autosave", () => {
  it("sends quick successive edits as one save of the latest value", async () => {
    const { save } = renderNotes();
    await settled(save);
    fireEvent.change(titleBox(), { target: { value: "Planning v2" } });
    fireEvent.change(titleBox(), { target: { value: "Planning v3" } });

    await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
    expect(save).toHaveBeenCalledWith({ id: "a", patch: { title: "Planning v3" } });
    // Nothing else is queued behind it.
    await act(() => new Promise((r) => setTimeout(r, 400)));
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("saves an edit to its own note when the user leaves before the timer fires", async () => {
    const { save } = renderNotes();
    await settled(save);

    // Leave inside the debounce window. The clock is frozen so the timer
    // can't fire first and make the unmount's flush look unnecessary.
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      fireEvent.change(titleBox(), { target: { value: "Planning v2" } });
      fireEvent.click(screen.getByRole("button", { name: "go to /note/b" }));
      await act(async () => {});
      expect(save).toHaveBeenCalledWith({ id: "a", patch: { title: "Planning v2" } });
    } finally {
      vi.useRealTimers();
    }
    await waitFor(() => expect(titleBox()).toHaveValue("Retro"));
  });
});
