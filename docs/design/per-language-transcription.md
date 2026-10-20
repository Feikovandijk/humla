# Per-language transcription model routing

**Status:** Historical design draft (Phase 4 scoping; the mechanism shipped, but NB Whisper was later retired).
**Author:** Claude (with Michael)
**Date:** 2026-05-07
**References:** `docs/design/stt-adapter.md`, Phase 1–3 implementation plans.

> **Editor's note (2026):** NB Whisper was removed and its Norwegian routing retired. The
> `transcribe_config` / per-language mechanism described here is unchanged; read every
> NB-specific example as an illustration of the mechanism, not a current model choice.


---

## Problem

Humla currently lets the user pick exactly one transcription provider + model globally (with a per-note override for one-off cases). That forces a single compromise across every language the user records in:

- **Norwegian** is much better served by NB Whisper Large (finetuned by Nasjonalbiblioteket on Norwegian) than by the multilingual Whisper Turbo.
- **English** is much better served by Deepgram Nova-3 than by OpenAI Whisper-1 (Nova-3 has stronger speaker turn handling, native diarization, and lower WER on conversational English).
- **Other languages** (Swedish, Danish, German, …) sit in the multilingual-Whisper sweet spot.

Right now a Norwegian + English bilingual user has to either (a) pick one model and accept worse output for the other language, or (b) override per-note every time, which is tedious and error-prone.

A second issue: the existing **"addon" mechanism** inside the local provider (NB Whisper auto-applies for Norwegian when downloaded) partially solves this — but only within the local provider, and the naming + UI hide what's happening. Users find "addon" misleading: it sounds like an extra component you stack on top of a main model, when really it's just an alternative model for one language.

## Goals

1. Let the user route different languages to different providers + models — e.g. Norwegian → local NB Whisper, English → Deepgram Nova-3, default → OpenAI whisper-1.
2. Replace the "addon" terminology with a clearer mental model. Models are just models; what makes one different is its language scope.
3. Preserve the auto-pick-the-right-model-for-the-language ergonomics for users who don't want to think about routing.
4. Stay backwards-compatible: existing `transcribe_config` settings keep working unchanged after upgrade.

## Non-goals

- **No** per-folder or per-tag routing. Only per-language. (Adding more axes is easy later if needed.)
- **No** in-recording language switching. The recording's language is fixed at start time (per-note language override or global default); we resolve to one config and use it for the whole session.
- **No** automatic language detection driving routing. If the user picks "auto" for a recording's language, we use the default config — same as today.
- **No** per-language *system prompt* override (that's `summary_prompt`'s domain, not transcription).

---

## The naming refactor

The current code has:

```rust
pub enum ModelKind {
    Primary,
    LanguageAddon { language: &'static str },
}
```

This made sense when local was the only provider with multiple model files, but it doesn't translate to the cross-provider world. Renaming proposals:

| Today | Better name | Reason |
|---|---|---|
| `Primary` | `Multilingual` | Describes *what it does*, not its UI position. |
| `LanguageAddon { language }` | `LanguageSpecific { language }` | Drops the misleading "addon" stacking implication. |
| UI badge "NO auto" | UI badge "Norwegian" | The activation behaviour is implicit; tagging just declares scope. |

In Settings, a downloaded NB Whisper row would read:

> **NB Whisper Large** — Norwegian
> Norwegian-finetuned by Nasjonalbiblioteket. Auto-applies when a recording's language is Norwegian.

Versus today's:

> **NB Whisper Large (Norwegian add-on)** [NO auto]
> Norwegian-finetuned by Nasjonalbiblioteket. Auto-used for Norwegian recordings when downloaded; English/other-language meetings keep using your active primary model.

The new copy makes the language scope a property of the model rather than a separate "addon" concept the user has to model in their head.

---

## Schema design

### Today

```jsonc
// settings.transcribe_config
{ "provider": "deepgram", "model": "nova-3" }
```

### Proposed

```jsonc
// settings.transcribe_config
{
  "default": { "provider": "openai", "model": "whisper-1" },
  "per_language": {
    "no": { "provider": "local", "model_id": "nb-whisper-large-q5", "preset": "quality", "use_gpu": true },
    "en": { "provider": "deepgram", "model": "nova-3" }
  }
}
```

Rust shape:

