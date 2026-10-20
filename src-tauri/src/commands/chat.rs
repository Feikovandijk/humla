//! Chat commands (issue #46). `chat_send` runs a single-pass, Note-grounded
//! completion: it resolves the configured chat provider, grounds the prompt in
//! the current Note's content (as reference material, never the system prompt),
//! streams the answer to the frontend, and persists the turn. `chat_history`
//! reloads a Note's conversation after restart. The heavy lifting (prompt
//! assembly, budget, streaming orchestration) lives Tauri-free in `crate::chat`.

use super::{DEFAULT_LOCAL_LLM_BASE_URL, DEFAULT_SUMMARY_MODEL};
use crate::chat::{self, ChatCtx, ChatEvent, Citation, ToolScope};
use crate::db::{self, ChatTarget, CHAT_TENANT_PERSONAL};
use crate::embed::{self, EmbeddingAdapter, OLLAMA_EMBED_MODEL, OPENAI_EMBED_MODEL};
use crate::openai;
use crate::AppState;

/// A user-set display name for chat's first-person referent (#103). Empty/unset
/// falls back to the macOS account name — see [`asker_name`].
pub(crate) const SETTING_DISPLAY_NAME: &str = "user_display_name";
use parking_lot::Mutex;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::providers::ProviderId;
use tauri::{AppHandle, Emitter, Manager, State};

/// Derive the embedding config from the resolved chat provider (issue #48).
/// Cloud chat embeds with `text-embedding-3-small` and offers no choice; local
/// chat defaults to `embeddinggemma` on the chat server, and overrides both
/// halves (#179), because only Ollama serves the embedder beside the chat
/// model: mlx_lm.server has no `/v1/embeddings` route, llama-server needs its
/// own process, and LM Studio names the model `text-embedding-embeddinggemma-…`.
/// `None` where the chat provider has no embeddings endpoint at all
/// (Anthropic): retrieval then runs keyword-only rather than spending the
/// Anthropic key against an OpenAI route it can't authenticate.
fn resolve_embed(resolved: &ResolvedChat) -> Option<embed::EmbedConfig> {
    match resolved.provider {
        ProviderId::Local => Some(embed::EmbedConfig {
            provider: "ollama",
            model: resolved.embed.model.clone().unwrap_or_else(|| OLLAMA_EMBED_MODEL.to_string()),
            base_url: resolved.embed.base_url.clone().unwrap_or_else(|| resolved.base_url.clone()),
            api_key: None,
        }),
        ProviderId::Anthropic => None,
        _ => Some(embed::EmbedConfig {
            provider: "openai",
            model: OPENAI_EMBED_MODEL.to_string(),
            base_url: resolved.base_url.clone(),
            api_key: resolved.api_key.clone(),
        }),
    }
}

/// The Keychain key for whichever provider chat is set to. Reading every slot
/// would prompt for providers this turn never calls, so the setting is read
/// first and only the matching slot is opened.
fn chat_api_key(state: &State<AppState>) -> Option<String> {
    let stored = {
        let conn = state.db.lock();
        db::get_setting(&conn, "chat_provider")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "local".into())
    };
    match ProviderId::parse(&stored) {
        Some(id) if id.keychain_account().is_some() => {
            super::read_provider_api_key(state, id.as_str()).ok().flatten()
        }
        _ => None,
    }
}

/// Embed a Note's not-yet-embedded chunks under the adapter's model and cache
/// them (issue #48). Content-addressed, so only changed/new text embeds. Best-
/// effort: any failure (model missing, API error) logs and returns — chat still
/// works keyword-only. The DB lock is never held across the embed `.await`.
pub(crate) async fn embed_note(
    db: &Arc<Mutex<Connection>>,
    adapter: &dyn EmbeddingAdapter,
    note_id: &str,
) {
    let need = {
        let conn = db.lock();
        db::note_texts_needing_embedding(&conn, note_id, adapter.model_id()).unwrap_or_default()
    };
    if need.is_empty() {
        return;
    }
    let texts: Vec<String> = need.iter().map(|(_, t)| t.clone()).collect();
    match adapter.embed(&texts).await {
        Ok(vectors) if vectors.len() == need.len() => {
            let conn = db.lock();
            for ((hash, _), vector) in need.iter().zip(vectors.iter()) {
                let _ = db::store_embedding(&conn, hash, adapter.model_id(), vector);
            }
        }
        Ok(vectors) => eprintln!(
            "[chat] embed count mismatch for note {note_id}: {} vectors for {} chunks",
            vectors.len(),
            need.len()
        ),
        Err(e) => eprintln!("[chat] embed_note {note_id} failed (keyword-only): {e}"),
    }
}

/// Resolve the embedding provider and embed a Note off the request path
/// (content-settled checkpoint). Spawned fire-and-forget after a re-chunk so
/// editing never blocks on embedding. No-op if no provider is configured.
pub async fn embed_note_bg(app: AppHandle, note_id: String) {
    let state: State<AppState> = app.state();
    let key = chat_api_key(&state);
    let resolved = {
        let conn = state.db.lock();
        resolve_chat(&conn, key)
    };
    let Ok(resolved) = resolved else { return };
    let Some(cfg) = resolve_embed(&resolved) else { return };
    let adapter = cfg.adapter();
    embed_note(&state.db, &adapter, &note_id).await;
}

/// Rebuild one Note's retrieval chunks from its current content. The single
/// place body-HTML→text + `db::reindex_note` are combined, called from every
/// content-settled checkpoint (after summarize, after diarization, on
/// Note-view unmount) and the startup backfill. Best-effort: a failure to
/// index never blocks the user-facing action.
pub(crate) fn reindex_note_content(conn: &rusqlite::Connection, note_id: &str) {
    if let Ok(note) = db::get_note(conn, note_id) {
        let body_text = crate::html_text::html_to_text(&note.body);
        if let Err(e) = db::reindex_note(conn, note_id, &body_text, &note.transcript, &note.summary) {
            eprintln!("[chat] reindex of note {note_id} failed: {e}");
        }
    }
}

/// One-time backfill: index every live Note that has no chunks yet, so notes
/// created before #47 become searchable. Cheap and idempotent — reruns index
/// only the still-missing ones. Runs off-thread at startup.
pub fn backfill_note_chunks(db: &std::sync::Arc<parking_lot::Mutex<rusqlite::Connection>>) {
    let ids = {
        let conn = db.lock();
        db::note_ids_needing_reindex(&conn).unwrap_or_default()
    };
    if ids.is_empty() {
        return;
    }
    eprintln!("[chat] backfilling retrieval chunks for {} note(s)", ids.len());
    for id in ids {
        let conn = db.lock();
        reindex_note_content(&conn, &id);
    }
}

/// One-time embedding backfill (issue #48): after chunks exist, embed every
/// Note that still lacks vectors under the current model, so semantic search
/// covers the whole corpus on day one — not only notes touched since #48.
/// Runs off the request path at startup, best-effort (a missing model / API
/// error just leaves those notes keyword-only). Batched per Note + cached, so
/// it's idempotent and cheap on reruns.
pub async fn embed_backfill(app: AppHandle) {
    let state: State<AppState> = app.state();
    let key = chat_api_key(&state);
    let resolved = {
        let conn = state.db.lock();
        resolve_chat(&conn, key)
    };
    let Ok(resolved) = resolved else { return };
    let Some(cfg) = resolve_embed(&resolved) else { return };
    let adapter = cfg.adapter();
    let ids = {
        let conn = state.db.lock();
        db::note_ids_needing_embedding(&conn, adapter.model_id()).unwrap_or_default()
    };
    if ids.is_empty() {
        return;
    }
    eprintln!("[chat] embedding backfill: {} note(s) under {}", ids.len(), adapter.model_id());
    for id in ids {
        embed_note(&state.db, &adapter, &id).await;
    }
    eprintln!("[chat] embedding backfill complete");
}

/// How many live notes hold chunks with no vector under the embedder now in force.
///
/// The embedding index is keyed `(text_hash, model)` and search filters on the model,
/// so naming a different embedder (#179) leaves every existing vector unreachable —
/// silently, since retrieval degrades to keyword-only rather than failing. A probe
/// that reports the embedder answering says nothing about whether the corpus is
/// embedded under it, and those are the two halves a user reads as one.
#[tauri::command]
pub fn chat_unembedded_note_count(state: State<AppState>) -> Result<usize, String> {
    let key = chat_api_key(&state);
    let conn = state.db.lock();
    let Ok(resolved) = resolve_chat(&conn, key) else { return Ok(0) };
    let Some(cfg) = resolve_embed(&resolved) else { return Ok(0) };
    db::note_ids_needing_embedding(&conn, &cfg.model).map(|ids| ids.len()).map_err(super::err)
}

/// Embed what `chat_unembedded_note_count` counts, without waiting for the next
/// launch. Same mechanism as the startup backfill — it already finds exactly the
/// chunks missing vectors under the current model — so the two can't drift.
#[tauri::command]
pub async fn chat_embed_missing(app: AppHandle) -> Result<usize, String> {
    let before = {
        let state: State<AppState> = app.state();
        let key = chat_api_key(&state);
        let conn = state.db.lock();
        let Ok(resolved) = resolve_chat(&conn, key) else { return Ok(0) };
        let Some(cfg) = resolve_embed(&resolved) else { return Ok(0) };
        db::note_ids_needing_embedding(&conn, &cfg.model).map_err(super::err)?.len()
    };
    embed_backfill(app).await;
    Ok(before)
}

/// How many notes a rebuild would repair (#122), so the UI can offer the action only
/// when it does something — and say what it will do.
///
/// A blind "rebuild search index" button is a bad bargain: it is slow, it spends the
/// user's embedding key, and it gives no way to tell whether any of that was needed.
/// With a count, the row can stay quiet on a current library and state the work on a
/// stale one.
#[tauri::command]
pub fn chat_stale_note_count(state: State<AppState>) -> Result<usize, String> {
    let conn = state.db.lock();
    db::notes_with_stale_chunks(&conn).map_err(super::err)
}

