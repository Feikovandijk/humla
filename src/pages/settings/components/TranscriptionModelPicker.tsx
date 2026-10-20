import { Download, Trash2 } from "lucide-react";
import type { ProviderConfig } from "../../../lib/ipc";
import { LANGUAGES } from "../../../lib/languages";
import { useDownloadStore } from "../../../lib/store";
import type { LocalState } from "../types";
import { formatBytes } from "./format";

// One-word character of each model, shown right-aligned on its row.
const TAG: Record<string, string> = {
  "large-v3-turbo-q5": "Recommended",
  "large-v3-q5": "Most accurate",
  "large-v2-q5": "Accented speech",
  "medium-q5": "Fastest",
};

function languageOnly(code: string | null) {
  const name = LANGUAGES.find((l) => l.value === code)?.label ?? code ?? "One language";
  return `${name} only`;
}

const iconBtn =
  "nd-btn-icon-sm shrink-0 text-[var(--color-text-muted)] hover:text-[var(--color-text)] disabled:opacity-40";

// OpenWhisper-style list of on-device Whisper models: one row per model, a
// dot for the default, download / delete in place — the list is also the
// model manager.
export function TranscriptionModelPicker({
  value,
  onChange,
  local,
  onDownload,
  onDelete,
  setLanguageOverride,
}: {
  value: ProviderConfig;
  onChange: (next: ProviderConfig) => void;
  local: LocalState;
  onDownload: (id: string) => void;
  onDelete: (id: string) => void;
  setLanguageOverride: (language: string, cfg: ProviderConfig) => Promise<void>;
}) {
  const active = useDownloadStore((s) => s.active);
  const current = value.provider === "local" ? value.model_id : null;

  return (
    <div className="py-3.5">
      <div role="radiogroup" aria-label="Local models">
        {local.models.map((m) => {
          const mine = active?.modelId === m.id ? active : null;
          const pct = mine?.total ? Math.round((mine.received / mine.total) * 100) : 0;
          const multilingual = m.kind === "multilingual";
          const selected = current === m.id;
          const disabled = !m.downloaded || !multilingual;
          return (
            <div
              key={m.id}
              className={
                "group flex items-center gap-2 rounded-lg border pr-2 transition-colors " +
                (selected
                  ? "border-[var(--color-accent-text)]/40 bg-[var(--color-accent-soft)]"
                  : "border-transparent hover:bg-[var(--color-pill-hover)]")
              }
            >
              <button
                type="button"
                role="radio"
                aria-checked={selected}
                disabled={disabled}
                title={
                  !m.downloaded
                    ? "Download it first"
                    : !multilingual
                      ? "Use it through a per-language override under Advanced"
                      : m.description
                }
                onClick={() =>
                  onChange(
                    value.provider === "local"
                      ? { ...value, model_id: m.id }
                      : { provider: "local", model_id: m.id, preset: "quality", use_gpu: true },
                  )
                }
                className="flex flex-1 min-w-0 items-center gap-3 px-3 py-2.5 text-left disabled:cursor-default"
              >
                <span
                  aria-hidden
                  className={
                    "size-2 shrink-0 rounded-full " +
                    (selected ? "bg-[var(--color-accent-text)]" : "bg-[var(--color-line-visible)]")
                  }
                />
                <span
                  className={
                    "flex-1 truncate text-sm " + (disabled ? "text-[var(--color-text-muted)]" : "")
                  }
                >
                  {m.label}
                </span>
                <span className="text-xs text-[var(--color-text-muted)]">
                  {multilingual ? TAG[m.id] : languageOnly(m.specificLanguage)}
                </span>
                <span className="w-16 text-right text-xs tabular-nums text-[var(--color-text-muted)]">
                  {mine ? `${pct}%` : formatBytes(m.sizeBytes ?? m.sizeBytesHint)}
                </span>
              </button>
              {m.downloaded ? (
                <button
                  type="button"
                  onClick={() => onDelete(m.id)}
                  aria-label={`Delete ${m.label}`}
                  className={iconBtn + " opacity-0 group-hover:opacity-100 focus-visible:opacity-100"}
                >
                  <Trash2 size={14} />
                </button>
              ) : (
                // One download at a time app-wide; the backend serialises them anyway.
                <button
                  type="button"
                  onClick={() => onDownload(m.id)}
                  disabled={active !== null}
                  aria-label={`Download ${m.label}`}
                  className={iconBtn}
                >
                  <Download size={14} />
                </button>
              )}
            </div>
          );
        })}
      </div>
      {current === null ? (
        <p className="text-xs text-[var(--color-warning-text)] mt-2">
          Pick a model above — recordings won’t transcribe on this Mac until one is chosen.
        </p>
      ) : (
        !local.models.some((m) => m.id === current && m.downloaded) && (
          <p className="text-xs text-[var(--color-warning-text)] mt-2">
            The selected model isn’t downloaded — recording won’t start until it is.
          </p>
        )
      )}
      {local.flash && (
        <div
          className="flex items-center gap-3 px-3 py-2 mt-3 rounded bg-[var(--color-pill-hover)] text-xs"
          role="status"
        >
          <span className="break-all">{local.flash.message}</span>
          {local.flash.kind === "suggest_language_override" && (
            <button
              type="button"
              onClick={() => {
                if (local.flash?.kind !== "suggest_language_override") return;
                setLanguageOverride(local.flash.language, {
                  provider: "local",
                  model_id: local.flash.modelId,
                  preset: "quality",
                  use_gpu: true,
                });
              }}
              className="ml-auto text-xs px-2 py-1 rounded border border-[var(--color-line)] hover:bg-[var(--color-canvas)] whitespace-nowrap"
            >
              Add as override
            </button>
          )}
        </div>
      )}
      {local.error && (
        <p className="text-sm text-[var(--color-danger)] break-all mt-2">{local.error}</p>
      )}
    </div>
  );
}