```rust
#[derive(Serialize, Deserialize)]
pub struct TranscribeConfig {
    pub default: ProviderConfig,
    #[serde(default)]
    pub per_language: BTreeMap<String, ProviderConfig>,
}
```

- `BTreeMap` (not `HashMap`) so the JSON key order is stable across writes — easier diff-and-eyeball when debugging.
- `String` keys are ISO 639-1 codes (`"no"`, `"en"`, `"de"`, …). The existing per-note `language` field uses the same convention, so resolution is a direct lookup.
- `default` is required; `per_language` is optional and empty-by-default.

### Migration from today's shape

The existing `transcribe_config` row is a bare `ProviderConfig`. Migration is a one-shot wrap:

```rust
// Pseudocode for migrate_transcribe_config_v4
let raw = get_setting(conn, "transcribe_config")?;
if let Some(json) = raw {
    if let Ok(_typed) = serde_json::from_str::<TranscribeConfig>(&json) {
        // Already in the new shape (re-running migration). Done.
    } else if let Ok(legacy) = serde_json::from_str::<ProviderConfig>(&json) {
        // Wrap it into the new shape — preserves the user's existing choice
        // as the new `default`, with no per-language overrides.
        let migrated = TranscribeConfig { default: legacy, per_language: BTreeMap::new() };
        set_setting(conn, "transcribe_config", &serde_json::to_string(&migrated)?)?;
    }
}
```

Idempotent via the typed-vs-legacy parse check. No new flag row needed.

### Resolution order at recording time

Given a chunk's effective language `lang`, the dispatcher picks the config in this order:

1. **Per-language override** — if `cfg.per_language.contains_key(lang)`, use that.
2. **Default** — otherwise `cfg.default`.

The existing per-note `language` field continues to work the same way it does today: it determines which language the resolution uses. A user who sets a per-note language to "en" will hit the English override; one who leaves it blank falls back to the global language, which then either matches a per-language entry or falls back to default.

Pseudocode:

```rust
fn resolve_provider_config(cfg: &TranscribeConfig, language: &str) -> &ProviderConfig {
    cfg.per_language
        .get(language)
        .unwrap_or(&cfg.default)
}
```

The current `addon_for_language` lookup (which auto-swaps NB Whisper for Norwegian inside the local provider) **goes away** — it's subsumed by the per-language map.

---

## UI sketch

### Settings → Transcription tab

```
┌─ Default provider ────────────────────────────────────────┐
│ Source:  ⦿ OpenAI    ○ Deepgram    ○ Groq    ○ Local      │
│ Model:   [whisper-1 ▾]                                    │
└───────────────────────────────────────────────────────────┘

┌─ Per-language overrides ──────────────────────────────────┐
│                                                           │
│ Override for specific languages — useful when one model   │
│ is materially better for a given language.                │
│                                                           │
│ ┌─────────────────────────────────────────────────────┐   │
│ │ Norwegian (no)                                  [×] │   │
│ │   Local · NB Whisper Large · Quality · GPU          │   │
│ │   [Edit]                                            │   │
│ └─────────────────────────────────────────────────────┘   │
│ ┌─────────────────────────────────────────────────────┐   │
│ │ English (en)                                    [×] │   │
│ │   Deepgram · nova-3                                 │   │
│ │   [Edit]                                            │   │
│ └─────────────────────────────────────────────────────┘   │
│                                                           │
│ [+ Add language override]                                 │
└───────────────────────────────────────────────────────────┘

┌─ Local model behaviour (default + any local overrides) ───┐
│ Quality preset · GPU acceleration                         │
│   (only shown when at least one Local config is in use)   │
└───────────────────────────────────────────────────────────┘

┌─ Local models ────────────────────────────────────────────┐
│ ○ Whisper Large v3 Turbo · Multilingual    [downloaded]   │
│ ⦿ NB Whisper Large · Norwegian             [downloaded]   │
│ ○ Whisper Medium · Multilingual            [download]     │
└───────────────────────────────────────────────────────────┘
```

### "Add language override" flow

Clicking `[+ Add language override]` opens an inline form:

```
Language: [Norwegian (no) ▾]
Provider: [Local ▾]
Model:    [NB Whisper Large ▾]
Preset:   [Quality ▾]
GPU:      [✓]
[Cancel]  [Add]
```