/// Rebuild the retrieval index for the WHOLE library (issue #104).
///
/// The only way to repair an existing library after a chunking-shape change. The
/// startup backfill is keyed on sentinels for never-indexed notes, so it cannot see
/// a note whose chunks are present but built the old way — and without this, an
/// archive would stay hard-split until each note happened to be opened, which is
/// precisely the wrong outcome: old meetings are what a "briefing on X" query needs.
///
/// Deliberately user-triggered rather than automatic. Re-chunking changes every
/// chunk's `text_hash`, so the embedding cache misses and the library re-embeds on
/// the user's own API key — cents, but not to be spent unasked.
///
/// Returns the number of notes rebuilt. Embedding is kicked off afterwards and
/// reuses `embed_backfill`, which already finds exactly the chunks missing vectors
/// under the current model, so the two stay one mechanism rather than two.
///
/// `async` + `spawn_blocking` is load-bearing, not decoration: Tauri runs a
/// synchronous command on the main thread, so walking a whole library there would
/// freeze the webview for the duration — the "Rebuilding…" state could never even
/// paint, which is the one piece of UI that makes a slow, key-spending action
/// tolerable. Same shape as the local-Whisper path for the same reason.
#[tauri::command]
pub async fn chat_rebuild_index(app: AppHandle) -> Result<usize, String> {
    // Clone the Arc out and drop the `State` borrow before any await: a
    // `parking_lot` guard held across one would make this future non-Send, and all
    // the locking below happens inside the blocking closure regardless.
    let db = {
        let state: State<AppState> = app.state();
        state.db.clone()
    };
    let rebuilt = tauri::async_runtime::spawn_blocking(move || {
        let ids = {
            let conn = db.lock();
            db::live_note_ids(&conn).map_err(super::err)?
        };
        // One lock acquisition per note, not one around the loop: a large library
        // would otherwise hold the connection long enough to stall every other
        // command that needs the database.
        for id in &ids {
            let conn = db.lock();
            reindex_note_content(&conn, id);
        }
        Ok::<usize, String>(ids.len())
    })
    .await
    .map_err(|e| format!("rebuild task failed: {e}"))??;
    eprintln!("[chat] rebuilt retrieval chunks for {rebuilt} note(s)");
    tauri::async_runtime::spawn(embed_backfill(app));
    Ok(rebuilt)
}

/// Rebuild a Note's retrieval index on demand — the frontend calls this when
/// the Note view unmounts, so edits made without triggering summarize/diarize
/// still land in search. Re-chunks synchronously, then embeds off the request
/// path (issue #48) so unmount stays snappy.
#[tauri::command]
pub fn chat_reindex_note(app: AppHandle, note_id: String) -> Result<(), String> {
    {
        let state: State<AppState> = app.state();
        let conn = state.db.lock();
        reindex_note_content(&conn, &note_id);
    }
    tauri::async_runtime::spawn(embed_note_bg(app, note_id));
    Ok(())
}

/// Whether a Note currently sits in a (non-empty) folder.
fn note_has_folder(note: &db::Note) -> bool {
    note.folder_id.as_deref().is_some_and(|f| !f.is_empty())
}

/// Read the breadth a turn runs at, self-healing a stale value as a side effect
/// (issue #58). The name says `heal_and_read` because this MUTATES: a stored
/// "folder" breadth on a Note that no longer has a folder is reset to "note"
/// (persisted here) so the stored value, the request scope, and the Scope chip
/// never diverge. The folder-edge policy for this issue is auto-heal, not a
/// request-time error, so removing a Note's folder never breaks chat behind the
/// user's back. Errors loudly on an unrecognised stored value rather than
/// clamping — a corrupt row is a bug, not a note-scope turn.
fn heal_and_read_breadth(
    conn: &rusqlite::Connection,
    conversation_id: &str,
    stored: &str,
    note: &db::Note,
) -> Result<String, String> {
    let breadth = chat::validate_breadth(stored)?;
    if breadth == "folder" && !note_has_folder(note) {
        db::set_conversation_breadth(conn, conversation_id, "note").map_err(|e| e.to_string())?;
        return Ok("note".into());
    }
    Ok(breadth.to_string())
}

/// The note-less counterpart: a conversation with no anchor note holds exactly
/// one breadth — `all` for a library-wide thread (#82), `folder` for a
/// folder-scoped one (#110) — because it has no anchor to mean any other by.
/// Heal rather than error, matching the folder-heal above: a corrupt row must not
/// be able to break the user's chat.
///
/// A separate function rather than an `Option<&Note>` arm on the one above,
/// because `None` there would hide a whole policy branch behind a nullable
/// parameter — a reader couldn't tell "note-less" from "note not loaded".
fn heal_and_read_pinned_breadth(
    conn: &rusqlite::Connection,
    conversation_id: &str,
    stored: &str,
    pinned: &'static str,
) -> Result<String, String> {
    if chat::validate_breadth(stored)? != pinned {
        db::set_conversation_breadth(conn, conversation_id, pinned).map_err(|e| e.to_string())?;
    }
    Ok(pinned.into())
}

/// Effective breadth for whichever target a turn is on — the one place the heal
/// policies are chosen between.
///
/// A folder target pins to `folder` rather than being allowed to widen: the clamp
/// is what "chat about this folder" means, and it matches how a note-scoped thread
/// can't be widened past its own note either. A user who wants the library has
/// `/chat` one click away.
fn effective_breadth(
    conn: &rusqlite::Connection,
    conversation_id: &str,
    stored: &str,
    target: &ChatTarget,
    note: Option<&db::Note>,
) -> Result<String, String> {
    match (target, note) {
        (ChatTarget::Folder(_), _) => {
            heal_and_read_pinned_breadth(conn, conversation_id, stored, "folder")
        }
        (_, Some(note)) => heal_and_read_breadth(conn, conversation_id, stored, note),
        (_, None) => heal_and_read_pinned_breadth(conn, conversation_id, stored, "all"),
    }
}

/// Load a turn's anchor note, or None for a library-wide turn. There is
/// deliberately no fallback note (#82) — a global turn runs on retrieval alone.
fn anchor_note(
    conn: &rusqlite::Connection,
    target: &ChatTarget,
) -> Result<Option<db::Note>, String> {
    match target.note_id() {
        Some(id) => db::get_note(conn, id).map(Some).map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// The reference block for a turn, plus the anchor's plain-text body when there is
/// one (the caller reindexes it). A library-wide turn gets NO grounding at all —
/// `assemble_prompt` skips an empty block, and nothing was truncated because
/// nothing was injected.
fn turn_grounding(note: Option<&db::Note>) -> (chat::Grounding, Option<String>) {
    match note {
        Some(note) => {
            let body_text = crate::html_text::html_to_text(&note.body);
            let grounding =
                chat::build_grounding(&body_text, &note.transcript, &note.summary);
            (grounding, Some(body_text))
        }
        None => (chat::Grounding { text: String::new(), truncated: false }, None),
    }
}

/// Resolve an (already-effective) breadth into a server-enforced `ToolScope`.
///
/// "folder" resolves to the conversation's OWN folder when it has one (#110),
/// falling back to the anchor Note's folder — the only source there was before,
/// back when a folder breadth could only be reached from a note that sat in one.
/// The thread's own folder wins because it is the identity the user chose; the
/// note's is merely where the note happens to live.
///
/// Unrecognised breadths and a folder breadth with no folder from either source
/// are loud errors (issue #58) rather than a silent clamp to Note — in practice
/// `heal_and_read_breadth` heals the folder-less case upstream, so those arms are
/// belt-and-suspenders.
///
/// `note` is `None` for a note-less turn (#93, #110), which resolves without
/// reaching for an anchor — the retrieval tools carry it alone.
fn resolve_scope(
    breadth: &str,
    note: Option<&db::Note>,
    target_folder: Option<&str>,
) -> Result<ToolScope, String> {
    match chat::validate_breadth(breadth)? {
        "all" => Ok(ToolScope::All),
        "folder" => {
            let folder = target_folder
                .or_else(|| note.and_then(|n| n.folder_id.as_deref()))
                .filter(|f| !f.is_empty());
            match folder {
                Some(folder_id) => Ok(ToolScope::Folder(folder_id.to_string())),
                None => Err("This note isn't in a folder, so \"Folder\" scope isn't available.".into()),
            }
        }
        // A "note" breadth with no note can only be a corrupt global row.
        // `heal_and_read_global_breadth` rewrites those to "all" before a turn ever
        // reaches here, so this is unreachable in practice — and it errors rather
        // than widening to the whole library, which would silently search
        // everything on the strength of a bad row. Loud, like every other arm.
        _ => match note {
            Some(n) => Ok(ToolScope::Note(n.id.clone())),
            None => Err(
                "This conversation has no anchor note, so only \"All notes\" scope applies.".into(),
            ),
        },
    }
}

/// Whether a model id is embedding-only (can't do chat completions). Mirrors
/// the frontend `isEmbeddingModel` heuristic — matches embeddinggemma and the
/// common embedding families.
fn is_embedding_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("embed")
        || m.starts_with("bge-")
        || m.starts_with("all-minilm")
        || m.starts_with("paraphrase-")
}

/// Whether a stored `chat_model` belongs to the provider now in force. One
/// setting serves every provider, so switching leaves the last one's id behind;
/// a foreign id is read as unset rather than sent. Local ids are arbitrary
/// strings and nothing is claimed about them.
fn suits_provider(model: &str, provider: ProviderId) -> bool {
    let is_claude = model.starts_with("claude-");
    let ok = match provider {
        ProviderId::Anthropic => is_claude,
        ProviderId::OpenAi => !is_claude,
        _ => true,
    };
    if !ok {
        eprintln!("[chat] stored chat_model “{model}” isn't a {provider} model — using the default");
    }
    ok
}

// Resolved chat provider for a single call.
struct ResolvedChat {
    provider: ProviderId,
    base_url: String,
    api_key: Option<String>,
    model: String,
    think: bool,
    embed: EmbedOverride,
}

/// What the user has said about the local embedder, if anything. `None` on
/// either half means "follow the chat provider".
#[derive(Default)]
struct EmbedOverride {
    base_url: Option<String>,
    model: Option<String>,
}

fn resolve_chat(
    conn: &rusqlite::Connection,
    api_key: Option<String>,
) -> anyhow::Result<ResolvedChat> {
    // Read a setting as a non-empty trimmed value, or None.
    let setting = |key: &str| -> anyhow::Result<Option<String>> {
        Ok(db::get_setting(conn, key)?
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()))
    };

    let provider = setting("chat_provider")?.unwrap_or_else(|| "local".into());
    let model_setting = setting("chat_model")?;
    let id = ProviderId::parse(&provider)
        .filter(|p| p.capabilities().chat)
        .ok_or_else(|| anyhow::anyhow!("unknown chat provider “{provider}” — pick one in Settings → Chat."))?;

    match id {
        ProviderId::Local => {
            let base_url =
                setting("local_llm_base_url")?.unwrap_or_else(|| DEFAULT_LOCAL_LLM_BASE_URL.to_string());
            let think = db::get_setting(conn, "local_llm_think")?
                .map(|s| s.trim().eq_ignore_ascii_case("true"))
                .unwrap_or(false);
            let model = model_setting.ok_or_else(|| {
                anyhow::anyhow!("No chat model configured — pick one in Settings → Chat.")
            })?;
            // Defence in depth: an embedding model (e.g. embeddinggemma, pulled
            // for semantic search) can't do chat — Ollama 400s "does not support
            // chat". The pickers now exclude it, but guard a stale setting too.
            if is_embedding_model(&model) {
                anyhow::bail!(
                    "“{model}” is an embedding model and can't chat — pick a chat model in \
                     Settings → Chat."
                );
            }
            let embed = EmbedOverride {
                base_url: setting("embed_base_url")?,
                model: setting("embed_model")?,
            };
            Ok(ResolvedChat { provider: id, base_url, api_key: None, model, think, embed })
        }
        ProviderId::OpenAi => {
            let api_key = api_key.filter(|s| !s.is_empty()).ok_or_else(|| {
                anyhow::anyhow!("OpenAI API key not set — add one in Settings → Chat.")
            })?;
            // A fresh install may have a key but no explicit model yet; fall
            // back to the default chat-class model rather than erroring. So
            // does a model left behind by another provider — `chat_model` is
            // one setting shared by all of them.
            let model = model_setting
                .filter(|m| suits_provider(m, id))
                .unwrap_or_else(|| DEFAULT_SUMMARY_MODEL.to_string());
            Ok(ResolvedChat {
                provider: id,
                base_url: openai::BASE.into(),
                api_key: Some(api_key),
                model,
                think: false,
                embed: EmbedOverride::default(),
            })
        }
        ProviderId::Anthropic => {
            let api_key = api_key.filter(|s| !s.is_empty()).ok_or_else(|| {
                anyhow::anyhow!("Anthropic API key not set — add one in Settings → Chat.")
            })?;
            let model = model_setting
                .filter(|m| suits_provider(m, id))
                .unwrap_or_else(|| crate::anthropic::DEFAULT_MODEL.to_string());
            Ok(ResolvedChat {
                provider: id,
                base_url: crate::anthropic::BASE.into(),
                api_key: Some(api_key),
                model,
                think: false,
                embed: EmbedOverride::default(),
            })
        }
        ProviderId::Deepgram | ProviderId::Groq | ProviderId::Parakeet => {
            unreachable!("filtered on the chat capability")
        },
    }
}

