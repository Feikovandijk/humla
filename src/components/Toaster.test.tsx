import { afterEach, describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { Toaster } from "./Toaster";
import { useRecordingStore } from "../lib/store";

afterEach(() => useRecordingStore.setState({ errors: [], flashes: [] }));

function renderToaster() {
  render(
    <MemoryRouter>
      <Toaster />
    </MemoryRouter>,
  );
}

describe("Toaster", () => {
  it("heads each error with the action that failed", () => {
    renderToaster();
    act(() => {
      const { pushError } = useRecordingStore.getState();
      pushError({ noteId: "n1", message: "disk full", kind: "export" });
      pushError({ noteId: "n1", message: "HTTP 500", kind: "webhook" });
      pushError({ noteId: "n1", message: "rate limited", kind: "summary" });
    });
    expect(screen.getByText("Export failed")).toBeInTheDocument();
    expect(screen.getByText("Couldn’t send to webhook")).toBeInTheDocument();
    expect(screen.getByText("Summary failed")).toBeInTheDocument();
    expect(screen.queryByText("Recording issue")).not.toBeInTheDocument();
  });

  it("treats an error with no kind as a recording issue", () => {
    renderToaster();
    act(() => useRecordingStore.getState().pushError({ noteId: null, message: "sidecar exited" }));
    expect(screen.getByText("Recording issue")).toBeInTheDocument();
  });
});
