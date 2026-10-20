import { useEffect, useState } from "react";
import { ipc } from "./ipc";
import type { SpeakerLabelStat, SpeakerSuggestionSource } from "./speakerSuggest";

/**
 * Fetch the rename picker's suggestion source (#116 part 1).
 *
 * One fetch when the strip mounts; all filtering and ranking happens in TS per
 * keystroke, never over IPC.
 *
 * Best-effort. Losing suggestions must never break the rename itself, which
 * still accepts free text.
 */
export function useSpeakerSuggestions(enabled: boolean): SpeakerSuggestionSource {
  const [stats, setStats] = useState<SpeakerLabelStat[]>([]);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    void ipc.speakerLabelStats().then(
      (rows) => alive && setStats(rows),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [enabled]);

  return { stats };
}
