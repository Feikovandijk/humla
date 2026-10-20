import { useState } from "react";
import { ipc } from "../lib/ipc";
import { LANGUAGES, languageOptionLabel } from "../lib/languages";
import { useRecordingStore } from "../lib/store";
import { Modal } from "../pages/settings/components/Modal";

const keepOpen = () => {};

export function RecordingLanguageDialog() {
  const status = useRecordingStore((s) => s.status);
  if (!status.noteId || !status.languageSessionId) return null;
  return <LanguageChoice key={status.languageSessionId} noteId={status.noteId} sessionId={status.languageSessionId} />;
}

function LanguageChoice({ noteId, sessionId }: { noteId: string; sessionId: string }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  async function select(language: string) {
    setBusy(true);
    setError("");
    try {
      await ipc.recordingSelectLanguage(noteId, sessionId, language);
      useRecordingStore.setState((state) =>
        state.status.languageSessionId === sessionId
          ? { status: { ...state.status, languageSessionId: undefined } }
          : {},
      );
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <Modal open onClose={keepOpen} title="Choose the recording language" size="sm">
      <p className="text-sm text-[var(--color-text-muted)]">
        Recording has started. Choose the language spoken so Humla can transcribe it.
      </p>
      <div className="flex gap-3 mt-5">
        {LANGUAGES.map((language) => (
          <button key={language.value} type="button" className="nd-btn" disabled={busy} onClick={() => void select(language.value)}>
            {languageOptionLabel(language)}
          </button>
        ))}
      </div>
      {error && <p role="alert" className="mt-3 text-sm text-[var(--color-danger)]">{error}</p>}
    </Modal>
  );
}
