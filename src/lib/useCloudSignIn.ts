import { useState } from "react";
import { cloudApi, cloudError, useCloudStore, type SecondFactor } from "./cloud";

/**
 * Signing in to the cloud, for every surface that does it (Settings → Account,
 * onboarding, the new-workspace sheet). An account with two-factor
 * authentication answers the password with a `code` step, which the
 * surface renders as `TwoFactorCodeForm`. Each action resolves true once the
 * user is signed in and the cloud store has caught up.
 */
export function useCloudSignIn() {
  const refresh = useCloudStore((s) => s.refresh);
  const [step, setStep] = useState<"password" | "code">("password");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<boolean>): Promise<boolean> {
    setBusy(true);
    setError(null);
    try {
      return await action();
    } finally {
      setBusy(false);
    }
  }

  const signIn = (email: string, password: string) =>
    run(async () => {
      try {
        const result = await cloudApi.login(email, password);
        if (result.step === "two_factor") {
          setStep("code");
          return false;
        }
      } catch (e) {
        setError(cloudError(e).message);
        return false;
      }
      await refresh();
      return true;
    });

  const signUp = (email: string, password: string, name: string) =>
    run(async () => {
      try {
        await cloudApi.signup(email, password, name);
      } catch (e) {
        setError(cloudError(e).message);
        return false;
      }
      await refresh();
      return true;
    });

  const verify = (factor: SecondFactor) =>
    run(async () => {
      try {
        await cloudApi.verifyTwoFactor(factor);
      } catch (e) {
        const failure = cloudError(e);
        // The sign-in it answered is gone; only the password starts another.
        if (failure.reason === "challenge_expired") setStep("password");
        setError(failure.message);
        return false;
      }
      setStep("password");
      await refresh();
      return true;
    });

  function back() {
    setStep("password");
    setError(null);
  }

  return { step, busy, error, setError, signIn, signUp, verify, back };
}