// ── Event payloads (camelCase to match the app's other Tauri events) ────────
// Slice-3 subset of the #46 wire contract: text-delta, done, error. Each
// carries conversationId; the delta/done ones also carry the assistant
// messageId (the row exists by then).

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatTextDeltaPayload {
    conversation_id: String,
    message_id: String,
    block_id: String,
    delta: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatDonePayload {
    conversation_id: String,
    message_id: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatErrorPayload {
    conversation_id: String,
    message: String,
    /// Machine reason code (issue #76) so the client can render role-aware copy
    /// for the BYOK error taxonomy (byok_key_invalid / byok_provider_quota /
    /// byok_key_unavailable / chat_not_activated). Empty when unknown.
    #[serde(default)]
    reason: String,
}

/// Tool activity between steps — drives the "searching your notes…" progress
/// line (issue #47, story 18).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatToolActivityPayload {
    conversation_id: String,
    message_id: String,
    name: String,
    is_error: bool,
}

/// Sources gathered so far, for citation chips (story 28).
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatCitationsPayload {
    conversation_id: String,
    message_id: String,
    citations: Vec<Citation>,
}

/// Synchronous result of `chat_send`: enough for the UI to attach a
/// "context truncated" notice to the turn and to know which conversation it
/// landed in. The streamed answer arrives via events, not here.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSendResult {
    conversation_id: String,
    truncated: bool,
}

// ── Chat sessions (issue #61) ───────────────────────────────────────────────
// Multiple conversations ("sessions") per Note. The active session is the most-
// recently-updated one; an explicit command creates a fresh one. Sessions can be
// deleted and renamed since #109 (`chat_delete_conversation` /
// `chat_rename_conversation`).

/// The longest title we store on a rename (issue #109).
const TITLE_MAX_STORED_CHARS: usize = 300;

/// One row in the session list. `id` is the local conversation id, which
/// `chat_history` / `chat_send` / breadth all key on. `title` is resolved (stored title, else a date
/// fallback) so a not-yet-titled session still shows something.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMeta {
    id: String,
    title: String,
    breadth: String,
    /// The pinned Client's id, or "" for none. The token resolves it to a name
    /// against the Client list.
    client_filter: String,
    /// The pinned speaker's label, or "" for none. Already a display string —
    /// there is no id to resolve (ADR-0002).
    speaker_filter: String,
    updated_at: i64,
    message_count: i64,
}

/// `chat_history` result (issue #61): the messages plus the conversation they
/// were resolved to, so the panel learns which session it's on. `conversation_id`
/// is None when the Note has no session yet (opening the tab creates nothing).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatHistoryResult {
    conversation_id: Option<String>,
    messages: Vec<ChatMessageDto>,
}

/// A conversation's display title: its stored title, else a derived date label
/// (an untitled session — e.g. one just created — still reads sensibly).
fn resolved_title(c: &db::Conversation) -> String {
    match c.title.as_deref() {
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => chat::derive_title(None, c.created_at),
    }
}

/// Whether a conversation has no stored title yet — the first user message
/// should set it (issue #61).
fn resolved_title_is_unset(c: &db::Conversation) -> bool {
    c.title.as_deref().map_or(true, |t| t.trim().is_empty())
}

/// Build a session-list entry for a local row.
fn conversation_meta(
    conn: &rusqlite::Connection,
    c: &db::Conversation,
) -> Result<ConversationMeta, String> {
    let message_count =
        db::conversation_message_count(conn, &c.id).map_err(|e| e.to_string())?;
    Ok(ConversationMeta {
        id: c.id.clone(),
        title: resolved_title(c),
        breadth: c.breadth.clone(),
        client_filter: c.client_filter.clone(),
        speaker_filter: c.speaker_filter.clone(),
        updated_at: c.updated_at,
        message_count,
    })
}

/// The breadth a new session inherits (issue #61): the target's most-recent
/// session's breadth, or the target's own default for its first-ever session
/// ("note" for a Note, "all" for the library — a global thread has no anchor to
/// narrow to).
fn inherited_breadth(conn: &rusqlite::Connection, tenant: &str, target: &ChatTarget) -> String {
    db::latest_conversation(conn, tenant, target.scope(), target.scope_id())
        .ok()
        .flatten()
        .map(|c| c.breadth)
        .unwrap_or_else(|| chat::default_breadth(target).to_string())
}

/// Resolve the session a command targets WITHOUT creating one (None when the
/// Note has no session yet). `explicit` targets a specific session; otherwise
/// the most-recently-updated one is the active session.
///
/// An explicit id is validated to belong to the expected (tenant, scope,
/// scope_id) — a mismatched id is a clean error, never a silent cross-scope
/// operation (which would, e.g., heal breadth against the wrong Note, or splice
/// a note thread into the library list). A not-found explicit id is treated as
/// "no session" (None) so the caller falls back to the active one.
fn resolve_existing(
    conn: &rusqlite::Connection,
    tenant: &str,
    target: &ChatTarget,
    explicit: Option<&str>,
) -> Result<Option<db::Conversation>, String> {
    if let Some(id) = explicit {
        let Some(c) = db::get_conversation_by_id(conn, id).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        if c.tenant != tenant || c.scope != target.scope() || c.scope_id != target.scope_id() {
            return Err(match target {
                ChatTarget::Note(_) => "That chat session doesn't belong to this note.".into(),
                ChatTarget::Folder(_) => {
                    "That chat session doesn't belong to this folder.".to_string()
                }
                ChatTarget::Global => "That chat session isn't a library-wide one.".to_string(),
            });
        }
        return Ok(Some(c));
    }
    db::latest_conversation(conn, tenant, target.scope(), target.scope_id())
        .map_err(|e| e.to_string())
}

/// Whether opening this target's pane RESUMES its most-recent conversation, or
/// starts an unsaved draft (issue #120).
///
/// A library-wide pane drafts. `/chat` is a front door, and asking a new question
/// is overwhelmingly what it is for — reopening a finished thread there hid the
/// prompt cards, named the bar after an old question, and made "ask something"
/// take a second deliberate action. Returning to a specific past thread is the
/// rarer, more intentional act, and the sidebar list already exists for it.
///
/// A folder pane drafts for the same reason (#110): it is a route you navigate to
/// in order to ask something, not an object you were already reading. The folder
/// narrows *what is in reach*, which is not the same as being an anchor — there is
/// no folder text to continue a line of thinking about.
///
/// A Note's pane resumes, deliberately diverging: a note is an anchor, and coming
/// back to it to continue the same line of thinking is a plausible default in a
/// way it isn't on a route. Recorded here, in one predicate, so the divergence
/// stays a decision rather than becoming an accident — every caller that cares
/// reads it from this function.
fn resumes_on_open(target: &ChatTarget) -> bool {
    match target {
        ChatTarget::Note(_) => true,
        ChatTarget::Folder(_) | ChatTarget::Global => false,
    }
}

/// The conversation-list filter implied by the resume rule (issue #120).
///
/// A target that drafts can never return to an empty thread, so an empty row in
/// its list is residue — from an older client, or a breadth chosen then abandoned
/// — and hiding it is right. A target that resumes has the opposite relationship
/// to the same row: it IS the draft being resumed, so hiding it would hide what
/// the pane is showing.
fn list_filter_for(target: &ChatTarget) -> db::ListFilter {
    if resumes_on_open(target) {
        db::ListFilter::All
    } else {
        db::ListFilter::WithMessages
    }
}

/// Resolve the session a READ command targets, honouring the resume rule (#120).
///
/// Shared by `chat_history`, `chat_get_breadth` and `chat_get_pin` so all
/// three agree about what a bare request means. An explicit id always wins — a
/// thread picked from the sidebar must open — but a bare request on a drafting
/// target resolves to nothing rather than to whatever was newest.
fn resolve_for_read(
    conn: &rusqlite::Connection,
    tenant: &str,
    target: &ChatTarget,
    explicit: Option<&str>,
) -> Result<Option<db::Conversation>, String> {
    if explicit.is_none() && !resumes_on_open(target) {
        return Ok(None);
    }
    resolve_existing(conn, tenant, target, explicit)
}

