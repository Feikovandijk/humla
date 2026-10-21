//! Sync notes to Anytype: one explicit push per press, never a pull, never a
//! timer (ADR 0007). The note is rendered to Markdown ([`doc`]) and created
//! or updated as one object in one space through Anytype's local API
//! ([`client`]); the mapping lives in `anytype_links` so the next push
//! updates the same object instead of duplicating it.
//!
//! The destination is the user's own knowledge base, not an arbitrary
//! receiver, so the shape differs from the webhook's: the space is chosen in
//! Settings rather than named per send, the API key lives in the Keychain
//! under [`ACCOUNT`], and only the on/off switch is a plain setting, so the
//! note menu can ask without touching the Keychain.
//!
//! Every push goes through one paced [`Queue`] — per-note from the ⋯ menu,
//! or all of them from Settings — because Anytype rate limits its writes
//! (~1 rps sustained). Progress rides two events (`anytype_sync` per note,
//! `anytype_bulk` for a run), never `recording_status`: that channel belongs
//! to live capture alone.

pub mod client;
pub mod doc;

use crate::commands::err;
use crate::{db, keychain, sessions, AppState};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

/// Keychain account for the API key, created by the user in Anytype →
/// Settings → API Keys. Write-only from the webview, like every other key.
pub const ACCOUNT: &str = "anytype_api_key";
pub const ENABLED: &str = "anytype_enabled";
pub const SPACE_ID: &str = "anytype_space_id";
const SPACE_NAME: &str = "anytype_space_name";
const TYPE_KEY: &str = "anytype_type_key";
const BASE_URL: &str = "anytype_base_url";
pub const DEFAULT_TYPE_KEY: &str = "page";
pub const SETTINGS: &str = "Settings → General → Integrations";

/// How far apart two requests sit. The API's sustained limit is 1 rps; this
/// keeps a bulk run under it without a token bucket.
const PACE: std::time::Duration = std::time::Duration::from_millis(1100);

// --- Configuration ---------------------------------------------------------

/// What Settings shows: everything except the key, which it only learns is set.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub enabled: bool,
    pub has_key: bool,
    pub space_id: String,
    pub space_name: String,
    pub type_key: String,
    pub base_url: String,
}

/// What Settings saves. A `None` field keeps its stored value; an empty
/// `api_key` removes the key, an empty `base_url` resets the default.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInput {
    pub api_key: Option<String>,
    pub space_id: Option<String>,
    pub space_name: Option<String>,
    pub type_key: Option<String>,
    pub base_url: Option<String>,
}

pub fn config_view(state: &AppState) -> Result<ConfigView, String> {
    let (enabled, space_id, space_name, type_key, base_url) = {
        let conn = state.db.lock();
        let get = |key: &str| db::get_setting(&conn, key).map_err(err);
        (
            get(ENABLED)?.as_deref() == Some("true"),
            get(SPACE_ID)?.unwrap_or_default(),
            get(SPACE_NAME)?.unwrap_or_default(),
            get(TYPE_KEY)?.unwrap_or_default(),
            get(BASE_URL)?.unwrap_or_default(),
        )
    };
    // No Keychain read until the feature is on, so a user who never turns it
    // on is never asked for Keychain access on its behalf.
    let has_key = enabled && keychain::read(&state.keychain_cache, ACCOUNT)?.is_some();
    Ok(ConfigView {
        enabled,
        has_key,
        space_id,
        space_name,
        type_key,
        base_url,
    })
}

