# Humla — agent guide

Humla is a personal macOS meeting-notes app (Tauri 2 + React 19 + Rust + two Swift sidecars). It records mic and system audio as two separate streams, transcribes them, diarizes after stop, and summarizes your typed notes together with the transcript. Data stays local in SQLite and secrets go in the Keychain. There is no backend.

This file is the map. **`CLAUDE.md` is the full reference**: the reasoning behind every rule below, the data flow during a recording, the sidecars, signing and releases. Read the section of it that covers the area you are changing before you change it.

## Before you start

- `CONTEXT.md`: the domain glossary (Note, Title, Folder, Client, Take…). Name concepts the way it names them.
- `docs/adr/`: six decisions. Read the ones that touch your area, and say so if your change contradicts one.
  - 0001 Client is a distinct entity from Folder
  - 0002 No Person entity: speakers are derived
  - 0003 gpt-transcribe not adopted
  - 0004 The timeline is canonical for a note's content
  - 0005 Nemotron 3 is the default diarization engine
  - 0006 Parakeet Ultra is a second on-device ASR engine; NB Whisper retired
- `docs/agents/`: the issue tracker (GitHub `michaelwilhelmsen/humla` through `gh`), the triage labels and the domain-docs conventions.
- `docs/research/`: measurements that back current thresholds (echo, STT, embeddings, MCP).

## Repo index

```
src/                      React frontend
  App.tsx, main.tsx       routes (/, /all-notes, /note/:id, /chat, /folder/:id[/chat], /trash, /onboarding)
  pages/                  Note.tsx (the main view), AllNotes, Folder, Chat, Home, Trash,
                          ExportModal, SendToWebhookModal, TranscriptEditor
    settings/             SettingsDialog + tabs/ (General, Recording, Transcription,
                          Summary, Chat, Integrations, Webhook, Account, TwoFactor, …)
    onboarding/           first-run steps
  components/             Layout, Sidebar, RecordingBar, CaptureIndicator, NoteCard,
                          NoteFilters, ChatPanel, SpeakerLabels, ImportDialog, …
    ui/                   Popover / Menu / Select / Combobox: Radix wrappers (use these)
    provider/             provider key / model download / local-LLM readiness cards
  lib/                    ipc.ts (every invoke), store.ts (Zustand + backend listeners),
                          plus the TS halves of the mirrored rules (see below)
  styles/                 globals.css + themes/{warm,graphite}.css (token contract)
  content/releases.ts     in-app changelog; must have an entry per release
  test/                   vitest setup, Tauri IPC mock, fixtures
src-tauri/                Rust backend (crate `humla`, Rust 1.85)
  src/lib.rs              AppState, command registration, startup migrations
  src/commands.rs         recording lifecycle, transcribe_chunk, diarize-on-stop,
                          replay/transcribe_note, timelines
  src/commands/           other #[tauri::command] groups (notes, folders, settings, summary,
                          title, chat, export, mcp, webhook, models, …)
  src/stt/                BatchSttAdapter + OpenAI / Local / Deepgram / Groq
  src/chat/               agentic chat over notes (tools, providers)
  src/mcp/ + bin/humla-mcp.rs   read-only MCP server binary
  src/db.rs               schema, CRUD, migrations
  src/recording.rs        LiveCapture, CaptureSink, Phase, Step
  src/diarize.rs, echo.rs diarize sidecar wrapper + engine choice; echo removal
  src/sessions.rs         takes/manifest, keep_audio + transcribe-manually rules
  src/openai.rs, anthropic.rs, providers.rs, keychain.rs, local_whisper.rs, …
  crates/echo-probe/      echo lag/cancel library + measurement CLI
audio-capture/            Swift sidecar: AVAudioEngine mic + ScreenCaptureKit system audio
speaker-diarize/          Swift sidecar: FluidAudio diarization (pinned 0.17.4), `swift test`
scripts/                  build-*.sh (sidecars, mcp, metallib, dmg), release.sh, dev-sign.sh,
                          measure-toolbar.js / measure-recording-bar.js (run in pnpm mock)
```

## Commands

