import type { LocalWhisperModelStatus, ProviderConfig } from "../../../lib/ipc";
import { WHISPER_PRESETS } from "../types";
import { Row } from "./Section";
import { Select } from "../../../components/ui/Select";
import { Toggle } from "./Toggle";

type LocalConfig = Extract<ProviderConfig, { provider: "local" }>;

// The local models usable for a language: multilingual ones plus any trained
// for exactly that language. Only downloaded models are offered.
export function localModelsFor(models: LocalWhisperModelStatus[], language?: string) {
  return models.filter(
    (m) =>
      m.downloaded &&
      (m.kind === "multilingual" || (language !== undefined && m.specificLanguage === language)),
  );
}

// On-device model + tuning rows for a per-language override. Returns a fragment
// of Rows so the parent's divide-y hairlines apply between them.
export function ProviderConfigForm({
  value,
  onChange,
  localModels,
  filterLocalToLanguage,
}: {
  value: LocalConfig;
  onChange: (next: LocalConfig) => void;
  localModels: LocalWhisperModelStatus[];
  filterLocalToLanguage?: string;
}) {
  const options = localModelsFor(localModels, filterLocalToLanguage).map((m) => ({
    value: m.id,
    label: m.label,
  }));
  return (
    <>
      <Row
        label="Model"
        description="Only downloaded models are offered — download more in the list above."
        control={
          <Select
            value={value.model_id}
            onChange={(v) => onChange({ ...value, model_id: v })}
            options={
              options.length > 0
                ? options
                : [{ value: value.model_id, label: `${value.model_id} (not downloaded)` }]
            }
          />
        }
      />
      <LocalTuningRows value={value} onChange={onChange} />
    </>
  );
}

// Quality preset + Metal toggle for an on-device config; shared with the
// Transcription tab's default model.
export function LocalTuningRows<T extends LocalConfig>({
  value,
  onChange,
}: {
  value: T;
  onChange: (next: T) => void;
}) {
  return (
    <>
      <Row
        label="Quality"
        description="Speed vs accuracy for on-device Whisper — Quality uses beam search and catches more borderline words."
        control={
          <Select
            value={value.preset}
            onChange={(v) => onChange({ ...value, preset: v })}
            options={WHISPER_PRESETS}
          />
        }
      />
      <Row
        label="Use Metal (Apple GPU)"
        description="Runs Whisper inference on the GPU — much faster on Apple Silicon."
        control={
          <Toggle
            label="Use Metal (Apple GPU) for Whisper inference"
            checked={value.use_gpu}
            onChange={(on) => onChange({ ...value, use_gpu: on })}
          />
        }
      />
    </>
  );
}