The Language picker shows ISO codes the user might pick — same list as the global Language setting. No free-form input.

### Local-models list

The third section above replaces today's mixed Primary/Addon list with a flat list where each model declares its language scope:

- "Multilingual" tag → the model handles any language.
- "Norwegian" / "English" / etc. tag → the model is specialised for that language.

The radio button (active *local* model for the default config) only applies to multilingual models — language-specific models are never "the default"; they're picked via per-language overrides. Downloading a language-specific model doesn't auto-add an override anymore (loses the auto-magic but gains transparency); we suggest it via a flash hint:

> ✓ NB Whisper Large downloaded.
> [Add as Norwegian override?]

One click adds the entry. Power users get clear control; newcomers get the same end-state with one extra click on first download.

### What about the per-note override?

The current per-note language picker stays. Routing logic doesn't change for it: changing the note's language causes resolution to lookup that language in the per-language map. The Note view doesn't need any UI change — the model selection is invisible to the per-note flow.

If we *also* wanted per-note provider override (deferred from Phase 2's open questions), that's an orthogonal feature — would slot in nicely on top of this schema (a note's per-provider override would just take precedence over both the per-language map and the default), but Phase 4 doesn't require it.

---

## Edge cases

1. **User sets a per-language override that needs an API key they don't have stored.**
   E.g. they pick Deepgram for English but never saved a Deepgram key. The recording's prereq check (`recording_start`) needs to look up the *resolved* config's key, not just the default. Today's check already routes through `read_provider_api_key(provider_id)`; we just need to feed it the resolved provider id.

2. **User sets a per-language override pointing at a local model that isn't downloaded.**
   Same prereq path: resolve the config first, then check the file. Error message becomes:
   > "Local model 'nb-whisper-large-q5' is configured for Norwegian recordings but not downloaded. Download it in Settings → Local models."

3. **`language == "auto"` on a note where the user wants automatic detection.**
   `auto` doesn't match any per-language entry; we use `default`. (This matches today's `addon_for_language` behaviour, which returns None for "auto".)

4. **User downgrades to v0.23.x with v0.24 typed config on disk.**
   Old `read_provider_config` would fail to parse `{ default, per_language }` as `ProviderConfig`. It already has the corrupt-JSON fallback path (returns default OpenAI/whisper-1), so the user wouldn't be locked out, but they'd lose their settings until they upgrade back. Acceptable tradeoff.

5. **User has v0.21 → upgrades straight to v0.24** (skipping v0.22 and v0.23).
   v0.23's `migrate_transcribe_config` builds a `ProviderConfig` from legacy keys; v0.24's migration then wraps that into `{ default: …, per_language: {} }`. Both migrations run in order on first launch — no work for the user.

6. **Per-language map drift.**
   What if the user has a Norwegian override pointing at Deepgram and then deletes their Deepgram key? The override stays; the next Norwegian recording's prereq fails with the "API key not set for Deepgram" error message. Correct behaviour — we don't silently delete user-set config.

---

## Open design questions

These need user input before the implementation plan locks down. My proposals are listed under each, but they're directional, not final.

### Q1. Auto-apply behaviour for downloaded language-specific models

**Today:** Downloading NB Whisper auto-applies it for Norwegian recordings, no explicit user action needed.

**Proposal:** Drop auto-apply. After download, surface a "Add as Norwegian override?" suggestion that adds the override with one click.

**Tradeoff:** Auto-apply is delightful when it works ("just download and go"). Explicit override is more transparent and lets the user opt out (e.g. wants to compare NB Whisper vs Deepgram on Norwegian audio). Auto-apply also breaks down once we have multiple Norwegian options (NB Whisper local vs. some cloud Norwegian-tuned model) — the explicit picker scales naturally.

**Recommendation:** Drop auto-apply. The one-click suggestion preserves most of the convenience.

### Q2. Default config when no overrides exist — same UX as today, or different?

**Today's UX:** A single provider+model selector at the top of Settings.

**Proposal:** Keep that exact UX as the "Default provider" section. The "Per-language overrides" section is empty by default and only takes screen space when the user adds entries.

**Recommendation:** Keep today's UX as the default. No regression for users who never want per-language routing.

### Q3. Drop `addon_for_language` entirely, or keep as fallback?

**Option A:** Drop it. Migration adds an explicit per-language override for any installed addon (e.g. NB Whisper downloaded → migration adds `{ "no": local/nb-whisper-large-q5 }`). After migration, no implicit routing remains; everything is in `transcribe_config`.

**Option B:** Keep `addon_for_language` as a fallback layer that runs *after* `per_language` lookup but *before* `default`. So resolution becomes: per-language override → addon match (local-only) → default.

**Tradeoff:** A is cleaner (one resolution rule); B is more forgiving (a user who has NB Whisper but no per-language override still gets it auto-applied). B also means downloaded language models keep "just working" without UI interaction, preserving the v0.23 behaviour.

**Recommendation:** A. Pair it with Q1's one-click suggestion to keep the migration path discoverable. Single resolution rule is much easier to reason about and document.

### Q4. Provider scope for per-language overrides

**Proposal:** Same four providers as the default — openai, local, deepgram, groq. Anywhere the default works, the override works.

**Recommendation:** Yes — keep symmetric. No reason to constrain.

### Q5. Multiple-language entries pointing at the same config

**Today:** Not relevant.

**Proposal:** A user who wants both Norwegian and Swedish to use NB Whisper just adds two entries. We don't add range/group support.

**Recommendation:** Yes — KISS. If someone has many similar overrides, that's a UX problem to solve later (maybe a "duplicate this entry" button).

### Q6. Where does the per-language override sit relative to the per-note language override?

**Today:** Per-note `language` field overrides the global `language` setting. Both feed into resolution.

**Proposal:** No change to that. Per-note language picks the resolution input; per-language overrides pick what config that input resolves to.

**Recommendation:** Yes. They're complementary, not competing.

### Q7. What happens when a user *removes* a language they previously set as their global default?

E.g. global language was "no", they had `per_language: { "no": local/nb-whisper-large-q5 }`. Now they switch global language to "en" with no English override. The Norwegian override sits dormant. Does the UI flag it as unused?

**Proposal:** No flag. Per-language entries are independent of the current global language; a future Norwegian recording (per-note override) will still use them.

**Recommendation:** Yes. Don't infer disuse from a single setting.

---

## Phase 4 implementation outline

Once design is agreed, the impl plan would split into roughly these tasks:

1. **Schema + serde** — `TranscribeConfig` struct in `stt::config`, round-trip tests, the `ProviderConfig` → `TranscribeConfig` wrap migration.
2. **`local_whisper` rename** — `Primary`/`LanguageAddon` → `Multilingual`/`LanguageSpecific`. Drop `addon_for_language`. Update copy on the NB Whisper entry to drop "add-on" wording.
3. **Resolver + commands** — `resolve_provider_config(&cfg, language: &str)`. `read_provider_config(state, language)` returns the resolved config. `recording_start` prereq + `transcribe_chunk` use the resolved config (callers pass the recording's language).
4. **Migration v4** — startup wraps a bare `ProviderConfig` into `TranscribeConfig`. If a `LanguageSpecific` model is currently downloaded, surface a one-time "Add as Norwegian override?" suggestion in Settings (a settings flag `migrated_per_language_v4_suggested`).
5. **`get_provider_config` / `set_provider_config` Tauri commands** — return / accept the new `TranscribeConfig` shape. Frontend `ProviderConfig` type is renamed/wrapped in the new shape.
6. **Settings UI** — Default provider section (unchanged), Per-language overrides section (list + add/remove form), refactored Local models list with language tags.
7. **Frontend resolution** — `useSettings` exposes `transcribeConfig` (full shape) plus a memoised `defaultConfig` getter. `Transcription.tsx` reads/writes via the new shape.
8. **Smoke tests + v0.24.0 release.**

Estimated diff: ~600 lines added, ~150 removed. Estimated focused work: 8–10 hours.

---

## What I'd defer past Phase 4

- **Per-note provider override** (Phase 2 open question §1). The new schema makes this trivial — note gets a `provider_override: Option<ProviderConfig>` column — but it's a separate feature.
- **Auto-detect language → route** combo. Would require running a quick language ID pass before transcription, which adds latency and complexity. Worth it only if user data shows lots of mixed-language recordings.
- **Routing by other axes** (folder, tag, custom rule). Add only if real friction surfaces.
- **Quota / cost-aware routing** ("use Groq when over X minutes/day"). Premature without billing visibility.
