import { useEffect, useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { ipc, type WebhookConfig } from "../../../lib/ipc";
import { broadcastSettingChange } from "../../../lib/settingsBus";
import { Row, Section } from "../components/Section";
import { Toggle } from "../components/Toggle";
import { inputClass } from "../types";

export function WebhookSection() {
  const [config, setConfig] = useState<WebhookConfig | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    let active = true;
    ipc.webhookGetConfig().then((value) => {
      if (active) setConfig(value);
    }).catch((e) => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, []);

  function update(next: WebhookConfig) {
    setConfig(next);
    setSaved(false);
    setError(null);
  }

  async function save() {
    if (!config || saving) return;
    setSaving(true);
    setError(null);
    setSaved(false);
    try {
      await ipc.webhookSetConfig(config);
      broadcastSettingChange("webhook_enabled", String(config.enabled));
      setSaved(true);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Section title="Webhook">
      {!config ? (
        <p className="py-3 text-sm" role={error ? "alert" : "status"}>{error ?? "Loading webhook settings…"}</p>
      ) : (
        <>
          <Row label="Send notes to a webhook"
            description="Adds a send button to each note. Send the current notes, summary, transcript and speaker labels as JSON when you choose."
            control={<Toggle label="Enable webhook" checked={config.enabled} disabled={saving}
              onChange={(enabled) => update({ ...config, enabled })} />} />
          <Row label="Webhook URL">
            <input aria-label="Webhook URL" type="url" autoComplete="off" spellCheck={false}
              className={inputClass} placeholder="https://example.com/webhook" value={config.url} disabled={saving}
              onChange={(e) => update({ ...config, url: e.target.value })} />
          </Row>
          <Row label="Headers (optional)">
            <p className="text-xs text-[var(--color-text-muted)] mb-3">For example, Authorization with a value of Bearer your-token. Content-Type is set to application/json.</p>
            <div className="space-y-2">
              {config.headers.map((header, index) => (
                <div className="flex items-center gap-2" key={index}>
                  <input aria-label={`Header ${index + 1} name`} placeholder="Authorization" autoComplete="off" spellCheck={false}
                    className={`${inputClass} min-w-0 flex-1`} value={header.name} disabled={saving}
                    onChange={(e) => update({ ...config, headers: config.headers.map((h, i) => i === index ? { ...h, name: e.target.value } : h) })} />
                  <input aria-label={`Header ${index + 1} value`} placeholder="Bearer your-token" type="password" autoComplete="off"
                    className={`${inputClass} min-w-0 flex-1`} value={header.value} disabled={saving}
                    onChange={(e) => update({ ...config, headers: config.headers.map((h, i) => i === index ? { ...h, value: e.target.value } : h) })} />
                  <button type="button" className="nd-btn-icon shrink-0" aria-label={`Remove header ${index + 1}`} disabled={saving}
                    onClick={() => update({ ...config, headers: config.headers.filter((_, i) => i !== index) })}>
                    <Trash2 size={15} />
                  </button>
                </div>
              ))}
              <button type="button" className="nd-btn" disabled={saving}
                onClick={() => update({ ...config, headers: [...config.headers, { name: "", value: "" }] })}>
                <Plus size={14} /> Add header
              </button>
            </div>
          </Row>
          <div className="py-3 space-y-3">
            <p className="text-xs text-[var(--color-text-muted)]">The URL and headers are stored in macOS Keychain on this device. Use HTTPS for remote endpoints. Audio files are not sent.</p>
            {error && <p role="alert" className="text-sm text-[var(--color-danger)]">{error}</p>}
            <div className="flex items-center gap-3">
              <button type="button" className="nd-btn nd-btn-primary" disabled={saving} onClick={() => void save()}>
                {saving ? "Saving…" : "Save webhook"}
              </button>
              {saved && <span role="status" className="text-xs text-[var(--color-text-muted)]">Webhook settings saved</span>}
            </div>
          </div>
        </>
      )}
    </Section>
  );
}
