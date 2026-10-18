import { useState } from "react";
import type { SecondFactor } from "../lib/cloud";

const DEFAULT_INPUT_CLS =
  "w-full text-sm px-3 py-2 rounded-md border border-[var(--color-line-visible)] bg-[var(--color-surface)] focus:border-[var(--color-text-muted)] transition-colors";

/** Six digits, however the app grouped them. */
export function isAuthenticatorCode(value: string): boolean {
  return /^\d{6}$/.test(value.replace(/\s/g, ""));
}

/**
 * The second step of signing in to an account with two-factor authentication:
 * the code from the authenticator app, or one of the recovery codes
 * for someone without their phone. Also the confirmation step wherever
 * Settings asks for a second factor.
 */
export function TwoFactorCodeForm({
  busy,
  error,
  onSubmit,
  onBack,
  submitLabel = "Verify",
  inputClassName = DEFAULT_INPUT_CLS,
  allowRecovery = true,
  disabled = false,
  autoFocus = true,
}: {
  busy: boolean;
  error: string | null;
  onSubmit: (factor: SecondFactor) => void;
  /** Omitted where there is no step to go back to. */
  onBack?: () => void;
  submitLabel?: string;
  inputClassName?: string;
  /** False where only the app's own code will do: confirming a new app. */
  allowRecovery?: boolean;
  /** Holds the submit back while something else the step needs is missing. */
  disabled?: boolean;
  autoFocus?: boolean;
}) {
  const [recovery, setRecovery] = useState(false);
  const [value, setValue] = useState("");

  const ready =
    !disabled && (recovery ? value.trim().length > 0 : isAuthenticatorCode(value));

  function submit() {
    if (!ready || busy) return;
    onSubmit(recovery ? { recoveryCode: value.trim() } : { code: value.replace(/\s/g, "") });
  }

  function toggle() {
    setRecovery(!recovery);
    setValue("");
  }

  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm text-[var(--color-text-muted)]">
        {recovery
          ? "Enter one of the recovery codes you saved when you set up two-factor authentication."
          : "Enter the 6-digit code from your authenticator app."}
      </p>
      <input
        key={recovery ? "recovery" : "code"}
        autoFocus={autoFocus}
        className={inputClassName}
        aria-label={recovery ? "Recovery code" : "Authentication code"}
        inputMode={recovery ? "text" : "numeric"}
        autoComplete={recovery ? "off" : "one-time-code"}
        spellCheck={false}
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && submit()}
        placeholder={recovery ? "XXXXX-XXXXX" : "123456"}
      />
      {error && <p className="text-xs text-[var(--color-danger)]">{error}</p>}
      <div className="flex flex-wrap items-center gap-3">
        <button
          type="button"
          onClick={submit}
          disabled={busy || !ready}
          className="nd-btn nd-btn-primary"
        >
          {busy ? "Verifying…" : submitLabel}
        </button>
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            disabled={busy}
            className="nd-btn"
          >
            Back
          </button>
        )}
        {allowRecovery && (
          <button
            type="button"
            onClick={toggle}
            className="text-xs text-[var(--color-interactive)] hover:underline"
          >
            {recovery ? "Use your authenticator app instead" : "Use a recovery code instead"}
          </button>
        )}
      </div>
    </div>
  );
}
