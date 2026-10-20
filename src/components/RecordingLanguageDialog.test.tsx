import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it } from "vitest";
import { RecordingLanguageDialog } from "./RecordingLanguageDialog";
import { useRecordingStore } from "../lib/store";
import { mockTauri } from "../test/tauri";

it("requires a choice after capture starts and submits it for that capture", async () => {
  let selected: unknown;
  mockTauri({
    recording_select_language: (args) => { selected = args; },
  });
  useRecordingStore.setState({
    status: { noteId: "note-1", phase: "recording", languageSessionId: "take-1" },
  });
  render(<RecordingLanguageDialog />);
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  expect(screen.queryByText("Auto-detect")).not.toBeInTheDocument();
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.getByRole("dialog")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Dutch (Nederlands)" }));
  await waitFor(() => expect(selected).toEqual({
    noteId: "note-1", sessionId: "take-1", language: "nl",
  }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
});

it("keeps the choice visible if saving fails", async () => {
  mockTauri({ recording_select_language: () => { throw new Error("Save failed"); } });
  useRecordingStore.setState({
    status: { noteId: "note-1", phase: "recording", languageSessionId: "take-2" },
  });
  render(<RecordingLanguageDialog />);
  fireEvent.click(screen.getByRole("button", { name: "English" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Save failed");
  expect(screen.getByRole("button", { name: "English" })).toBeEnabled();
});
