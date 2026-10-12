import { useCallback, useEffect, useRef, useState } from "react";
import { Check, LoaderCircle, Upload, X } from "lucide-react";
import { useLiveSetting } from "../lib/settingsBus";
import { Modal } from "../pages/settings/components/Modal";

export function WebhookButton({ send }: { send: () => Promise<number> }) {
  const enabled = useLiveSetting("webhook_enabled") === "true";
  const inFlight = useRef(false);
  const [sending, setSending] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; message: string } | null>(null);
  const cancel = useCallback(() => setConfirming(false), []);

  useEffect(() => {
    if (!enabled) setConfirming(false);
  }, [enabled]);

  async function submit() {
    if (inFlight.current || !confirming || !enabled) return;
    inFlight.current = true;
    setConfirming(false);
    setSending(true);
    setResult(null);
    try {
      const status = await send();
      setResult({ ok: true, message: `Sent to webhook (HTTP ${status})` });
    } catch (e) {
      setResult({ ok: false, message: String(e) });
    } finally {
      inFlight.current = false;
      setSending(false);
    }
  }

  if (!enabled) return null;
  return (
    <>
      <button type="button" className="no-drag nd-btn @max-[420px]:[--control-pad-x:8px]" disabled={sending}
        title={sending ? "Sending to webhook…" : "Send to webhook"}
        aria-label={sending ? "Sending to webhook…" : "Send to webhook"}
        onClick={() => { setResult(null); setConfirming(true); }}>
        {sending ? <LoaderCircle size={16} className="animate-spin" /> : <Upload size={16} />}
        <span className="@max-[720px]:hidden">{sending ? "Sending…" : "Send"}</span>
      </button>
      <Modal open={confirming} onClose={cancel} title="Send to webhook?" size="sm">
        <p className="text-sm">Do you want to send this note and transcript to the configured webhook?</p>
        <div className="flex justify-end gap-2 mt-5">
          <button type="button" className="nd-btn" onClick={cancel}>No</button>
          <button type="button"
            className="text-sm px-3 py-1.5 rounded-md bg-[var(--color-text)] text-[var(--color-canvas)] hover:opacity-90 transition-opacity"
            onClick={() => void submit()}>Yes</button>
        </div>
      </Modal>
      {result && (
        <div role={result.ok ? "status" : "alert"}
          className="no-drag absolute top-full right-3 mt-2 max-w-sm rounded-lg border border-[var(--color-line-visible)] bg-[var(--color-surface)] p-3 shadow-lg flex items-start gap-2 text-sm">
          {result.ok && <Check size={16} className="shrink-0 mt-0.5" />}
          <span>{result.message}</span>
          <button type="button" className="nd-btn-icon shrink-0" aria-label="Dismiss webhook result" onClick={() => setResult(null)}><X size={14} /></button>
        </div>
      )}
    </>
  );
}