/// Resolve the session a SETTINGS write targets, or `None` when there is nothing
/// to write to yet (issue #120).
///
/// A Note still lazily creates its first session, which is what #61 wanted: that
/// pane resumes, so a breadth chosen before the first turn has to be stored
/// somewhere it will be found again. A drafting target returns `None` instead —
/// the pane holds the value, and `DraftSettings` carries it into the turn that
/// makes the row.
fn resolve_for_write(
    conn: &rusqlite::Connection,
    tenant: &str,
    target: &ChatTarget,
    explicit: Option<&str>,
) -> Result<Option<db::Conversation>, String> {
    if explicit.is_none() && !resumes_on_open(target) {
        return Ok(None);
    }
    Ok(Some(resolve_or_create(conn, tenant, target, explicit, DraftSettings::default())?))
}

/// A pin value as an optional: empty (and blank) is "no pin", never a pin on
/// nothing. The one owner of that normalisation, so no read path depends on a
/// setter having refused the blank first.
fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// What a draft had chosen before it had a row to store it on (issue #120).
///
/// A library-wide pane holds its breadth and authorship pin locally until the
/// first turn, so those values arrive with the send that materialises the row.
/// This replaces the old lazy-create (#61, #103): a row that existed only to hold
/// a pre-turn setting was invisible in the list yet still resolved to by the next
/// send, so the pane could show "no pin" while the stored row narrowed the turn
/// anyway — exactly the lie #103 set out to prevent. Now the row appears once,
/// already carrying what the chips showed.
#[derive(Debug, Default, Clone)]
pub struct DraftSettings {
    pub breadth: Option<String>,
    pub client_filter: Option<String>,
    pub speaker_filter: Option<String>,
}

/// The one breadth a note-less target may hold, if it is pinned to one.
///
/// A library-wide thread is `all` and a folder thread is `folder`: with no anchor
/// note, the target's identity IS its reach, so there is nothing for a picker to
/// offer. (A Note is the exception — it can widen to its folder or the library,
/// which is what the breadth picker is for.) Returns `None` when the target has a
/// real choice to make.
fn pinned_breadth(target: &ChatTarget) -> Option<&'static str> {
    match target {
        ChatTarget::Note(_) => None,
        ChatTarget::Folder(_) => Some("folder"),
        ChatTarget::Global => Some("all"),
    }
}

/// Reject a breadth a pinned target may not hold. One owner for the rule, since
/// both `chat_set_breadth` and `validated_draft` write the same column and would
/// otherwise be free to disagree about it.
fn check_pinned_breadth(target: &ChatTarget, breadth: &str) -> Result<(), String> {
    match pinned_breadth(target) {
        Some(pinned) if breadth != pinned => Err(match target {
            ChatTarget::Folder(_) => {
                "A folder chat always searches that folder.".to_string()
            }
            _ => "A library-wide chat always searches all notes.".to_string(),
        }),
        _ => Ok(()),
    }
}

/// Check a draft's pending settings against the same rules their dedicated
/// setters enforce (issue #120).
///
/// `chat_set_breadth` rejects a value outside the vocabulary and a non-`all`
/// breadth on a library-wide target. `DraftSettings` writes the same columns as
/// the setters, so it answers to the same rules — a chip that can't
/// produce an illegal value today is not a reason for the command to accept one.
fn validated_draft(
    target: &ChatTarget,
    breadth: Option<String>,
    client_filter: Option<String>,
    speaker_filter: Option<String>,
) -> Result<DraftSettings, String> {
    if let Some(b) = breadth.as_deref() {
        chat::validate_breadth(b)?;
        check_pinned_breadth(target, b)?;
    }
    Ok(DraftSettings {
        breadth,
        client_filter: client_filter.as_deref().and_then(non_empty),
        speaker_filter: speaker_filter.as_deref().and_then(non_empty),
    })
}

/// Resolve the active session, creating one when none applies.
///
/// Creation happens either because the target has no session yet, or — on a
/// drafting target with no explicit id — because a bare send always starts its own
/// conversation. That second case is load-bearing: resolving a draft's first turn
/// to "most recent" would file the message into a thread the user believed they
/// had left, which is worse than any list bug.
fn resolve_or_create(
    conn: &rusqlite::Connection,
    tenant: &str,
    target: &ChatTarget,
    explicit: Option<&str>,
    draft: DraftSettings,
) -> Result<db::Conversation, String> {
    if explicit.is_some() || resumes_on_open(target) {
        if let Some(c) = resolve_existing(conn, tenant, target, explicit)? {
            return Ok(c);
        }
    }
    // A draft's own choice wins over inheritance; absent one, "+"-style
    // inheritance is unchanged.
    let breadth = draft.breadth.unwrap_or_else(|| inherited_breadth(conn, tenant, target));
    let conv = db::create_conversation(conn, tenant, target.scope(), target.scope_id(), &breadth)
        .map_err(|e| e.to_string())?;
    if let Some(client) = draft.client_filter.as_deref().and_then(non_empty) {
        db::set_conversation_client_filter(conn, &conv.id, Some(&client))
            .map_err(|e| e.to_string())?;
    }
    if let Some(speaker) = draft.speaker_filter.as_deref().and_then(non_empty) {
        db::set_conversation_speaker_filter(conn, &conv.id, Some(&speaker))
            .map_err(|e| e.to_string())?;
    }
    let reread = |conn: &rusqlite::Connection| {
        db::get_conversation_by_id(conn, &conv.id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "conversation vanished after insert".to_string())
    };
    let pinned = draft.client_filter.is_some() || draft.speaker_filter.is_some();
    if pinned { reread(conn) } else { Ok(conv) }
}

/// Resolve the "new session" for a target in the personal scope (issue #61),
/// honoring the no-op guard: if the most-recent session has no messages, reuse
/// it instead of piling up empty sessions; otherwise create a fresh one that
/// inherits breadth from the most-recent session.
fn new_personal_conversation(
    conn: &rusqlite::Connection,
    target: &ChatTarget,
) -> Result<db::Conversation, String> {
    if let Some(latest) =
        db::latest_conversation(conn, CHAT_TENANT_PERSONAL, target.scope(), target.scope_id())
            .map_err(|e| e.to_string())?
    {
        if db::conversation_message_count(conn, &latest.id).map_err(|e| e.to_string())? == 0 {
            return Ok(latest);
        }
    }
    let breadth = inherited_breadth(conn, CHAT_TENANT_PERSONAL, target);
    db::create_conversation(
        conn,
        CHAT_TENANT_PERSONAL,
        target.scope(),
        target.scope_id(),
        &breadth,
    )
    .map_err(|e| e.to_string())
}

/// List a Note's chat sessions, most-recently-updated first (issue #61).
/// Opening/listing creates nothing.
#[tauri::command]
pub async fn chat_list_conversations(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<Vec<ConversationMeta>, String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let page = conversation_page(limit, offset);
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    let convs = db::list_conversations(
        &conn,
        CHAT_TENANT_PERSONAL,
        target.scope(),
        target.scope_id(),
        page,
        list_filter_for(&target),
    )
    .map_err(|e| e.to_string())?;
    convs.iter().map(|c| conversation_meta(&conn, c)).collect()
}

/// The requested window, or None for "everything" (issue #95).
///
/// A caller that omits `limit` gets the whole list — the Note header's history
/// popover has always shown all of a note's conversations and there is no reason
/// to change that. `offset` without `limit` is meaningless and ignored rather
/// than errored: it can only come from our own frontend, and a dropped window is
/// a longer list, never a wrong one.
fn conversation_page(limit: Option<u32>, offset: Option<u32>) -> Option<db::Page> {
    limit.map(|limit| db::Page { limit: limit.into(), offset: offset.unwrap_or(0).into() })
}

/// Create a fresh chat session for a target (issue #61): a local row, with the
/// no-op guard reusing an empty most-recent session.
#[tauri::command]
pub async fn chat_new_conversation(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
) -> Result<ConversationMeta, String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    let conv = new_personal_conversation(&conn, &target)?;
    conversation_meta(&conn, &conv)
}

/// Delete a conversation and its messages (issue #109).
///
/// The id is explicit and required — there is no "delete the active one" fallback.
/// Every other command here resolves an absent id to the most-recent session,
/// which is a helpful default for reading and an unacceptable one for destroying:
/// a caller that forgot to pass an id would silently take out whatever thread
/// happened to be newest.
#[tauri::command]
pub async fn chat_delete_conversation(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: String,
) -> Result<(), String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let state: State<AppState> = app.state();
    // Resolve within the caller's tenant + target first, so a stray id can't
    // reach another note's or another tenant's thread.
    let conv = {
        let conn = state.db.lock();
        resolve_existing(&conn, CHAT_TENANT_PERSONAL, &target, Some(&conversation_id))?
    };
    let Some(conv) = conv else {
        // Nothing local to remove. Idempotent by intent: the UI drops the row
        // optimistically, so a retry after a partial failure must still succeed.
        return Ok(());
    };

    let conn = state.db.lock();
    db::delete_conversation(&conn, &conv.id).map_err(|e| e.to_string())
}

/// Rename a conversation (issue #109) — the user's override of the title derived
/// from the first turn.
///
/// Rejects an all-whitespace title, because an empty title is how "never titled"
/// is spelled — storing one would re-arm the send path's derivation and let the
/// next turn overwrite the user's choice.
#[tauri::command]
pub async fn chat_rename_conversation(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: String,
    title: String,
) -> Result<ConversationMeta, String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let state: State<AppState> = app.state();
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err("A conversation needs a name.".into());
    }
    // Sliced on a char boundary.
    let capped: String = trimmed.chars().take(TITLE_MAX_STORED_CHARS).collect();

    let conv = {
        let conn = state.db.lock();
        resolve_existing(&conn, CHAT_TENANT_PERSONAL, &target, Some(&conversation_id))?
    };
    let Some(conv) = conv else {
        return Err("That conversation no longer exists.".into());
    };

    let conn = state.db.lock();
    db::rename_conversation(&conn, &conv.id, &capped).map_err(|e| e.to_string())?;
    let updated = db::get_conversation_by_id(&conn, &conv.id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "That conversation no longer exists.".to_string())?;
    conversation_meta(&conn, &updated)
}

