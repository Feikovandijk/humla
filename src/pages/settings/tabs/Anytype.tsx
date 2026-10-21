import { useEffect, useState } from "react";
import { open as openExternal } from "@tauri-apps/plugin-shell";
import { ipc, type AnytypeObjectType, type AnytypeSpace } from "../../../lib/ipc";
import { useAnytypeBulk } from "../../../lib/anytype";
import { Btn } from "../components/Btn";
import { Row } from "../components/Section";
import { Toggle } from "../components/Toggle";
import { Select } from "../../../components/ui/Select";
import { inputClass, type EditableKey } from "../types";

// Sync notes to Anytype (ADR 0007): rows in the Integrations section, below
// the webhook, because that is what it is — a way for the user's own knowledge
// base to receive their notes.
//
// The switch is an ordinary setting written at once, like the webhook's. It
// only decides whether a note's ⋯ menu offers the sync. The API key is a
// secret, so it lives in the Keychain and has its own Save, like the webhook's
// endpoint: the backend checks all of it together, and a half-typed key must
// never be where a note goes.
//
// Nothing here reads the Keychain until the switch is on, so a user who never
// turns it on is never asked for Keychain access on its behalf.

const DOCS_URL = "https://github.com/michaelwilhelmsen/humla/blob/main/docs/anytype.md";
const DEFAULT_BASE_URL = "http://127.0.0.1:31009";

export function AnytypeRows({
  s,
  update,
}: {
  s: Record<EditableKey, string>;
  update: (key: EditableKey, value: string) => void;
}) {
  const enabled = s.anytype_enabled === "true";
  return (
    <>
      <Row
        label="Sync notes to Anytype"
        description="Adds Sync to Anytype… to each note’s ⋯ menu, so your knowledge base gets the meeting as one object. A note is pushed only when you push it, after you confirm where."
        control={
          <Toggle
            checked={enabled}
            onChange={(next) => update("anytype_enabled", next ? "true" : "false")}
            label="Enable Anytype sync"
          />
        }
      />
      {enabled && <AnytypeForm />}
    </>
  );
}

