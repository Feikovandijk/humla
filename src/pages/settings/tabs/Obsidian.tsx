import { open } from "@tauri-apps/plugin-dialog";
import { Btn } from "../components/Btn";
import { Row } from "../components/Section";
import { Toggle } from "../components/Toggle";
import type { EditableKey } from "../types";

// Export to Obsidian: picking a vault folder is what turns it on — the note's
// ⋯ menu offers "Export to Obsidian" only once a folder is set.
export function ObsidianRows({
  s,
  update,
}: {
  s: Record<EditableKey, string>;
  update: (key: EditableKey, value: string) => void;
}) {
  const vault = s.obsidian_vault_path;
  async function choose() {
    const picked = await open({ directory: true, defaultPath: vault || undefined });
    if (typeof picked === "string") update("obsidian_vault_path", picked);
  }
  return (
    <>
      <Row
        label="Obsidian vault folder"
        description={
          vault
            ? `Export to Obsidian writes <Title>.md into ${vault}, replacing the file when you export the same note again.`
            : "Choose a folder in your vault to add Export to Obsidian to each note’s ⋯ menu."
        }
        control={
          <div className="flex gap-2">
            <Btn onClick={choose}>{vault ? "Change…" : "Choose…"}</Btn>
            {vault && <Btn onClick={() => update("obsidian_vault_path", "")}>Clear</Btn>}
          </div>
        }
      />
      {vault && (
        <Row
          label="Include transcript"
          description="Summary on top, transcript below. Off exports the summary only."
          control={
            <Toggle
              checked={s.obsidian_include_transcript !== "false"}
              onChange={(next) => update("obsidian_include_transcript", next ? "true" : "false")}
              label="Include transcript in Obsidian export"
            />
          }
        />
      )}
    </>
  );
}