/// Persist the Scope chip's breadth on a conversation (issue #58) — the single
/// source of truth for retrieval breadth. Validates the value loudly (garbage →
/// Err), resolves the target session (`conversation_id` when the panel has one,
/// else the active/most-recent session, lazily creating the Note's first
/// session so a breadth chosen before the first turn survives — issue #61), and
/// writes the column.
#[tauri::command]
pub fn chat_set_breadth(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: Option<String>,
    breadth: String,
) -> Result<(), String> {
    chat::validate_breadth(&breadth)?;
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    // A note-less target's identity IS its reach, so it holds exactly one breadth
    // and no picker can send another (#82, #110). Reject rather than store a value
    // `effective_breadth` would immediately undo.
    check_pinned_breadth(&target, &breadth)?;
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    // A draft has nowhere to store this yet, and must not grow a row to hold it
    // (#120): the pane keeps the value and the first turn carries it in via
    // `DraftSettings`. Writing here would create a row the list hides but the next
    // send resolves to — the stale-setting trap this replaced.
    let Some(conversation) = resolve_for_write(&conn, CHAT_TENANT_PERSONAL, &target, conversation_id.as_deref())?
    else {
        return Ok(());
    };
    db::set_conversation_breadth(&conn, &conversation.id, &breadth).map_err(|e| e.to_string())
}

/// Which pin a `chat_set_pin` / `chat_get_pin` call is about.
///
/// One pair of commands rather than a pair per pin: the resolve-or-return-early
/// dance is the whole body, and copying it per pin is how one copy comes to
/// disagree with the others about drafts or tenancy.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinKind {
    Client,
    Speaker,
}

/// Pin (or clear) the conversation's Client or speaker filter.
///
/// A draft holds its pin in the pane until the first turn creates the row (#120),
/// which is why a missing conversation returns early rather than creating one.
#[tauri::command]
pub fn chat_set_pin(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: Option<String>,
    kind: PinKind,
    value: Option<String>,
) -> Result<(), String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let value = value.as_deref().and_then(non_empty);
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    let Some(conversation) =
        resolve_for_write(&conn, CHAT_TENANT_PERSONAL, &target, conversation_id.as_deref())?
    else {
        return Ok(());
    };
    let write = match kind {
        PinKind::Client => db::set_conversation_client_filter,
        PinKind::Speaker => db::set_conversation_speaker_filter,
    };
    write(&conn, &conversation.id, value.as_deref()).map_err(|e| e.to_string())
}

/// Read a persisted pin. `""` = off, and so is no session yet: a pin is only ever
/// something the user set, so there is nothing to inherit.
#[tauri::command]
pub fn chat_get_pin(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: Option<String>,
    kind: PinKind,
) -> Result<String, String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    Ok(
        resolve_for_read(&conn, CHAT_TENANT_PERSONAL, &target, conversation_id.as_deref())?
            .map(|c| match kind {
                PinKind::Client => c.client_filter,
                PinKind::Speaker => c.speaker_filter,
            })
            .unwrap_or_default(),
    )
}

/// Read the persisted breadth for a conversation so the Scope chip initialises
/// from the backend in one round trip (issue #58). Resolves `conversation_id`
/// when the panel has one, else the active/most-recent session; when the Note
/// has no session yet it returns the would-be-inherited default (issue #61) so
/// the chip shows something sane WITHOUT creating a row. NOTE: this read may
/// PERSIST a heal — a stale "folder" breadth (the Note's folder was since
/// removed) is reset to "note" via `heal_and_read_breadth`; the heal-on-read is
/// intentional.
#[tauri::command]
pub fn chat_get_breadth(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: Option<String>,
) -> Result<String, String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    let Some(conversation) =
        resolve_for_read(&conn, CHAT_TENANT_PERSONAL, &target, conversation_id.as_deref())?
    else {
        // No session yet → report the target's own default WITHOUT creating a row,
        // which is also what `inherited_breadth` would return here (its
        // `latest_conversation` read would be redundant — `resolve_existing` just
        // established there's no session).
        return Ok(chat::default_breadth(&target).into());
    };
    // A note-less conversation has no anchor to heal against.
    let note = anchor_note(&conn, &target)?;
    effective_breadth(&conn, &conversation.id, &conversation.breadth, &target, note.as_ref())
}

/// The name to call the person asking, or `None` when we genuinely don't know
/// one. It is the referent for "I"/"me"/"my" in a turn (#103) — without it, a
/// third of the questions people ask a meeting assistant can't be resolved.
///
/// Two sources, in order:
///
/// 1. The `user_display_name` setting, when the user has set one.
/// 2. The macOS account's full name.
///
/// The OS name matters because it's the ONLY one the local-only majority has:
/// Humla has no account. Falling back to it is what keeps "what did I promise?"
/// answerable. A short login name (`msmith`) is
/// deliberately not used — as a referent it's worse than nothing.
///
/// Never fatal: any failure just omits the line from the prompt.
pub(crate) fn asker_name(conn: &rusqlite::Connection) -> Option<String> {
    display_name_chain(conn)
}

/// The one name chain: `user_display_name` → the macOS account's full name,
/// first non-empty wins. Keeping the order here means the prompt's referent and
/// the rename prefill can never disagree about who you are.
fn display_name_chain(conn: &rusqlite::Connection) -> Option<String> {
    let non_empty = |s: &str| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t.to_string()) }
    };
    crate::db::get_setting(conn, SETTING_DISPLAY_NAME)
        .ok()
        .flatten()
        .and_then(|v| non_empty(&v))
        .or_else(os_full_name)
}

/// The name to prefill when renaming the literal `You:` label to a real one
/// (#116 part 2).
///
/// The same [`display_name_chain`] the prompt's asker line uses, so the name a
/// transcript gets renamed to and the name the model calls you cannot diverge.
///
/// The result is only ever a *suggestion*: the field is editable, deliberately.
/// An automatic migration was rejected partly because Personal falls back to the
/// OS name and could stamp `admin` across every transcript — showing the exact
/// target before it runs is that mitigation, and an uneditable prefill would
/// leave such a user stuck on the screen they're already on.
#[tauri::command]
pub fn speaker_default_name(state: State<AppState>) -> Option<String> {
    let conn = state.db.lock();
    display_name_chain(&conn)
}