pub fn apply_config(state: &AppState, input: ConfigInput) -> Result<(), String> {
    if let Some(url) = &input.base_url {
        if !url.trim().is_empty() {
            client::validate_base_url(url)?;
        }
    }
    {
        let conn = state.db.lock();
        let set = |key: &str, value: &str| db::set_setting(&conn, key, value).map_err(err);
        if let Some(v) = &input.space_id {
            set(SPACE_ID, v.trim())?;
        }
        if let Some(v) = &input.space_name {
            set(SPACE_NAME, v.trim())?;
        }
        if let Some(v) = &input.type_key {
            set(TYPE_KEY, v.trim())?;
        }
        if let Some(v) = &input.base_url {
            set(BASE_URL, v.trim())?;
        }
    }
    if let Some(key) = &input.api_key {
        keychain::write(&state.keychain_cache, ACCOUNT, Some(key))?;
    }
    Ok(())
}

/// The saved base URL, or the default — without asking for a key or a space,
/// so a connection check can run before either exists.
pub fn base_url_or_default(state: &AppState) -> Result<String, String> {
    let conn = state.db.lock();
    let url = db::get_setting(&conn, BASE_URL)
        .map_err(err)?
        .unwrap_or_default();
    Ok(if url.trim().is_empty() {
        client::DEFAULT_BASE_URL.to_string()
    } else {
        url
    })
}

/// The values a push needs, or the sentence that tells the user which one is
/// missing. An empty `type_key` falls back to `page`.
pub(crate) struct SyncConfig {
    pub(crate) key: String,
    pub(crate) space_id: String,
    pub(crate) type_key: String,
    pub(crate) base_url: String,
}

pub(crate) fn read_sync_config(state: &AppState) -> Result<SyncConfig, String> {
    let (space_id, type_key, base_url) = {
        let conn = state.db.lock();
        let get = |key: &str| db::get_setting(&conn, key).map_err(err);
        if get(ENABLED)?.as_deref() != Some("true") {
            return Err(format!("Turn on Anytype sync in {SETTINGS} first."));
        }
        (
            get(SPACE_ID)?.unwrap_or_default(),
            get(TYPE_KEY)?.unwrap_or_default(),
            get(BASE_URL)?.unwrap_or_default(),
        )
    };
    let key = keychain::read(&state.keychain_cache, ACCOUNT)?
        .ok_or_else(|| format!("Add your Anytype API key in {SETTINGS} first."))?;
    if space_id.is_empty() {
        return Err(format!("Choose an Anytype space in {SETTINGS} first."));
    }
    Ok(SyncConfig {
        key,
        space_id,
        type_key: if type_key.is_empty() {
            DEFAULT_TYPE_KEY.to_string()
        } else {
            type_key
        },
        base_url: if base_url.is_empty() {
            client::DEFAULT_BASE_URL.to_string()
        } else {
            base_url
        },
    })
}

// --- Events -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncEvent {
    pub note_id: String,
    /// "syncing" while the push runs, then the outcome: "synced", "unchanged"
    /// or "failed" (with `message`).
    pub state: String,
    pub message: Option<String>,
    pub recreated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkEvent {
    pub done: u32,
    pub total: u32,
    pub finished: bool,
    pub cancelled: bool,
}

/// What one push settled as. `Unchanged` means the fingerprint matched, so
/// Anytype already holds exactly this note.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncOutcome {
    Synced { recreated: bool },
    Unchanged,
}

// --- The queue ----------------------------------------------------------------

type SyncResult = Result<SyncOutcome, String>;

struct Job {
    note_id: String,
    confirmed: Option<bool>,
    result: Option<oneshot::Sender<SyncResult>>,
    bulk: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkProgress {
    pub done: u32,
    pub total: u32,
    pub cancelled: bool,
}

struct QueueState {
    /// Note ids waiting or in flight, so a second press doesn't queue a
    /// second push for the same note.
    queued: HashSet<String>,
    bulk: Option<BulkProgress>,
}

pub struct Queue {
    tx: tokio::sync::mpsc::UnboundedSender<Job>,
    rx: std::sync::Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Job>>>,
    state: Arc<parking_lot::Mutex<QueueState>>,
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

impl Queue {
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Queue {
            tx,
            rx: std::sync::Mutex::new(Some(rx)),
            state: Arc::new(parking_lot::Mutex::new(QueueState {
                queued: HashSet::new(),
                bulk: None,
            })),
        }
    }