function AnytypeForm() {
  const [loaded, setLoaded] = useState(false);
  // The key is write-only: `hasKey` says whether one is stored, the field
  // itself starts empty and an empty save keeps the stored key.
  const [hasKey, setHasKey] = useState(false);
  const [apiKey, setApiKey] = useState("");
  const [spaceId, setSpaceId] = useState("");
  const [spaceName, setSpaceName] = useState("");
  const [typeKey, setTypeKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [spaces, setSpaces] = useState<AnytypeSpace[] | null>(null);
  const [types, setTypes] = useState<AnytypeObjectType[] | null>(null);
  const [testing, setTesting] = useState(false);
  const [testError, setTestError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  // The space as last loaded or saved — NOT read off the picker at save time,
  // where it has already changed. When the user moves to a new space, the
  // notice says where the old objects stay.
  const [savedSpace, setSavedSpace] = useState<{ id: string; name: string } | null>(null);
  const [spaceChanged, setSpaceChanged] = useState<string | null>(null);
  const bulk = useAnytypeBulk();

  useEffect(() => {
    let cancelled = false;
    ipc
      .anytypeConfigGet()
      .then((cfg) => {
        if (cancelled) return;
        setHasKey(cfg.hasKey);
        setSpaceId(cfg.spaceId);
        setSpaceName(cfg.spaceName);
        setTypeKey(cfg.typeKey);
        setBaseUrl(cfg.baseUrl);
        setSavedSpace({ id: cfg.spaceId, name: cfg.spaceName });
        setLoaded(true);
        // A saved space + key can list its types without a fresh test.
        if (cfg.hasKey && cfg.spaceId) void loadTypes();
      })
      .catch((e) => {
        if (cancelled) return;
        setError(String(e));
        setLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  function loadTypes() {
    // A failed fetch leaves the picker on its fallback options.
    ipc
      .anytypeListTypes()
      .then(setTypes)
      .catch(() => setTypes(null));
  }

  function edit(next: () => void) {
    next();
    setSaved(false);
    setError(null);
  }

  async function test() {
    setTesting(true);
    setTestError(null);
    try {
      const found = await ipc.anytypeCheck(apiKey.trim() || null, baseUrl.trim() || null);
      setSpaces(found);
      if (found.length === 1) {
        // One space: picking it is the only reasonable move.
        edit(() => {
          setSpaceId(found[0].id);
          setSpaceName(found[0].name);
        });
      }
    } catch (e) {
      setTestError(String(e));
    } finally {
      setTesting(false);
    }
  }

  async function save() {
    setSaving(true);
    setError(null);
    setSaved(false);
    setSpaceChanged(null);
    try {
      await ipc.anytypeConfigSet({
        // An empty field keeps the stored key; only a typed one replaces it.
        apiKey: apiKey.trim() === "" ? null : apiKey.trim(),
        spaceId: spaceId.trim(),
        spaceName: spaceName.trim(),
        typeKey: typeKey.trim(),
        baseUrl: baseUrl.trim(),
      });
      setApiKey("");
      setHasKey(true);
      setSaved(true);
      // The objects already pushed stay in the space they were pushed to;
      // the next sync of each note sends it to the newly chosen one.
      if (savedSpace && savedSpace.id !== "" && savedSpace.id !== spaceId.trim()) {
        setSpaceChanged(savedSpace.name);
      }
      setSavedSpace({ id: spaceId.trim(), name: spaceName.trim() });
      // The key and space just saved can list the space's types now.
      void loadTypes();
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }

  async function sendAll() {
    setError(null);
    try {
      const total = await ipc.anytypeSyncAll();
      if (total === 0) setError("There are no notes to sync.");
    } catch (e) {
      setError(String(e));
    }
  }

  const spaceOptions = (spaces ?? []).map((sp) => ({ value: sp.id, label: sp.name }));
  // The saved type may not be in the fetched list (an untested session, or a
  // type renamed since) — keep it selectable rather than silently dropping it.
  const typeOptions = (types ?? []).map((t) => ({ value: t.key, label: t.name }));
  if (typeKey && !typeOptions.some((t) => t.value === typeKey)) {
    typeOptions.unshift({ value: typeKey, label: typeKey });
  }
  if (!typeOptions.some((t) => t.value === "page")) {
    typeOptions.push({ value: "page", label: "Page (default)" });
  }

  if (!loaded) {
    return (
      <p className="py-3.5 text-sm text-[var(--color-text-muted)]" role="status">
        Loading Anytype…
      </p>
    );
  }

  return (
    <>
      <Row label="API key">
        <p className="text-xs text-[var(--color-text-muted)] mb-2">
          In Anytype: Settings → API Keys → Create New, named “Humla”. The key is kept in the
          macOS Keychain on this Mac and never synced. A saved key isn’t shown again; leave the
          field empty to keep it.
        </p>
        <input
          aria-label="Anytype API key"
          type="password"
          autoComplete="off"
          spellCheck={false}
          className={inputClass}
          placeholder={hasKey ? "Saved" : "Paste the key from Anytype"}
          value={apiKey}
          disabled={saving || testing}
          onChange={(e) => edit(() => setApiKey(e.target.value))}
        />
      </Row>
      <Row label="Space">
        <p className="text-xs text-[var(--color-text-muted)] mb-2">
          Test the connection to list your spaces, then pick one. Humla talks to Anytype on this
          Mac only.
        </p>
        <div className="flex items-center gap-2">
          <Select
            ariaLabel="Anytype space"
            className="min-w-0 flex-1"
            value={spaceId}
            onChange={(id) =>
              edit(() => {
                setSpaceId(id);
                setSpaceName(spaces?.find((sp) => sp.id === id)?.name ?? "");
              })
            }
            options={spaceOptions}
            placeholder={spaces === null ? "Test the connection to list spaces" : "Choose a space"}
            disabled={saving || testing || (spaces !== null && spaces.length === 0)}
          />
          <Btn disabled={saving || testing} onClick={() => void test()}>
            {testing ? "Testing…" : "Test connection"}
          </Btn>
        </div>
        {testError && (
          <p role="alert" className="text-sm text-[var(--color-danger)] mt-2">
            {testError}
          </p>
        )}
        {spaceName && spaces === null && (
          <p className="text-xs text-[var(--color-text-muted)] mt-2">
            Synced to “{spaceName}”.
          </p>
        )}
      </Row>
      <Row label="Object type">
        <p className="text-xs text-[var(--color-text-muted)] mb-2">
          What a synced note becomes in Anytype. Page is the safe default.
        </p>
        <Select
          ariaLabel="Anytype object type"
          className="min-w-0"
          value={typeKey || "page"}
          onChange={(key) => edit(() => setTypeKey(key))}
          options={typeOptions}
          disabled={saving || testing}
        />
      </Row>
      <Row label="API address">
        <p className="text-xs text-[var(--color-text-muted)] mb-2">
          Advanced. The local API runs inside the Anytype app; only this Mac’s address is accepted.
        </p>
        <input
          aria-label="Anytype API address"
          type="url"
          autoComplete="off"
          spellCheck={false}
          className={inputClass}
          placeholder={DEFAULT_BASE_URL}
          value={baseUrl}
          disabled={saving || testing}
          onChange={(e) => edit(() => setBaseUrl(e.target.value))}
        />
      </Row>
      <div className="py-3.5 space-y-3">
        <p className="text-xs text-[var(--color-text-muted)]">
          The API key is kept in the macOS Keychain on this Mac and never synced. Audio is never
          sent. Humla never reads anything back: an edit made in Anytype is replaced the next
          time the note syncs.{" "}
          <button
            type="button"
            onClick={() => void openExternal(DOCS_URL)}
            className="underline hover:text-[var(--color-text)]"
          >
            How Anytype sync works
          </button>
        </p>
        {error && (
          <p role="alert" className="text-sm text-[var(--color-danger)]">
            {error}
          </p>
        )}
        {spaceChanged && (
          <p role="status" className="text-sm text-[var(--color-text-muted)]">
            Notes already synced stay in “{spaceChanged}” — they’ll be sent to the new space
            the next time each is synced.
          </p>
        )}
        <div className="flex items-center gap-3">
          <Btn disabled={saving || testing} onClick={() => void save()}>
            {saving ? "Saving…" : "Save Anytype settings"}
          </Btn>
          {saved && (
            <span role="status" className="text-xs text-[var(--color-text-muted)]">
              Saved
            </span>
          )}
        </div>
      </div>
      <Row label="Send all notes">
        <p className="text-xs text-[var(--color-text-muted)] mb-2">
          Push every note to Anytype, one object each. Notes already up to date are skipped.
          Anytype limits how fast it accepts changes, so a large library takes a while.
        </p>
        {bulk ? (
          <div className="flex items-center gap-3">
            <p role="status" className="text-sm text-[var(--color-text-muted)]">
              Syncing {bulk.done} of {bulk.total}…
            </p>
            <Btn onClick={() => void ipc.anytypeSyncCancel()}>Stop</Btn>
          </div>
        ) : (
          <Btn disabled={saving || testing || !hasKey || !spaceId} onClick={() => void sendAll()}>
            Send all notes to Anytype
          </Btn>
        )}
      </Row>
    </>
  );
}
