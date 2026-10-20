import { useState } from "react";
import { LANGUAGES, languageOptionLabel } from "../../../lib/languages";
import type { ProviderConfig, TranscribeConfig } from "../../../lib/ipc";
import { ProviderConfigForm, localModelsFor, type OnDeviceConfig } from "./ProviderConfigForm";
import { Select } from "../../../components/ui/Select";
import type { LocalState } from "../types";

// Section that lists existing per-language overrides and offers a + Add
// form to create new ones. Each entry is rendered as a card with a
// one-line summary plus a [×] delete button. No inline edit — to change
// an existing override, the user deletes and re-adds. Keeps the UI
// simple; revisit if friction surfaces.
export function PerLanguageOverrides({
  config,
  setLanguageOverride,
  removeLanguageOverride,
  local,
  parakeetDownloaded = false,
}: {
  config: TranscribeConfig;
  setLanguageOverride: (language: string, cfg: ProviderConfig) => Promise<void>;
  removeLanguageOverride: (language: string) => Promise<void>;
  local: LocalState;
  parakeetDownloaded?: boolean;
}) {
  const [showAddForm, setShowAddForm] = useState(false);
  const entries = Object.entries(config.per_language).sort(([a], [b]) =>
    a.localeCompare(b),
  );

  return (
    <div className="flex flex-col gap-3">
      <p className="text-xs text-[var(--color-text-muted)]">
        Override the default model for specific recording languages.
        Useful when one model is materially better for a given language —
        e.g. Parakeet Ultra for English.
      </p>

      {entries.length > 0 ? (
        <div className="flex flex-col gap-2">
          {entries.map(([language, cfg]) => (
            <OverrideRow
              key={language}
              language={language}
              cfg={cfg}
              onDelete={() => removeLanguageOverride(language)}
            />
          ))}
        </div>
      ) : (
        <p className="text-xs text-[var(--color-text-muted)] italic">
          No overrides — every recording uses the default model.
        </p>
      )}

      {!showAddForm && (
        <button
          type="button"
          onClick={() => setShowAddForm(true)}
          className="self-start text-sm px-3 py-1.5 rounded-md border border-[var(--color-line)] hover:bg-[var(--color-pill-hover)]"
        >
          + Add language override
        </button>
      )}
      {showAddForm && (
        <AddOverrideForm
          existingLanguages={Object.keys(config.per_language)}
          local={local}
          parakeetDownloaded={parakeetDownloaded}
          onCancel={() => setShowAddForm(false)}
          onAdd={async (language, cfg) => {
            await setLanguageOverride(language, cfg);
            setShowAddForm(false);
          }}
        />
      )}
    </div>
  );
}

function OverrideRow({
  language,
  cfg,
  onDelete,
}: {
  language: string;
  cfg: ProviderConfig;
  onDelete: () => void;
}) {
  const lang = LANGUAGES.find((l) => l.value === language);
  return (
    <div className="flex items-start gap-3 px-3 py-2 rounded-md border border-[var(--color-line)]">
      <div className="flex-1 flex flex-col gap-0.5">
        <div className="text-sm font-medium">
          {lang ? languageOptionLabel(lang) : language}
        </div>
        <div className="text-xs text-[var(--color-text-muted)]">
          {summariseProvider(cfg)}
        </div>
      </div>
      <button
        type="button"
        onClick={onDelete}
        aria-label={`Remove ${language} override`}
        className="text-sm px-2 py-1 rounded hover:bg-[var(--color-pill-hover)]"
      >
        ×
      </button>
    </div>
  );
}

function summariseProvider(cfg: ProviderConfig): string {
  switch (cfg.provider) {
    case "openai":
      return `OpenAI · ${cfg.model}`;
    case "deepgram":
      return `Deepgram · ${cfg.model}`;
    case "groq":
      return `Groq · ${cfg.model}`;
    case "local":
      return `Local · ${cfg.model_id} · ${cfg.preset} preset · GPU ${cfg.use_gpu ? "on" : "off"}`;
    case "parakeet":
      return `Parakeet · ${cfg.model}`;
  }
}

function AddOverrideForm({
  existingLanguages,
  local,
  parakeetDownloaded = false,
  onCancel,
  onAdd,
}: {
  existingLanguages: string[];
  local: LocalState;
  parakeetDownloaded?: boolean;
  onCancel: () => void;
  onAdd: (language: string, cfg: ProviderConfig) => Promise<void>;
}) {
  // Filter the language picker to languages NOT already overridden.
  // Drop "auto" — overrides apply to specific recording languages, not
  // the auto-detect sentinel.
  const available = LANGUAGES.filter(
    (l) => l.value !== "auto" && !existingLanguages.includes(l.value),
  );
  const initialLanguage = available[0]?.value ?? "no";
  const [language, setLanguage] = useState(initialLanguage);
  const [picked, setPicked] = useState<OnDeviceConfig>({
    provider: "local",
    model_id: "",
    preset: "quality",
    use_gpu: true,
  });
  // A model picked for one language may not suit the next; fall back to the
  // first that does.
  const usable = localModelsFor(local.models, language);
  const cfg: OnDeviceConfig =
    (picked.provider === "parakeet" && parakeetDownloaded) ||
    (picked.provider === "local" && usable.some((m) => m.id === picked.model_id))
      ? picked
      : usable.length === 0 && parakeetDownloaded
        ? { provider: "parakeet", model: "ultra" }
        : { provider: "local", model_id: usable[0]?.id ?? "", preset: "quality", use_gpu: true };

  return (
    <div className="flex flex-col gap-3 px-3 py-3 rounded-md border border-[var(--color-line)] bg-[var(--color-canvas)]">
      <div className="text-xs uppercase tracking-wide text-[var(--color-text-muted)]">
        New override
      </div>
      <Select
        value={language}
        onChange={setLanguage}
        options={available.map((l) => ({
          value: l.value,
          label: languageOptionLabel(l),
        }))}
      />
      {usable.length > 0 || parakeetDownloaded ? (
        <ProviderConfigForm
          value={cfg}
          onChange={setPicked}
          localModels={local.models}
          filterLocalToLanguage={language}
          parakeetDownloaded={parakeetDownloaded}
        />
      ) : (
        <p className="text-xs text-[var(--color-text-muted)]">
          No downloaded model can transcribe this language — download one in the list above.
        </p>
      )}
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="text-sm px-3 py-1.5 rounded-md border border-[var(--color-line)] hover:bg-[var(--color-pill-hover)]"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={() => onAdd(language, cfg)}
          disabled={usable.length === 0 && !parakeetDownloaded}
          className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-text)] text-[var(--color-canvas)] disabled:opacity-40"
        >
          Add
        </button>
      </div>
    </div>
  );
}
