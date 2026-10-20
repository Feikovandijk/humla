import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { ParakeetModelManager } from "./ParakeetModelManager";
import { EMPTY_PARAKEET_STATE, type ParakeetState } from "../types";

function state(over: Partial<ParakeetState> = {}): ParakeetState {
  return { ...EMPTY_PARAKEET_STATE, ...over };
}

describe("ParakeetModelManager", () => {
  it("downloads when the model is absent", async () => {
    const onDownload = vi.fn();
    render(
      <ParakeetModelManager
        state={state()}
        selected={false}
        onSelect={vi.fn()}
        onDownload={onDownload}
        onDelete={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: /download parakeet ultra/i }));
    expect(onDownload).toHaveBeenCalledTimes(1);
  });

  it("selects and deletes once downloaded", async () => {
    const onSelect = vi.fn();
    const onDelete = vi.fn();
    render(
      <ParakeetModelManager
        state={state({ status: { downloaded: true, needsWarmUp: false, warmUpError: null, sizeBytes: 630_000_000, path: "/x" } })}
        selected={false}
        onSelect={onSelect}
        onDownload={vi.fn()}
        onDelete={onDelete}
      />,
    );
    await userEvent.click(screen.getByRole("radio"));
    expect(onSelect).toHaveBeenCalledTimes(1);
    await userEvent.click(screen.getByRole("button", { name: /delete parakeet ultra/i }));
    expect(onDelete).toHaveBeenCalledTimes(1);
  });

  it("surfaces a download error", () => {
    render(
      <ParakeetModelManager
        state={state({ error: "network down" })}
        selected={false}
        onSelect={vi.fn()}
        onDownload={vi.fn()}
        onDelete={vi.fn()}
      />,
    );
    expect(screen.getByText("network down")).toBeInTheDocument();
  });
});
