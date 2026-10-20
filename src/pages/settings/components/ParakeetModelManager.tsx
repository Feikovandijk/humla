import { Download, Trash2 } from "lucide-react";
import type { ParakeetState } from "../types";
import { formatBytes } from "./format";

const SIZE_HINT = 630_000_000;

const iconBtn =
  "nd-btn-icon-sm shrink-0 text-[var(--color-text-muted)] hover:text-[var(--color-text)] disabled:opacity-40";

// The ninth on-device model. Parakeet Ultra is a separate engine from the
// Whisper list (FluidAudio/CoreML rather than whisper.cpp), so it gets its own
// row with its own lifecycle instead of being folded into the GGML registry.
export function ParakeetModelManager({
  state,
  selected,
  onSelect,
  onDownload,
  onDelete,
}: {
  state: ParakeetState;
  selected: boolean;
  onSelect: () => void;
  onDownload: () => void;
  onDelete: () => void;
}) {
  const downloaded = state.status?.downloaded ?? false;
  const pct = Math.round(state.fraction * 100);

  return (
    <div
      className={
        "rounded-lg border px-3 py-2.5 transition-colors " +
        (selected
          ? "border-[var(--color-accent-text)]/40 bg-[var(--color-accent-soft)]"
          : "border-transparent hover:bg-[var(--color-pill-hover)]")
      }
    >
      <div className="flex items-start gap-3">
        <button
          type="button"
          role="radio"
          aria-checked={selected}
          disabled={!downloaded}
          title={!downloaded ? "Download it first" : "Use Parakeet Ultra for new recordings"}
          onClick={onSelect}
          className="flex flex-1 min-w-0 items-start gap-3 text-left disabled:cursor-default"
        >
          <span
            aria-hidden
            className={
              "mt-1.5 size-2 shrink-0 rounded-full " +
              (selected ? "bg-[var(--color-accent-text)]" : "bg-[var(--color-line-visible)]")
            }
          />
          <span className="flex-1 min-w-0">
            <span className="flex items-baseline gap-2">
              <span
                className={
                  "truncate text-sm " +
                  (!downloaded ? "text-[var(--color-text-muted)]" : "")
                }
              >
                Parakeet Ultra
              </span>
              <span className="text-xs text-[var(--color-text-muted)]">Fastest English</span>
              <span className="ml-auto w-16 text-right text-xs tabular-nums text-[var(--color-text-muted)]">
                {state.downloading ? `${pct}%` : formatBytes(state.status?.sizeBytes ?? SIZE_HINT)}
              </span>
            </span>
            <span className="mt-0.5 block text-xs text-[var(--color-text-muted)]">
              NVIDIA Parakeet Ultra via CoreML on the Neural Engine. 25 European
              languages, English-first — no Norwegian. No custom vocabulary or
              prior-context biasing.
            </span>
            {state.downloading && (
              <span className="mt-2 block h-1 overflow-hidden rounded-full bg-[var(--color-line)]">
                <span
                  className="block h-full bg-[var(--color-accent-text)] transition-[width]"
                  style={{ width: `${pct}%` }}
                />
              </span>
            )}
          </span>
        </button>
        {downloaded ? (
          <button
            type="button"
            onClick={onDelete}
            aria-label="Delete Parakeet Ultra"
            className={iconBtn}
          >
            <Trash2 size={14} />
          </button>
        ) : (
          <button
            type="button"
            onClick={onDownload}
            disabled={state.downloading}
            aria-label="Download Parakeet Ultra"
            className={iconBtn}
          >
            <Download size={14} />
          </button>
        )}
      </div>
      {state.error && (
        <p className="mt-2 text-xs text-[var(--color-danger)] break-all">{state.error}</p>
      )}
    </div>
  );
}