| Task | Command |
|---|---|
| Frontend tests | `pnpm test` (vitest) |
| Type-check + build | `pnpm build` |
| Lint (catches hook-order bugs; expect 0 errors, about 5 warnings) | `pnpm lint` |
| Rust tests | `cd src-tauri && cargo test --workspace` |
| Diarize sidecar logic | `cd speaker-diarize && swift test` |
| Visual check without Tauri | `pnpm mock` → `mock.html?case=<name>&palette=warm\|graphite&theme=light\|dark` |
| Run the app | `pnpm tauri dev` (needs the sidecars built first) |
| Build sidecars | `./scripts/build-sidecar.sh`, `./scripts/build-diarize.sh`, `./scripts/build-mcp.sh` |
| Ship | `pnpm release` (do not run it unless asked) |

On a fresh clone `src-tauri/binaries/` doesn't exist. `tauri-build` fails when an `externalBin` (`audio-capture`, `speaker-diarize`, `humla-mcp`) is missing, so run the build scripts before any cargo build. Building and signing need a Mac. Without a Developer ID identity the binaries still build, unsigned.

## Git and pull requests

- This checkout is the fork `Feikovandijk/humla` (`origin`). **Open every PR against the fork's `dev`** (`gh pr create --repo Feikovandijk/humla --base dev`). `main` is for releases only. Never open a PR against the upstream `michaelwilhelmsen/humla`.
- Branch off `dev`; don't commit to `dev` or `main` directly.
- Never put sensitive data in a PR (title, body, commits or diff): no API keys, tokens, credentials, signing or notarisation details, personal data, or meeting content.

## Rules that are easy to break

- **`keep_audio` is the single, absolute gate on audio.** When it is off, no WAV is written, uploaded or downloaded. Don't add exceptions. MCP and the webhook never return or reference audio.
- **Mirrored pairs: change both sides.** Most are pinned by a test that reads the other file.
  - `is_replaceable_title` ↔ `isReplaceableTitle` (`noteSync.ts`)
  - diarize engine consts ↔ `src/lib/diarizeEngine.ts`
  - `providers.rs` REGISTRY ↔ `src/lib/providers.ts`
  - `TranscribeScope` ↔ the `ipc.transcribeNote` union
  - `split_label` ↔ `parseTranscriptLines`
  - `resumes_on_open` ↔ `targetResumesOnOpen`
  - `pinned_breadth` ↔ `targetPinsScope`
  - `clamp_device_name` ↔ `noAudioWarning`
  - `Engine::label` ↔ `DIARIZE_ENGINE_LABEL`
- **The timeline is canonical (ADR-0004).** On a note that has a timeline, `note.transcript` is a projection of it. Edit through `note_timeline_*` and never write the transcript string directly.
- **Only the live capture's post-stop chain writes `recording_status`.** Other work reports through `ChainProgress` (`Rediarize` / `Replay` / `Silent`).
- **Derived local columns never bump `updated_at`** (`speakers`, `detected_language`). `updated_at` drives the activity-sorted lists.
- **Async/Tauri pitfalls:**
  - Never hold a `parking_lot` guard across `.await`.
  - Use `tauri::async_runtime::spawn`, not `tokio::spawn`, from `setup`.
  - Don't call `app.state::<AppState>()` while the sync observer is being built.
- **Secrets go through `crate::keychain`.** Don't call `keyring::Entry` directly. Device names (`input_device`) are display-only and never persisted.
- **UI:**
  - No hand-rolled popovers. Use `components/ui/`.
  - Colours, metrics and type come from theme tokens, and a new token must be declared in every theme (`themes.contract.test.ts`).
  - Gold accent text uses `--color-accent-text`.
  - `window.prompt` / `confirm` / `alert` are blocked in the webview.
- **Any code that writes a note's files ends with `purge_note_assets_if_gone`.**
- **Bump `CHUNKER_VERSION` when chunk boundaries change.**
- **Releases:**
  - A merge to `main` releases: CI bumps the version (`release:minor` / `release:major` PR label, else patch), tags it and publishes. Don't bump versions or push tags by hand.
  - The `dev → main` PR must carry exactly one `src/content/releases.ts` entry with `version: "next"`; CI stamps the real version and date into it.
