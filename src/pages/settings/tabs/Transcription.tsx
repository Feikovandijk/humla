import type { ReactNode } from "react";
import { DiarizeModelManager } from "../components/DiarizeModelManager";
import { Disclosure } from "../components/Disclosure";
import { ParakeetModelManager } from "../components/ParakeetModelManager";
import { PerLanguageOverrides } from "../components/PerLanguageOverrides";
import { LocalTuningRows } from "../components/ProviderConfigForm";
import { TranscriptionModelPicker } from "../components/TranscriptionModelPicker";
import { Row, Section } from "../components/Section";
import { RenameYouRow } from "../components/RenameYouRow";
import { Select } from "../../../components/ui/Select";
import { NEMOTRON_MAX_SPEAKERS } from "../../../lib/diarizeEngine";
import { LANGUAGES, languageOptionLabel } from "../../../lib/languages";
import { inputClass } from "../types";
import type { SettingsHook } from "../useSettings";


// Engine choice row: label + description left, pick-radio right, the
// download/status manager underneath. The radio is presence-gated — an
// engine can't be active until its models are on disk.
function EngineOption({
  label,
  description,
  checked,
  disabled,
  onPick,
  children,
}: {
  label: string;
  description: string;
  checked: boolean;
  disabled: boolean;
  onPick: () => void;
  children: ReactNode;
}) {
  return (
    <div className="py-3.5">
      <div className="flex items-start justify-between gap-6">
        <div className="min-w-0">
          <div className="text-sm">{label}</div>
          <p className="text-xs text-[var(--color-text-muted)] mt-0.5">
            {description}
          </p>
        </div>
        <input
          type="radio"
          name="diarize_model"
          checked={checked}
          disabled={disabled}
          onChange={onPick}
          aria-label={`Use ${label} for new recordings`}
          className="mt-1 shrink-0"
        />
      </div>
      <div className="mt-3">{children}</div>
    </div>
  );
}

// Small right-aligned mono numeric field with the row model's label +
// description — the threshold knobs' shared shape.
function ThresholdRow({
  label,
  description,
  value,
  placeholder,
  onChange,
}: {
  label: string;
  description: string;
  value: string;
  placeholder: string;
  onChange: (v: string) => void;
}) {
  return (
    <Row
      label={label}
      description={description}
      control={
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          aria-label={label}
          className="w-24 text-sm text-right px-2.5 py-1.5 rounded-md border border-[var(--color-line-visible)] bg-[var(--color-surface)] focus:border-[var(--color-text-muted)] transition-colors"
          style={{ fontFamily: "var(--font-mono)" }}
        />
      }
    />
  );
}

