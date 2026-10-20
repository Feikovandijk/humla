import { useEffect, useState } from "react";
import { Download, Trash2 } from "lucide-react";
import type { ProviderConfig } from "../../../lib/ipc";
import { LANGUAGES } from "../../../lib/languages";
import { useDownloadStore } from "../../../lib/store";
import {
  DEEPGRAM_MODELS,
  GROQ_MODELS,
  TRANSCRIBE_MODELS,
  type LocalState,
} from "../types";
import { formatBytes } from "./format";

type CloudProvider = "openai" | "deepgram" | "groq";

const CLOUD: { id: CloudProvider; label: string; models: string[] }[] = [
  { id: "openai", label: "OpenAI", models: TRANSCRIBE_MODELS },
  { id: "deepgram", label: "Deepgram", models: DEEPGRAM_MODELS },
  { id: "groq", label: "Groq", models: GROQ_MODELS },
];

// One-word character of each model, shown right-aligned on its row.
const TAG: Record<string, string> = {
  "whisper-1": "Reliable",
  "gpt-4o-mini-transcribe": "Fast",
  "gpt-4o-transcribe": "Accurate",
  "gpt-4o-transcribe-diarize": "Diarizes",
  "nova-3": "Best",
  "nova-2": "Previous",
  base: "Cheapest",
  "whisper-large-v3-turbo": "Fast · cheap",
  "large-v3-turbo-q5": "Recommended",
  "large-v3-q5": "Most accurate",
  "large-v2-q5": "Accented speech",
  "medium-q5": "Fastest",
};

function languageOnly(code: string | null) {
  const name = LANGUAGES.find((l) => l.value === code)?.label ?? code ?? "One language";
  return `${name} only`;
}

const tabClass = (on: boolean) =>
  "pb-1 text-sm border-b-2 transition-colors " +
  (on
    ? "border-[var(--color-text)] text-[var(--color-text)]"
    : "border-transparent text-[var(--color-text-muted)] hover:text-[var(--color-text)]");

const chipClass = (on: boolean) =>
  "px-3 py-1.5 rounded-md text-sm transition-colors " +
  (on
    ? "bg-[var(--color-surface-2)] text-[var(--color-text)] shadow-[var(--shadow-card)]"
    : "text-[var(--color-text-muted)] hover:text-[var(--color-text)]");

function ModelRow({
  name,
  tag,
  detail,
  selected,
  disabled,
  title,
  onSelect,
  action,
}: {
  name: string;
  tag?: string;
  detail?: string;
  selected: boolean;
  disabled?: boolean;
  title?: string;
  onSelect: () => void;
  action?: React.ReactNode;
}) {
  return (
    <div
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
        title={title}
        onClick={onSelect}
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
            "flex-1 truncate text-sm " +
            (disabled ? "text-[var(--color-text-muted)]" : "")
          }
        >
          {name}
        </span>
        {tag && <span className="text-xs text-[var(--color-text-muted)]">{tag}</span>}
        {detail && (
          <span className="w-16 text-right text-xs tabular-nums text-[var(--color-text-muted)]">
            {detail}
          </span>
        )}
      </button>
      {action}
    </div>
  );
}

// Granola/OpenWhisper-style picker for the default transcription model:
// Cloud | Local, a provider row for cloud, then one row per model. Local
// models download and delete in place, so the list is also the model manager.
export function TranscriptionModelPicker({
  value,
  onChange,
  local,
  onDownload,
  onDelete,
  setLanguageOverride,
  cloudFooter,
  localFooter,
}: {
  value: ProviderConfig;
  onChange: (next: ProviderConfig) => void;
  local: LocalState;
  onDownload: (id: string) => void;
  onDelete: (id: string) => void;
  setLanguageOverride: (language: string, cfg: ProviderConfig) => Promise<void>;
  // Rendered under the list for the browsed cloud provider (its API key).
  cloudFooter: (provider: CloudProvider) => React.ReactNode;
  localFooter: React.ReactNode;
}) {
  const [mode, setMode] = useState<"cloud" | "local">(
    value.provider === "local" ? "local" : "cloud",
  );
  const [browsed, setBrowsed] = useState<CloudProvider>(
    value.provider === "local" ? "openai" : value.provider,
  );
  // The config loads after mount; follow the stored provider when it arrives.
  useEffect(() => {
    if (value.provider === "local") setMode("local");
    else {
      setMode("cloud");
      setBrowsed(value.provider);
    }
  }, [value.provider]);
  const active = useDownloadStore((s) => s.active);
  const cloud = CLOUD.find((c) => c.id === browsed)!;

  const iconBtn =
    "nd-btn-icon-sm shrink-0 text-[var(--color-text-muted)] hover:text-[var(--color-text)] disabled:opacity-40";

  return (
    <div className="py-3.5">
      <div className="flex gap-5 mb-3" role="tablist" aria-label="Transcription location">
        {(["cloud", "local"] as const).map((m) => (
          <button
            key={m}
            type="button"
            role="tab"
            aria-selected={mode === m}
            onClick={() => setMode(m)}
            className={tabClass(mode === m)}
          >
            {m === "cloud" ? "Cloud" : "Local"}
          </button>
        ))}
      </div>

      {mode === "cloud" ? (
        <>
          <div className="flex gap-1 mb-2" role="tablist" aria-label="Cloud provider">
            {CLOUD.map((c) => (
              <button
                key={c.id}
                type="button"
                role="tab"
                aria-selected={browsed === c.id}
                onClick={() => setBrowsed(c.id)}
                className={chipClass(browsed === c.id)}
              >
                {c.label}
              </button>
            ))}
          </div>
          <div role="radiogroup" aria-label={`${cloud.label} models`}>
            {cloud.models.map((m) => (
              <ModelRow
                key={m}
                name={m}
                tag={TAG[m]}
                selected={value.provider === browsed && "model" in value && value.model === m}
                onSelect={() =>
                  onChange({ provider: browsed, model: m } as ProviderConfig)
                }
              />
            ))}
          </div>
          {cloudFooter(browsed)}
        </>
      ) : (
        <>
          <div role="radiogroup" aria-label="Local models">
            {local.models.map((m) => {
              const mine = active?.modelId === m.id ? active : null;
              const pct = mine?.total ? Math.round((mine.received / mine.total) * 100) : null;
              const multilingual = m.kind === "multilingual";
              return (
                <ModelRow
                  key={m.id}
                  name={m.label}
                  tag={multilingual ? TAG[m.id] : languageOnly(m.specificLanguage)}
                  detail={
                    mine
                      ? `${pct ?? 0}%`
                      : formatBytes(m.sizeBytes ?? m.sizeBytesHint)
                  }
                  selected={value.provider === "local" && value.model_id === m.id}
                  disabled={!m.downloaded || !multilingual}
                  title={
                    !m.downloaded
                      ? "Download it first"
                      : !multilingual
                        ? "Use it through a per-language override under Advanced"
                        : m.description
                  }
                  onSelect={() =>
                    onChange(
                      value.provider === "local"
                        ? { ...value, model_id: m.id }
                        : { provider: "local", model_id: m.id, preset: "quality", use_gpu: true },
                    )
                  }
                  action={
                    m.downloaded ? (
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
                    )
                  }
                />
              );
            })}
          </div>
          {value.provider === "local" &&
            !local.models.some((m) => m.id === value.model_id && m.downloaded) && (
              <p className="text-xs text-[var(--color-warning)] mt-2">
                The selected model isn’t downloaded — recording won’t start until it is.
              </p>
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
          {value.provider === "local" && localFooter}
        </>
      )}
    </div>
  );
}