/// The macOS account's full name ("Michael Wilhelmsen"), via `id -F`. `None` if
/// the command is unavailable, fails, or returns something empty.
fn os_full_name() -> Option<String> {
    let out = std::process::Command::new("/usr/bin/id").arg("-F").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Registers an in-flight turn's stop signal for a pane and clears it on drop,
/// so no early return can leak an entry that would let a later `chat_cancel`
/// abort the *next* turn (issue #80).
struct TurnCancel {
    registry: Arc<Mutex<std::collections::HashMap<String, Arc<chat::CancelFlag>>>>,
    key: String,
    flag: Arc<chat::CancelFlag>,
}

impl TurnCancel {
    fn register(state: &State<'_, AppState>, key: &str) -> Self {
        let flag = Arc::new(chat::CancelFlag::new());
        let registry = Arc::clone(&state.chat_cancels);
        // A fresh send for the same pane replaces any stale entry.
        registry.lock().insert(key.to_string(), Arc::clone(&flag));
        Self { registry, key: key.to_string(), flag }
    }

    fn flag(&self) -> &chat::CancelFlag {
        &self.flag
    }
}

impl Drop for TurnCancel {
    fn drop(&mut self) {
        let mut map = self.registry.lock();
        // Only clear OUR flag: a newer send for this pane may already have
        // replaced it, and removing that one would make its stop button dead.
        let ours = map.get(&self.key).is_some_and(|f| Arc::ptr_eq(f, &self.flag));
        if ours {
            map.remove(&self.key);
        }
    }
}

/// Stop the turn currently streaming in a pane. A no-op when nothing is in
/// flight, so a stray click can't error. The partial answer (if any text
/// arrived) is kept — see `chat::run_chat`.
#[tauri::command]
pub fn chat_cancel(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
) -> Result<(), String> {
    let state: State<AppState> = app.state();
    // Panes are keyed by `scope_id` — the note id for a note pane, the folder id
    // for a folder pane, the global sentinel for the library one — so no pane can
    // stop another's turn.
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    if let Some(flag) = state.chat_cancels.lock().get(target.scope_id()) {
        flag.cancel();
    }
    Ok(())
}

#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: Option<String>,
    message: String,
    // A drafting pane's breadth and pins, chosen before any row existed
    // (#120). Only read when this turn CREATES the conversation; an existing row
    // is already the source of truth for them.
    draft_breadth: Option<String>,
    draft_client_filter: Option<String>,
    draft_speaker_filter: Option<String>,
) -> Result<ChatSendResult, String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let state: State<AppState> = app.state();
    // Validated on the way in, to the same standard as `chat_set_breadth` and
    // `chat_set_pin`. These fields reach the same columns those
    // commands guard, so accepting them unchecked here would be a second, unguarded
    // door onto the same state — and the frontend enforcing it is not the same as
    // the backend enforcing it.
    let draft = validated_draft(
        &target,
        draft_breadth,
        draft_client_filter,
        draft_speaker_filter,
    )?;
    // Keychain read out of band — not inside the DB lock, and only the slot
    // the configured chat provider uses.
    let chat_key = chat_api_key(&state);

    let (grounding, resolved, conversation_id, tool_scope, pins, workspace) = {
        let conn = state.db.lock();
        let note = anchor_note(&conn, &target)?;
        let workspace = String::new();
        let resolved = resolve_chat(&conn, chat_key).map_err(|e| e.to_string())?;
        // Resolve the target session (issue #61): an explicit id, else the
        // active/most-recent one, lazily creating the target's first session on
        // the first send. Breadth is a persisted live filter within it.
        let conversation =
            resolve_or_create(&conn, CHAT_TENANT_PERSONAL, &target, conversation_id.as_deref(), draft)?;
        // Set the personal session's title once, from its first user message
        // (issue #61). Guarded on an empty title so later turns never rewrite it.
        if resolved_title_is_unset(&conversation) {
            let title = chat::derive_title(Some(&message), conversation.created_at);
            let _ = db::set_conversation_title(&conn, &conversation.id, &title);
        }
        let (grounding, body_text) = turn_grounding(note.as_ref());
        // Keep the anchor Note searchable — reindex it now so "this Note" and
        // any broader search always find the note the user is looking at, even
        // if a content-settled checkpoint hasn't fired yet. Nothing to do for a
        // library-wide turn: every note is reindexed at its own checkpoints.
        if let (Some(note), Some(body_text)) = (note.as_ref(), body_text.as_deref()) {
            let _ = db::reindex_note(&conn, &note.id, body_text, &note.transcript, &note.summary);
        }
        // Breadth is read from the conversation row (single source of truth),
        // self-healed against the Note's current folder.
        let breadth =
            effective_breadth(&conn, &conversation.id, &conversation.breadth, &target, note.as_ref())?;
        // The Client and speaker pins clamp local retrieval.
        let pins = chat::Pins {
            client: non_empty(&conversation.client_filter),
            speaker: non_empty(&conversation.speaker_filter),
        };
        let tool_scope = resolve_scope(&breadth, note.as_ref(), target.folder_id())?;
        (grounding, resolved, conversation.id, tool_scope, pins, workspace)
    };

    // Embed the anchor Note now (best-effort, cached) so semantic search works
    // for the note the user is chatting about on the very first question. Other
    // notes are embedded at their own checkpoints (issue #48) — so a library-wide
    // turn has nothing to pre-embed and skips straight to the loop.
    let embedder = resolve_embed(&resolved).map(|cfg| cfg.adapter());
    if let (Some(embedder), Some(anchor)) = (embedder.as_ref(), target.note_id()) {
        embed_note(&state.db, embedder, anchor).await;
    }

    let adapter = chat::build_chat_adapter(resolved.provider).map_err(|e| e.to_string())?;
    let conv_for_sink = conversation_id.clone();
    let app_for_sink = app.clone();
    let sink = move |ev: ChatEvent| match ev {
        ChatEvent::TextDelta { message_id, block_id, delta } => {
            let _ = app_for_sink.emit(
                "chat_text_delta",
                ChatTextDeltaPayload {
                    conversation_id: conv_for_sink.clone(),
                    message_id,
                    block_id,
                    delta,
                },
            );
        }
        ChatEvent::ToolActivity { message_id, name, is_error } => {
            let _ = app_for_sink.emit(
                "chat_tool_activity",
                ChatToolActivityPayload {
                    conversation_id: conv_for_sink.clone(),
                    message_id,
                    name,
                    is_error,
                },
            );
        }
        ChatEvent::Citations { message_id, citations } => {
            let _ = app_for_sink.emit(
                "chat_citations",
                ChatCitationsPayload { conversation_id: conv_for_sink.clone(), message_id, citations },
            );
        }
        ChatEvent::Done { message_id } => {
            let _ = app_for_sink.emit(
                "chat_done",
                ChatDonePayload { conversation_id: conv_for_sink.clone(), message_id },
            );
        }
    };

    // Register the stop signal for this pane; dropped (and cleared) when the
    // turn finishes, however it finishes (issue #80).
    let turn = TurnCancel::register(&state, target.scope_id());
    let ctx = ChatCtx {
        model: &resolved.model,
        api_key: resolved.api_key.as_deref(),
        base_url: &resolved.base_url,
        think: resolved.think,
        cancel: turn.flag(),
    };
    // Who is asking, for the prompt's first-person referent (#103). Resolved here
    // rather than inside run_chat so the prompt's inputs stay explicit and the
    // loop's tests stay independent of the host machine's account name.
    let asker = {
        let conn = state.db.lock();
        asker_name(&conn)
    };
    let result = chat::run_chat(
        &state.db,
        adapter.as_ref(),
        ctx,
        &conversation_id,
        &grounding.text,
        &tool_scope,
        &pins,
        &workspace,
        embedder.as_ref().map(|e| e as &dyn EmbeddingAdapter),
        &message,
        asker.as_deref(),
        sink,
    )
    .await;

    match result {
        Ok(()) => Ok(ChatSendResult { conversation_id, truncated: grounding.truncated }),
        Err(e) => {
            let message = e.to_string();
            let _ = app.emit(
                "chat_error",
                ChatErrorPayload {
                    conversation_id: conversation_id.clone(),
                    message: message.clone(),
                    reason: String::new(),
                },
            );
            Err(message)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessageDto {
    id: String,
    role: String,
    seq: i64,
    /// Raw opencode-v2 parts JSON: the serialised `Vec<chat::Part>`.
    parts: serde_json::Value,
    created_at: i64,
}

/// Reload a Note's conversation and report which session it resolved to (issue
/// #61). `conversation_id` targets a specific session; None resolves the
/// active/most-recent one (this is what keeps the single-thread panel working).
/// When the Note has no session yet, returns empty messages + `conversationId:
/// None` WITHOUT creating a row — merely opening the Chat tab must persist
/// nothing.
#[tauri::command]
pub async fn chat_history(
    app: AppHandle,
    note_id: Option<String>,
    folder_id: Option<String>,
    conversation_id: Option<String>,
) -> Result<ChatHistoryResult, String> {
    let target = ChatTarget::from_ids(note_id, folder_id)?;
    let state: State<AppState> = app.state();
    let conn = state.db.lock();
    let Some(conversation) =
        resolve_for_read(&conn, CHAT_TENANT_PERSONAL, &target, conversation_id.as_deref())?
    else {
        return Ok(ChatHistoryResult { conversation_id: None, messages: Vec::new() });
    };
    let messages = db::list_chat_messages(&conn, &conversation.id).map_err(|e| e.to_string())?;
    let messages = messages
        .into_iter()
        .map(|m| ChatMessageDto {
            id: m.id,
            role: m.role,
            seq: m.seq,
            parts: serde_json::to_value(chat::parse_parts(&m.content)).unwrap_or_default(),
            created_at: m.created_at,
        })
        .collect();
    Ok(ChatHistoryResult { conversation_id: Some(conversation.id), messages })
}

#[cfg(test)]
mod tests {
    use super::*;
    // Production code derives the scope from a `ChatTarget`; the tests still name
    // it directly to assert the stored value is what we think it is.
    use crate::db::CHAT_SCOPE_NOTE;

    /// Insert a Note with an explicit folder assignment, for scope tests.
    fn note_in_folder(conn: &rusqlite::Connection, folder: Option<&str>) -> db::Note {
        let note = db::create_note(conn, "en", "meeting", "").unwrap();
        if let Some(f) = folder {
            db::move_note(conn, &note.id, Some(f)).unwrap();
        }
        db::get_note(conn, &note.id).unwrap()
    }

    #[test]
    fn resolve_scope_maps_valid_breadths_and_errors_on_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("scope.sqlite")).unwrap();
        let folder = db::create_folder(&conn, "Team", "").unwrap();
        let with_folder = note_in_folder(&conn, Some(&folder.id));
        let no_folder = note_in_folder(&conn, None);

        assert!(matches!(resolve_scope("all", Some(&no_folder), None).unwrap(), ToolScope::All));
        assert!(matches!(resolve_scope("note", Some(&no_folder), None).unwrap(), ToolScope::Note(_)));
        match resolve_scope("folder", Some(&with_folder), None).unwrap() {
            ToolScope::Folder(id) => assert_eq!(id, folder.id),
            other => panic!("expected Folder scope, got {other:?}"),
        }
        // Unknown breadth is loud (issue #58) — the old `_ => Note` clamp is gone.
        assert!(resolve_scope("everything", Some(&no_folder), None).is_err());
        // Folder breadth on a folder-less note errors rather than clamping.
        assert!(resolve_scope("folder", Some(&no_folder), None).is_err());
    }

    // ── #93: library-wide (global) scope ────────────────────────────────────

    /// A library-wide turn resolves straight to `All` without an anchor — the
    /// retrieval tools carry it alone (#82: no fallback note is substituted).
    #[test]
    fn resolve_scope_resolves_a_library_wide_turn_without_a_note() {
        assert!(matches!(resolve_scope("all", None, None).unwrap(), ToolScope::All));
        // Every anchor-requiring breadth is LOUD without an anchor rather than
        // widening to the whole library — a corrupt global row must not silently
        // search everything. `heal_and_read_global_breadth` rewrites such rows to
        // "all" before a turn gets here, so these arms are belt-and-suspenders.
        for breadth in ["note", "folder"] {
            assert!(resolve_scope(breadth, None, None).is_err(), "{breadth} should not widen silently");
        }
        // Garbage is still loud.
        assert!(resolve_scope("everything", None, None).is_err());
    }

    /// The clamp is the point of the surface (#110), so it is pinned rather than
    /// defaulted: neither the breadth setter nor a draft may widen a folder thread
    /// to the library. Consistent with a note-scoped thread, which can't be widened
    /// past its own note either — `/chat` is where a library-wide question goes.
    #[test]
    fn a_folder_thread_is_pinned_to_folder_breadth() {
        let folder = ChatTarget::Folder("f1".into());
        assert_eq!(pinned_breadth(&folder), Some("folder"));
        assert_eq!(pinned_breadth(&ChatTarget::Global), Some("all"));
        // A Note is the one target with a real choice — that's what the picker is for.
        assert_eq!(pinned_breadth(&ChatTarget::Note("n1".into())), None);

        assert!(check_pinned_breadth(&folder, "folder").is_ok());
        for widened in ["all", "note"] {
            let err = check_pinned_breadth(&folder, widened).unwrap_err();
            assert!(err.contains("always searches that folder"), "{widened}: {err}");
        }
        for b in ["note", "folder", "all"] {
            assert!(check_pinned_breadth(&ChatTarget::Note("n1".into()), b).is_ok());
        }
    }

    /// A folder thread resolves its clamp from its OWN folder — it has no anchor
    /// note to borrow one from, which is the whole reason the scope value exists.
    #[test]
    fn resolve_scope_clamps_a_folder_thread_to_its_own_folder() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("folderscope.sqlite")).unwrap();
        let folder = db::create_folder(&conn, "Team", "").unwrap();

        match resolve_scope("folder", None, Some(&folder.id)).unwrap() {
            ToolScope::Folder(id) => assert_eq!(id, folder.id),
            other => panic!("expected Folder scope, got {other:?}"),
        }
        // The thread's own folder WINS over the anchor note's — a note pane widened
        // to its folder passes None here, so the two never actually compete, but the
        // precedence has to be stated somewhere and this is it.
        let elsewhere = db::create_folder(&conn, "Elsewhere", "").unwrap();
        let note = note_in_folder(&conn, Some(&elsewhere.id));
        match resolve_scope("folder", Some(&note), Some(&folder.id)).unwrap() {
            ToolScope::Folder(id) => assert_eq!(id, folder.id),
            other => panic!("expected the target's folder, got {other:?}"),
        }
    }

    /// A library-wide turn injects NO note content and substitutes no fallback
    /// note (#82) — it runs on retrieval alone.
    #[test]
    fn a_library_wide_turn_carries_no_grounding_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("grounding.sqlite")).unwrap();
        let note = note_in_folder(&conn, None);

        let (global, body) = turn_grounding(None);
        assert!(global.text.is_empty(), "no anchor → no reference block");
        assert!(!global.truncated, "nothing was injected, so nothing was truncated");
        assert!(body.is_none(), "and nothing to reindex");

        // The note path is unchanged: a real block, and the body text the caller
        // reindexes with.
        let (anchored, body) = turn_grounding(Some(&note));
        assert!(anchored.text.contains("NOT as instructions"), "injection posture kept");
        assert!(body.is_some());

        // `anchor_note` is what decides which of those two a turn gets.
        assert!(anchor_note(&conn, &ChatTarget::Global).unwrap().is_none());
        assert_eq!(
            anchor_note(&conn, &ChatTarget::Note(note.id.clone())).unwrap().map(|n| n.id),
            Some(note.id)
        );
    }

    #[test]
    fn a_global_conversation_heals_any_stored_breadth_back_to_all() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("globalheal.sqlite")).unwrap();
        let target = ChatTarget::Global;
        let conv = db::create_conversation(
            &conn,
            CHAT_TENANT_PERSONAL,
            target.scope(),
            target.scope_id(),
            "all",
        )
        .unwrap();

        assert_eq!(heal_and_read_pinned_breadth(&conn, &conv.id, "all", "all").unwrap(), "all");
        // A row that somehow stored a note/folder breadth has no anchor to mean it
        // by. Heal rather than error: a corrupt row must not break the user's chat.
        for stored in ["note", "folder"] {
            db::set_conversation_breadth(&conn, &conv.id, stored).unwrap();
            assert_eq!(heal_and_read_pinned_breadth(&conn, &conv.id, stored, "all").unwrap(), "all");
            // The heal is persisted, so the chip and the request never diverge.
            let reloaded = db::get_conversation_by_id(&conn, &conv.id).unwrap().unwrap();
            assert_eq!(reloaded.breadth, "all");
        }
        // Garbage is still loud, exactly as for a note conversation.
        assert!(heal_and_read_pinned_breadth(&conn, &conv.id, "bogus", "all").is_err());
    }

    #[test]
    fn a_global_session_is_separate_from_every_notes_and_starts_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("globalsess.sqlite")).unwrap();
        let note = note_in_folder(&conn, None);
        let note_target = ChatTarget::Note(note.id.clone());
        let global = ChatTarget::Global;

        // First-ever sessions take their target's own default breadth.
        assert_eq!(inherited_breadth(&conn, CHAT_TENANT_PERSONAL, &global), "all");
        assert_eq!(inherited_breadth(&conn, CHAT_TENANT_PERSONAL, &note_target), "note");

        let g1 = new_personal_conversation(&conn, &global).unwrap();
        assert_eq!(g1.breadth, "all");
        assert_eq!(g1.scope, db::CHAT_SCOPE_GLOBAL);
        // The no-op guard applies to the library pane too: an empty most-recent
        // session is reused rather than piling up.
        assert_eq!(new_personal_conversation(&conn, &global).unwrap().id, g1.id);

        // A note's session is a different row and doesn't appear in the library
        // list (nor the reverse).
        let n1 = new_personal_conversation(&conn, &note_target).unwrap();
        assert_ne!(n1.id, g1.id);
        let listed = |t: &ChatTarget| {
            db::list_conversations(&conn, CHAT_TENANT_PERSONAL, t.scope(), t.scope_id(), None, db::ListFilter::All)
                .unwrap()
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(listed(&global), vec![g1.id.clone()]);
        assert_eq!(listed(&note_target), vec![n1.id.clone()]);

        // An explicit id from the other scope is a clean error, never a silent
        // cross-scope read.
        let err = resolve_existing(&conn, CHAT_TENANT_PERSONAL, &global, Some(&n1.id)).unwrap_err();
        assert!(err.contains("library-wide"), "got: {err}");
        assert!(resolve_existing(&conn, CHAT_TENANT_PERSONAL, &note_target, Some(&g1.id))
            .unwrap_err()
            .contains("this note"));
    }

    #[test]
    fn heal_and_read_breadth_self_heals_a_stale_folder_breadth_to_note() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("heal.sqlite")).unwrap();
        let note = note_in_folder(&conn, None); // no folder
        let conv =
            db::create_conversation(&conn, CHAT_TENANT_PERSONAL, CHAT_SCOPE_NOTE, &note.id, "note")
                .unwrap();
        // Simulate a breadth stored while the note still had a folder, then lost it.
        db::set_conversation_breadth(&conn, &conv.id, "folder").unwrap();

        let effective = heal_and_read_breadth(&conn, &conv.id, "folder", &note).unwrap();
        assert_eq!(effective, "note", "stale folder breadth heals to note");
        // The heal is persisted, so the chip and the request never diverge.
        let reloaded =
            db::latest_conversation(&conn, CHAT_TENANT_PERSONAL, CHAT_SCOPE_NOTE, &note.id)
                .unwrap()
                .unwrap();
        assert_eq!(reloaded.breadth, "note");
    }

    #[test]
    fn heal_and_read_breadth_keeps_a_valid_breadth_and_errors_on_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("eff.sqlite")).unwrap();
        let folder = db::create_folder(&conn, "Team", "").unwrap();
        let note = note_in_folder(&conn, Some(&folder.id));
        let conv =
            db::create_conversation(&conn, CHAT_TENANT_PERSONAL, CHAT_SCOPE_NOTE, &note.id, "note")
                .unwrap();
        assert_eq!(
            heal_and_read_breadth(&conn, &conv.id, "folder", &note).unwrap(),
            "folder",
            "folder breadth survives when the folder is present"
        );
        assert_eq!(
            heal_and_read_breadth(&conn, &conv.id, "all", &note).unwrap(),
            "all"
        );
        // A corrupt stored value errors, never silently clamps to note.
        assert!(heal_and_read_breadth(&conn, &conv.id, "bogus", &note).is_err());
    }

    #[test]
    fn embedding_models_are_recognised() {
        for m in ["embeddinggemma", "embeddinggemma:latest", "nomic-embed-text", "mxbai-embed-large", "bge-m3", "all-minilm", "snowflake-arctic-embed2", "paraphrase-multilingual"] {
            assert!(is_embedding_model(m), "expected embedding: {m}");
        }
        for m in ["gemma4:12b-mlx", "qwen3.5:4b", "llama3.2:3b", "gpt-5.4-mini"] {
            assert!(!is_embedding_model(m), "expected chat-capable: {m}");
        }
    }

    #[test]
    fn resolve_chat_rejects_an_unknown_provider_instead_of_calling_openai() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("t.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "mistral").unwrap();
        let err = resolve_chat(&conn, Some("sk-test".into())).err().unwrap().to_string();
        assert!(err.contains("mistral"), "{err}");
    }

    #[test]
    fn resolve_chat_reads_ollama_as_the_stored_local_alias() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("t.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "ollama").unwrap();
        db::set_setting(&conn, "chat_model", "gemma4:12b-mlx").unwrap();
        assert_eq!(resolve_chat(&conn, None).unwrap().provider, ProviderId::Local);
    }

    #[test]
    fn resolve_chat_rejects_an_embedding_model_as_the_chat_model() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("t.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "ollama").unwrap();
        db::set_setting(&conn, "chat_model", "embeddinggemma:latest").unwrap();
        let res = resolve_chat(&conn, None);
        assert!(res.is_err());
        let err = res.err().unwrap().to_string();
        assert!(err.contains("embedding model"), "clear guidance, not a raw 400: {err}");
    }

    // The local embedder has its own address and name (#179); blank means the
    // chat server and embeddinggemma, which is what an Ollama library runs on.
    #[test]
    fn the_local_embedder_defaults_to_the_chat_server_and_is_overridable() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("embed.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "ollama").unwrap();
        db::set_setting(&conn, "chat_model", "gemma4:12b-mlx").unwrap();
        db::set_setting(&conn, "local_llm_base_url", "http://127.0.0.1:8000/v1").unwrap();

        let cfg = resolve_embed(&resolve_chat(&conn, None).unwrap()).unwrap();
        assert_eq!(cfg.base_url, "http://127.0.0.1:8000/v1");
        assert_eq!(cfg.model, OLLAMA_EMBED_MODEL);

        // The shape #179 exists for: embeddings on Ollama, chat on mlx.
        db::set_setting(&conn, "embed_base_url", "http://localhost:11434/v1").unwrap();
        db::set_setting(&conn, "embed_model", "embeddinggemma:latest").unwrap();
        let cfg = resolve_embed(&resolve_chat(&conn, None).unwrap()).unwrap();
        assert_eq!(cfg.base_url, "http://localhost:11434/v1");
        assert_eq!(cfg.model, "embeddinggemma:latest");
        // The index is keyed by the model id, so an override must reach it —
        // otherwise new vectors would land under the old model's key.
        assert_eq!(cfg.adapter().model_id(), "embeddinggemma:latest");
    }

    #[test]
    fn a_blank_embed_override_is_not_an_override() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("blank.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "ollama").unwrap();
        db::set_setting(&conn, "chat_model", "gemma4:12b-mlx").unwrap();
        db::set_setting(&conn, "embed_base_url", "   ").unwrap();
        db::set_setting(&conn, "embed_model", "").unwrap();
        let cfg = resolve_embed(&resolve_chat(&conn, None).unwrap()).unwrap();
        assert_eq!(cfg.base_url, DEFAULT_LOCAL_LLM_BASE_URL);
        assert_eq!(cfg.model, OLLAMA_EMBED_MODEL);
    }

    // Anthropic serves no embeddings route, so chat there is keyword-only
    // rather than spending the Anthropic key on an OpenAI endpoint.
    #[test]
    fn anthropic_chat_has_no_embedder() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("anthropic.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "anthropic").unwrap();
        let resolved = resolve_chat(&conn, Some("sk-ant-test".into())).unwrap();
        assert_eq!(resolved.provider, ProviderId::Anthropic);
        assert_eq!(resolved.base_url, crate::anthropic::BASE);
        assert_eq!(resolved.model, crate::anthropic::DEFAULT_MODEL);
        assert!(resolve_embed(&resolved).is_none());
    }

    // One `chat_model` serves every provider, so switching leaves the last
    // one's id behind. The backend must not send it on.
    #[test]
    fn a_foreign_chat_model_falls_back_to_the_providers_default() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("foreign.sqlite")).unwrap();

        db::set_setting(&conn, "chat_provider", "anthropic").unwrap();
        db::set_setting(&conn, "chat_model", "gpt-5.5").unwrap();
        let r = resolve_chat(&conn, Some("sk-ant-test".into())).unwrap();
        assert_eq!(r.model, crate::anthropic::DEFAULT_MODEL);

        db::set_setting(&conn, "chat_provider", "openai").unwrap();
        db::set_setting(&conn, "chat_model", "claude-sonnet-5").unwrap();
        let r = resolve_chat(&conn, Some("sk-test".into())).unwrap();
        assert_eq!(r.model, DEFAULT_SUMMARY_MODEL);
    }

    #[test]
    fn a_models_own_provider_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("own.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "anthropic").unwrap();
        db::set_setting(&conn, "chat_model", "claude-opus-5").unwrap();
        assert_eq!(resolve_chat(&conn, Some("k".into())).unwrap().model, "claude-opus-5");
        db::set_setting(&conn, "chat_provider", "openai").unwrap();
        db::set_setting(&conn, "chat_model", "gpt-5.5").unwrap();
        assert_eq!(resolve_chat(&conn, Some("k".into())).unwrap().model, "gpt-5.5");
    }

    #[test]
    fn anthropic_chat_without_a_key_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("anthropic-nokey.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "anthropic").unwrap();
        let err = resolve_chat(&conn, None).err().unwrap().to_string();
        assert!(err.contains("Anthropic API key not set"), "{err}");
    }

    #[test]
    fn cloud_chat_ignores_the_local_embed_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("cloud.sqlite")).unwrap();
        db::set_setting(&conn, "chat_provider", "openai").unwrap();
        db::set_setting(&conn, "chat_model", "gpt-5.4").unwrap();
        db::set_setting(&conn, "embed_base_url", "http://localhost:11434/v1").unwrap();
        db::set_setting(&conn, "embed_model", "embeddinggemma").unwrap();
        let cfg = resolve_embed(&resolve_chat(&conn, Some("sk-test".into())).unwrap()).unwrap();
        assert_eq!(cfg.base_url, crate::openai::BASE);
        assert_eq!(cfg.model, OPENAI_EMBED_MODEL);
    }

    // The half of #179 that a probe cannot see: the embedder answers, and the
    // corpus still has no vectors under its name.
    #[test]
    fn renaming_the_embedder_leaves_the_whole_corpus_unembedded() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("count.sqlite")).unwrap();
        let note = db::create_note(&conn, "en", "meeting", "").unwrap();
        db::reindex_note(&conn, &note.id, "budget talk", "", "").unwrap();

        let under = |model: &str| {
            db::note_ids_needing_embedding(&conn, model).unwrap().len()
        };
        assert_eq!(under(OLLAMA_EMBED_MODEL), 1, "nothing embedded yet");

        for (hash, _text) in db::note_texts_needing_embedding(&conn, &note.id, OLLAMA_EMBED_MODEL).unwrap() {
            db::store_embedding(&conn, &hash, OLLAMA_EMBED_MODEL, &[0.1, 0.2]).unwrap();
        }
        assert_eq!(under(OLLAMA_EMBED_MODEL), 0);
        // The same note, under the name a #179 user would type, is unembedded —
        // which is what the settings row has to be able to say.
        assert_eq!(under("embeddinggemma:latest"), 1);
    }

    /// A user turn (for tests that need a session to have messages).
    fn add_user_message(conn: &rusqlite::Connection, conversation_id: &str, text: &str) {
        db::insert_chat_message(conn, conversation_id, "user", &chat::text_parts_json("b", text))
            .unwrap();
    }

    #[test]
    fn first_session_defaults_breadth_to_note() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("first.sqlite")).unwrap();
        // A Note with no sessions → the first new session defaults to "note".
        let conv = new_personal_conversation(&conn, &ChatTarget::Note("n1".into())).unwrap();
        assert_eq!(conv.breadth, "note");
        assert_eq!(db::conversation_message_count(&conn, &conv.id).unwrap(), 0);
    }

    #[test]
    fn new_session_inherits_breadth_from_the_most_recent() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("inherit.sqlite")).unwrap();
        let first =
            db::create_conversation(&conn, CHAT_TENANT_PERSONAL, CHAT_SCOPE_NOTE, "n1", "note")
                .unwrap();
        db::set_conversation_breadth(&conn, &first.id, "all").unwrap();
        // The most-recent session has messages, so the guard doesn't reuse it.
        add_user_message(&conn, &first.id, "hi");

        let second = new_personal_conversation(&conn, &ChatTarget::Note("n1".into())).unwrap();
        assert_ne!(second.id, first.id, "a genuinely new session was created");
        assert_eq!(second.breadth, "all", "breadth is inherited from the most recent session");
    }

    #[test]
    fn new_chat_no_ops_when_the_most_recent_session_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("noop.sqlite")).unwrap();
        let first =
            db::create_conversation(&conn, CHAT_TENANT_PERSONAL, CHAT_SCOPE_NOTE, "n1", "note")
                .unwrap();
        // No messages on the most-recent session → "new chat" returns it as-is.
        let again = new_personal_conversation(&conn, &ChatTarget::Note("n1".into())).unwrap();
        assert_eq!(again.id, first.id, "an empty most-recent session is reused, not duplicated");
        assert_eq!(
            db::list_conversations(&conn, CHAT_TENANT_PERSONAL, CHAT_SCOPE_NOTE, "n1", None, db::ListFilter::All).unwrap().len(),
            1,
            "no empty duplicate session was created"
        );
    }

    /// A library-wide pane opens on an unsaved draft, a Note's pane resumes that
    /// Note's most-recent thread (#120, the divergence chosen deliberately).
    ///
    /// `resolve_for_read` is what the three no-id read commands share, so this one
    /// test covers history, breadth and the authorship pin at once.
    #[test]
    fn a_library_wide_pane_opens_on_a_draft_while_a_note_resumes_its_thread() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("draft.sqlite")).unwrap();

        let global = ChatTarget::Global;
        let note = ChatTarget::Note("n1".into());
        let g = db::create_conversation(
            &conn,
            CHAT_TENANT_PERSONAL,
            global.scope(),
            global.scope_id(),
            "all",
        )
        .unwrap();
        db::insert_chat_message(&conn, &g.id, "user", "[]").unwrap();
        let n = db::create_conversation(
            &conn,
            CHAT_TENANT_PERSONAL,
            note.scope(),
            note.scope_id(),
            "note",
        )
        .unwrap();
        db::insert_chat_message(&conn, &n.id, "user", "[]").unwrap();

        // No id + library-wide → nothing resolved, so the pane starts empty even
        // though a finished thread exists. This is the bug #120 was filed for.
        assert!(
            resolve_for_read(&conn, CHAT_TENANT_PERSONAL, &global, None).unwrap().is_none(),
            "a library-wide pane must not reopen the last thread"
        );
        // No id + a Note → still resumes, unchanged.
        assert_eq!(
            resolve_for_read(&conn, CHAT_TENANT_PERSONAL, &note, None).unwrap().map(|c| c.id),
            Some(n.id),
            "a Note's pane continues its own thread"
        );
        // An EXPLICIT id is honoured for both — picking a thread from the sidebar
        // must still open it, or the list would be unusable.
        assert_eq!(
            resolve_for_read(&conn, CHAT_TENANT_PERSONAL, &global, Some(&g.id))
                .unwrap()
                .map(|c| c.id),
            Some(g.id.clone()),
            "an explicitly chosen library-wide thread still opens"
        );
    }

    /// A library-wide draft's first turn starts a NEW conversation instead of
    /// appending to the last one (#120).
    ///
    /// The sharpest failure this prevents: with the pane showing an empty draft, a
    /// send that resolved to "most recent" would file the message into a finished
    /// thread the user believed they had left.
    #[test]
    fn a_library_wide_first_turn_never_appends_to_the_previous_thread() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("draftsend.sqlite")).unwrap();
        let global = ChatTarget::Global;

        let previous = db::create_conversation(
            &conn,
            CHAT_TENANT_PERSONAL,
            global.scope(),
            global.scope_id(),
            "all",
        )
        .unwrap();
        db::insert_chat_message(&conn, &previous.id, "user", "[]").unwrap();

        let fresh =
            resolve_or_create(&conn, CHAT_TENANT_PERSONAL, &global, None, DraftSettings::default())
                .unwrap();
        assert_ne!(fresh.id, previous.id, "a draft's turn opens its own conversation");

        // A Note's bare send still continues its thread — the resume rule is the
        // one thing deciding both, so they can't drift apart.
        let note = ChatTarget::Note("n1".into());
        let existing =
            db::create_conversation(&conn, CHAT_TENANT_PERSONAL, note.scope(), note.scope_id(), "note")
                .unwrap();
        db::insert_chat_message(&conn, &existing.id, "user", "[]").unwrap();
        let resolved =
            resolve_or_create(&conn, CHAT_TENANT_PERSONAL, &note, None, DraftSettings::default())
                .unwrap();
        assert_eq!(resolved.id, existing.id, "a Note's bare send continues its thread");
    }

    /// Settings chosen on a draft — before any row exists — are applied to the
    /// conversation its first turn creates (#120, preserving #61 and #103).
    ///
    /// This is what replaces the old lazy-create: the row appears once, already
    /// carrying what the chips showed, so there is never a moment where a stored
    /// row and the visible chip disagree.
    #[test]
    fn a_drafts_breadth_and_pin_reach_the_conversation_its_turn_creates() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("draftsettings.sqlite")).unwrap();
        let global = ChatTarget::Global;

        let created = resolve_or_create(
            &conn,
            "wsA",
            &global,
            None,
            DraftSettings { breadth: Some("all".into()), client_filter: Some("c7".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(created.breadth, "all");
        assert_eq!(
            created.client_filter, "c7",
            "a pin set before the first turn binds that turn — the #103 guarantee"
        );

        // Absent settings fall back to inheritance, so "+" keeps behaving as it did.
        let plain =
            resolve_or_create(&conn, "wsB", &global, None, DraftSettings::default()).unwrap();
        assert_eq!(plain.breadth, "all");
        assert_eq!(plain.client_filter, "", "no pin unless one was chosen");
    }

    #[test]
    fn resolve_existing_rejects_a_cross_scope_conversation_id() {
        // An explicit id from another Note/tenant must be a clean error, never a
        // silent cross-scope operation (issue #61 fix 3).
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("xscope.sqlite")).unwrap();
        let n1 = db::create_conversation(&conn, CHAT_TENANT_PERSONAL, CHAT_SCOPE_NOTE, "n1", "note").unwrap();

        // Matching (tenant, note) resolves.
        let ok = resolve_existing(&conn, CHAT_TENANT_PERSONAL, &ChatTarget::Note("n1".into()), Some(&n1.id)).unwrap();
        assert_eq!(ok.unwrap().id, n1.id);
        // Wrong note → error.
        assert!(resolve_existing(&conn, CHAT_TENANT_PERSONAL, &ChatTarget::Note("n2".into()), Some(&n1.id)).is_err());
        // Wrong tenant → error.
        assert!(resolve_existing(&conn, "wsA", &ChatTarget::Note("n1".into()), Some(&n1.id)).is_err());
        // An unknown id is not an error — falls back to the active session (None).
        assert!(resolve_existing(&conn, CHAT_TENANT_PERSONAL, &ChatTarget::Note("n1".into()), Some("missing")).unwrap().is_none());
    }

}