export function TranscriptionTab({
  s,
  update,
  transcribeConfig,
  setDefaultConfig,
  setLanguageOverride,
  removeLanguageOverride,
  local,
  downloadModel,
  deleteModel,
  parakeet,
  downloadParakeet,
  deleteParakeet,
  diarize,
  downloadDiarize,
  deleteDiarize,
}: Pick<
  SettingsHook,
  | "s"
  | "update"
  | "transcribeConfig"
  | "setDefaultConfig"
  | "setLanguageOverride"
  | "removeLanguageOverride"
  | "local"
  | "downloadModel"
  | "deleteModel"
  | "parakeet"
  | "downloadParakeet"
  | "deleteParakeet"
  | "diarize"
  | "downloadDiarize"
  | "deleteDiarize"
>) {
  const def = transcribeConfig.default;

  return (
    <>
      <Section title="Transcription">
        <TranscriptionModelPicker
          value={def}
          onChange={setDefaultConfig}
          local={local}
          onDownload={downloadModel}
          onDelete={deleteModel}
          setLanguageOverride={setLanguageOverride}
        />
        <ParakeetModelManager
          state={parakeet}
          selected={def.provider === "parakeet"}
          onSelect={() => setDefaultConfig({ provider: "parakeet", model: "ultra" })}
          onDownload={downloadParakeet}
          onDelete={deleteParakeet}
        />
        {def.provider === "parakeet" && !parakeet.status?.downloaded && (
          <p className="text-xs text-[var(--color-danger)] py-3">
            Parakeet Ultra isn't downloaded. Download it above before recording.
          </p>
        )}
        {def.provider === "local" && (
          <LocalTuningRows value={def} onChange={setDefaultConfig} />
        )}
        <Row
          label="Language"
          description="Default for new notes. Each recording asks you to choose English or Dutch after it starts."
          control={
            <Select
              value={s.language}
              onChange={(v) => update("language", v)}
              options={LANGUAGES.map((l) => ({
                value: l.value,
                label: languageOptionLabel(l),
              }))}
            />
          }
        />
        <Disclosure label="Advanced">
          <div>
            <div className="text-sm">Per-language overrides</div>
            <p className="text-xs text-[var(--color-text-muted)] mt-0.5 mb-3">
              Route specific recording languages to a different model than
              the default above, such as Parakeet Ultra for English.
            </p>
            <PerLanguageOverrides
              config={transcribeConfig}
              setLanguageOverride={setLanguageOverride}
              removeLanguageOverride={removeLanguageOverride}
              local={local}
              parakeetDownloaded={parakeet.status?.downloaded ?? false}
            />
          </div>
        </Disclosure>
      </Section>

      <Section title="Speaker labels">
        <p className="text-xs text-[var(--color-text-muted)] py-3">
          After a recording stops, each voice in it is labelled{" "}
          <code>Speaker 1:</code>, <code>Speaker 2:</code> and so on. Both
          engines run on your Mac, on the Neural Engine.
        </p>
        <EngineOption
          label="Nemotron 3 (end-to-end)"
          description={`NVIDIA Nemotron 3. Up to ${NEMOTRON_MAX_SPEAKERS} speakers, counts them itself. A note whose speaker count is set above ${NEMOTRON_MAX_SPEAKERS} uses Community-1 instead.`}
          checked={s.diarize_model === "nemotron3"}
          disabled={!diarize.nemotron3.status?.downloaded}
          onPick={() => update("diarize_model", "nemotron3")}
        >
          <DiarizeModelManager
            state={diarize.nemotron3}
            cost="The model is 193 MB. Setting it up also prepares it for the Neural Engine, which takes about two minutes, once."
            onDownload={() => downloadDiarize("nemotron3")}
            onDelete={() => deleteDiarize("nemotron3")}
          />
          {s.diarize_model === "nemotron3" &&
            diarize.nemotron3.status?.downloaded === false &&
            diarize.community1.status?.downloaded && (
              <p className="text-xs text-[var(--color-text-muted)] mt-2">
                Until it’s ready, recordings are labelled with Community-1.
              </p>
            )}
        </EngineOption>
        <EngineOption
          label="Community-1 (clustering)"
          description={`Pyannote community-1 segmentation and VBx clustering. Uses the note’s speaker count; on Auto it can merge a meeting one person dominates into a single speaker. It also labels any note set above ${NEMOTRON_MAX_SPEAKERS} speakers, so keep it downloaded.`}
          checked={s.diarize_model === "community1"}
          disabled={!diarize.community1.status?.downloaded}
          onPick={() => update("diarize_model", "community1")}
        >
          <DiarizeModelManager
            state={diarize.community1}
            cost="The model is about 22 MB. Setting it up also compiles it for the Neural Engine, which takes 20–30 s."
            onDownload={() => downloadDiarize("community1")}
            onDelete={() => deleteDiarize("community1")}
          />
        </EngineOption>
        <Disclosure label="Advanced">
          <ThresholdRow
            label="Community-1 clustering threshold"
            description="Higher = more aggressive separation (more speakers); lower = more merging. Default 0.5. Community-1 only."
            value={s.community1_threshold}
            placeholder="0.5"
            onChange={(v) => update("community1_threshold", v)}
          />
          <ThresholdRow
            label="Silence RMS threshold"
            description="Chunks quieter than this are skipped before Whisper sees them — prevents hallucinations on near-silence and mic hiss; too high and quiet speech gets cut. Default 0.008 (pure silence ≈ 0.0001, room tone ≈ 0.001, soft speech ≈ 0.01+)."
            value={s.silence_rms_threshold}
            placeholder="0.008"
            onChange={(v) => update("silence_rms_threshold", v)}
          />
          <p className="text-xs text-[var(--color-text-muted)]">
            Tweaks apply on the next recording or re-diarize. Diagnostic
            JSON is dumped per run — open the Note's diagnostics folder
            from its header to inspect where shifts landed.
          </p>
        </Disclosure>

        {/* Count-gated, so it is absent for a new install and gone for good once
            used (#116 part 2). In Transcription rather than Chat because the row
            is about what transcripts say. */}
        <RenameYouRow />
      </Section>

      <Section title="Vocabulary">
        <Row label="Custom terms">
          <textarea
            value={s.custom_vocabulary}
            onChange={(e) => update("custom_vocabulary", e.target.value)}
            rows={3}
            placeholder="Tauri, Transcriberr, ScreenCaptureKit, Granola"
            className={inputClass + " leading-relaxed"}
            style={{ fontFamily: "var(--font-mono)" }}
          />
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            Comma- or newline-separated. Names, jargon, and uncommon
            spellings — biases the transcriber toward these tokens.
          </p>
        </Row>
      </Section>
    </>
  );
}