    /// Spawn the worker. Once, from `setup`, on Tauri's runtime.
    pub fn start(&self, app: &AppHandle) {
        let rx = self
            .rx
            .lock()
            .expect("queue mutex")
            .take()
            .expect("the worker starts exactly once");
        let state = self.state.clone();
        let app = app.clone();
        tauri::async_runtime::spawn(async move { worker(app, rx, state).await });
    }

    /// Queue one note for a push and hand back a receiver for its outcome,
    /// or `None` when it is already queued. `confirmed` records the first-push
    /// consent the modal collects.
    pub fn enqueue(
        &self,
        note_id: &str,
        confirmed: Option<bool>,
        bulk: bool,
    ) -> Option<oneshot::Receiver<SyncResult>> {
        enqueue_locked(&mut self.state.lock(), &self.tx, note_id, confirmed, bulk)
    }

    /// Queue every note in `ids` for a bulk run and start counting progress.
    /// Returns how many notes this run will actually push: a note already
    /// queued by a manual press is skipped, and counting it would stick the
    /// progress at `total - 1` forever, since its push reports on the
    /// per-note channel, not the bulk one. Bulk pushes are explicit consent
    /// (the user pressed "Send all"), so each one confirms its note.
    ///
    /// A run already in flight is left alone and reports 0 — the Settings
    /// button is hidden while one runs, and two counters over one queue
    /// would corrupt each other.
    pub fn start_bulk(&self, ids: Vec<String>) -> u32 {
        let mut guard = self.state.lock();
        if guard.bulk.is_some() {
            return 0;
        }
        let mut total = 0u32;
        for id in &ids {
            // enqueue_locked sends while the lock is held. The send is
            // non-blocking, and this ordering keeps counter and queue
            // consistent: no bulk job exists before the bulk counter does
            // (the worker can't observe the channel until we release).
            if enqueue_locked(&mut guard, &self.tx, id, Some(true), true).is_some() {
                total += 1;
            }
        }
        // A zero-note run never reports — there is nothing to progress
        // towards, and no final event would ever fire.
        if total > 0 {
            guard.bulk = Some(BulkProgress {
                done: 0,
                total,
                cancelled: false,
            });
        }
        total
    }

    pub fn cancel_bulk(&self, app: &AppHandle) {
        let cancelled = self.state.lock().bulk.take();
        if let Some(b) = cancelled {
            let _ = app.emit(
                "anytype_bulk",
                BulkEvent {
                    done: b.done,
                    total: b.total,
                    finished: true,
                    cancelled: true,
                },
            );
        }
    }

    pub fn bulk_progress(&self) -> Option<BulkProgress> {
        self.state.lock().bulk
    }

    pub fn is_queued(&self, note_id: &str) -> bool {
        self.state.lock().queued.contains(note_id)
    }
}

/// Insert `note_id` into the queue and send its job, or return `None` when
/// it is already queued. The caller holds the state lock: an unbounded
/// channel's send never blocks, so holding it across the send can't deadlock,
/// and it keeps `queued` and the channel in step.
fn enqueue_locked(
    guard: &mut QueueState,
    tx: &tokio::sync::mpsc::UnboundedSender<Job>,
    note_id: &str,
    confirmed: Option<bool>,
    bulk: bool,
) -> Option<oneshot::Receiver<SyncResult>> {
    if !guard.queued.insert(note_id.to_string()) {
        return None;
    }
    let (result_tx, result_rx) = oneshot::channel();
    tx.send(Job {
        note_id: note_id.to_string(),
        confirmed,
        result: Some(result_tx),
        bulk,
    })
    .expect("the worker outlives every enqueue");
    Some(result_rx)
}

async fn worker(
    app: AppHandle,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<Job>,
    state: Arc<parking_lot::Mutex<QueueState>>,
) {
    let mut paced = false;
    while let Some(job) = rx.recv().await {
        // A cancelled bulk's remaining notes drain quietly — a cancelled run
        // reports once, at cancel time, not once per skipped note. (When the
        // bulk state is gone entirely — cancelled — its queued jobs drain the
        // same way.)
        let cancelled = job.bulk && state.lock().bulk.as_ref().is_none_or(|b| b.cancelled);
        if cancelled {
            state.lock().queued.remove(&job.note_id);
            if let Some(result) = job.result {
                let _ = result.send(Err("The sync was stopped.".to_string()));
            }
            continue;
        }
        if paced {
            tokio::time::sleep(PACE).await;
        }
        paced = true;

        let _ = app.emit(
            "anytype_sync",
            SyncEvent {
                note_id: job.note_id.clone(),
                state: "syncing".to_string(),
                message: None,
                recreated: false,
            },
        );
        let outcome = sync_one(&app, &job.note_id, job.confirmed).await;
        if let Err(message) = &outcome {
            // Keep the failure on the link row, where the note's status can
            // show it until the next push clears it.
            let app_state = app.state::<AppState>();
            let conn = app_state.db.lock();
            let _ = db::anytype_link_set_error(&conn, &job.note_id, message);
        }
        let event = SyncEvent {
            note_id: job.note_id.clone(),
            state: match &outcome {
                Ok(SyncOutcome::Synced { .. }) => "synced".to_string(),
                Ok(SyncOutcome::Unchanged) => "unchanged".to_string(),
                Err(_) => "failed".to_string(),
            },
            message: outcome.as_ref().err().cloned(),
            recreated: matches!(outcome, Ok(SyncOutcome::Synced { recreated: true })),
        };
        let _ = app.emit("anytype_sync", event);
        if let Some(result) = job.result {
            let _ = result.send(outcome);
        }
        if job.bulk {
            // Progress per note; the run ends with `finished: true`, and a
            // zero-note run never starts (see `start_bulk`).
            let progress = {
                let mut guard = state.lock();
                guard.bulk.as_mut().map(|b| {
                    b.done += 1;
                    (b.done, b.total, b.done >= b.total)
                })
            };
            if let Some((done, total, finished)) = progress {
                if finished {
                    state.lock().bulk = None;
                }
                let _ = app.emit(
                    "anytype_bulk",
                    BulkEvent {
                        done,
                        total,
                        finished,
                        cancelled: false,
                    },
                );
            }
        }
        state.lock().queued.remove(&job.note_id);
    }
}

// --- The push -----------------------------------------------------------------

/// Push one note. Shared by the ⋯ menu and the bulk run, so both do the same
/// thing to the same object. Reads the note as the database holds it — the
/// view flushes its queued edits first, exactly as the webhook does.
pub async fn sync_one(app: &AppHandle, note_id: &str, confirmed: Option<bool>) -> SyncResult {
    let state = app.state::<AppState>();
    let cfg = read_sync_config(&state)?;

    let (note, link, folder, client_name) = {
        let conn = state.db.lock();
        let note = db::get_note(&conn, note_id).map_err(err)?;
        if note.deleted_at.is_some() {
            return Err("Restore this note before syncing it.".to_string());
        }
        if doc::is_empty(&note) {
            return Err("There is nothing in this note to sync yet.".to_string());
        }
        let link = db::anytype_link_get(&conn, note_id).map_err(err)?;
        let folder = note
            .folder_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(|id| db::folder_name(&conn, id))
            .transpose()
            .map_err(err)?
            .flatten();
        let client_name = note
            .client_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(|id| db::client_name(&conn, id))
            .transpose()
            .map_err(err)?
            .flatten();
        (note, link, folder, client_name)
    };

    // Nothing changed since the last push: same content version, same space.
    // Checked before any document work — this is the common case in a bulk run.
    if let Some(link) = &link {
        if link.synced_updated_at == note.updated_at && link.space_id == cfg.space_id {
            return Ok(SyncOutcome::Unchanged);
        }
    }

    let app_dir = app.path().app_data_dir().map_err(err)?;
    let takes: Vec<_> = sessions::resolve_sessions(&sessions::recordings_dir(&app_dir, note_id))
        .into_iter()
        .map(|(entry, _)| entry)
        .collect();
    let in_progress = state.recording.lock().note_id.as_deref() == Some(note_id);
    let facts = doc::Facts {
        folder,
        client: client_name,
        recording: recording_facts(&takes, in_progress),
    };

    let name = crate::commands::display_title(&note.title).to_string();
    let markdown = doc::build(&note, &facts);
    let hash = doc::hash(&cfg.space_id, &cfg.type_key, &name, &markdown);

    // The note moved (`updated_at` bumped) but the document didn't — an edit
    // that never reaches it, like a speaker-count hint. Re-mark and stop.
    if let Some(link) = &link {
        if link.doc_hash == hash && link.space_id == cfg.space_id {
            let conn = state.db.lock();
            db::anytype_link_upsert(&conn, &db::AnytypeLink {
                synced_updated_at: note.updated_at,
                ..link.clone()
            })
            .map_err(err)?;
            return Ok(SyncOutcome::Unchanged);
        }
    }

    let client = client::Client::new(&cfg.base_url, &cfg.key)?;
    let mut recreated = false;
    let object_id = match &link {
        Some(link) if !link.object_id.is_empty() && link.space_id == cfg.space_id => {
            match client
                .update_object(&cfg.space_id, &link.object_id, &name, &markdown)
                .await
            {
                Ok(()) => link.object_id.clone(),
                Err(client::ApiError::Gone) => {
                    // Deleted or archived in Anytype: create a fresh copy and
                    // say so — the user may not know it went.
                    recreated = true;
                    client
                        .create_object(&cfg.space_id, &cfg.type_key, &name, &markdown)
                        .await
                        .map_err(|e| match e {
                            client::ApiError::Gone => {
                                "The space no longer exists in Anytype. Choose it again in Settings.".to_string()
                            }
                            other => other.message(),
                        })?
                }
                Err(other) => return Err(other.message()),
            }
        }
        // Never pushed, or pushed to a space that is no longer the chosen one:
        // create in the current space. The old object, if any, stays where it
        // is — deleting it is the user's call.
        _ => client
            .create_object(&cfg.space_id, &cfg.type_key, &name, &markdown)
            .await
            .map_err(|e| match e {
                client::ApiError::Gone => {
                    "The space no longer exists in Anytype. Choose it again in Settings."
                        .to_string()
                }
                other => other.message(),
            })?,
    };

    {
        let conn = state.db.lock();
        // A note purged while its push ran doesn't get its mapping back —
        // the object exists in Anytype, but the note is gone, and the purge
        // is the point of no return for everything the note was.
        if !db::note_exists(&conn, note_id).map_err(err)? {
            return Err("The note was deleted while it was syncing.".to_string());
        }
        db::anytype_link_upsert(
            &conn,
            &db::AnytypeLink {
                note_id: note_id.to_string(),
                object_id,
                space_id: cfg.space_id.clone(),
                doc_hash: hash,
                synced_updated_at: note.updated_at,
                synced_at: db::now_ms(),
                confirmed: confirmed.unwrap_or_else(|| {
                    link.as_ref().map(|l| l.confirmed).unwrap_or(false)
                }),
                last_error: None,
            },
        )
        .map_err(err)?;
    }
    Ok(SyncOutcome::Synced { recreated })
}

/// What the manifest says about when the note was recorded, mirrored from the
/// webhook's rules: takes with index > 0 only (the "Earlier transcript"
/// repair session is not a take), and a total is `None` unless every take's
/// value is known — a guessed duration is worse than none.
fn recording_facts(takes: &[sessions::SessionEntry], in_progress: bool) -> doc::RecordingFacts {
    let takes: Vec<&sessions::SessionEntry> = takes.iter().filter(|e| e.index > 0).collect();
    if takes.is_empty() && !in_progress {
        return doc::RecordingFacts::default();
    }
    let started = takes
        .iter()
        .map(|e| DateTime::parse_from_rfc3339(&e.started_at).ok())
        .collect::<Option<Vec<_>>>()
        .and_then(|starts| starts.into_iter().min())
        .map(|at| at.with_timezone(&Utc));
    let duration_ms = if in_progress {
        None
    } else {
        takes
            .iter()
            .map(|e| (e.duration_ms > 0).then_some(e.duration_ms))
            .try_fold(0u64, |total, ms| total.checked_add(ms?))
    };
    doc::RecordingFacts {
        started_label: started.map(|at| {
            at.with_timezone(&chrono::Local)
                .format("%e %b %Y, %H:%M")
                .to_string()
                .trim_start()
                .to_string()
        }),
        duration_ms,
        take_count: takes.len(),
        in_progress,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_queues_once_and_a_bulk_counts_only_what_it_enqueued() {
        let queue = Queue::new();
        assert!(queue.enqueue("n1", None, false).is_some());
        assert!(
            queue.enqueue("n1", None, false).is_none(),
            "a second press doesn't queue a second push"
        );
        assert!(queue.is_queued("n1"));
        // A bulk run skips the already-queued note and counts only its own
        // jobs — the manual push reports on the per-note channel, so counting
        // it here would stick the progress at total - 1 forever.
        assert_eq!(queue.start_bulk(vec!["n1".into(), "n2".into()]), 1);
        let bulk = queue.bulk_progress().unwrap();
        assert_eq!((bulk.done, bulk.total), (0, 1));
        // A run already in flight is left alone.
        assert_eq!(queue.start_bulk(vec!["n3".into()]), 0);
        assert_eq!(queue.bulk_progress().unwrap().total, 1);
    }

    #[test]
    fn an_empty_bulk_never_reports() {
        let queue = Queue::new();
        assert_eq!(queue.start_bulk(vec![]), 0);
        assert!(queue.bulk_progress().is_none());
    }

    fn entry(v: serde_json::Value) -> sessions::SessionEntry {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn recording_facts_cover_every_take_without_counting_gaps() {
        let takes = vec![
            entry(serde_json::json!({ "id": "earlier", "index": 0 })),
            entry(serde_json::json!({
                "id": "t1", "index": 1,
                "startedAt": "2026-10-12T09:00:00Z", "durationMs": 60000,
            })),
            entry(serde_json::json!({
                "id": "t2", "index": 2,
                "startedAt": "2026-10-12T09:05:00Z", "durationMs": 120000,
            })),
        ];
        let facts = recording_facts(&takes, false);
        assert_eq!(facts.take_count, 2, "the repair session at index 0 is not a take");
        assert_eq!(facts.duration_ms, Some(180_000), "takes sum, gaps don't count");
        assert!(facts.started_label.is_some(), "the earliest start is known");
        assert!(!facts.in_progress);

        // A take with unknown timing nulls the totals but keeps the takes.
        let partial = vec![
            entry(serde_json::json!({ "id": "legacy", "index": 1 })),
            entry(serde_json::json!({
                "id": "t2", "index": 2,
                "startedAt": "2026-10-12T09:05:00Z", "durationMs": 120000,
            })),
        ];
        let facts = recording_facts(&partial, false);
        assert_eq!(facts.take_count, 2);
        assert_eq!(facts.duration_ms, None);
        assert!(facts.started_label.is_none());

        // No takes and not recording: nothing to say. A first take in flight
        // is still a recording.
        assert_eq!(recording_facts(&[], false).take_count, 0);
        assert!(recording_facts(&[], true).in_progress);
    }
}
