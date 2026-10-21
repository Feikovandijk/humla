//! Anytype sync commands (ADR 0007): configuration, a connection check for
//! the Settings form, one push per ⋯-menu press, and a bulk run from
//! Settings. The push itself lives in `crate::anytype` so the queue worker
//! and these commands share one path.

use super::err;
use crate::anytype::{self, client, ConfigInput};
use crate::{db, AppState};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceView {
    id: String,
    name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeView {
    key: String,
    name: String,
}

/// Where one note stands with Anytype — read from the link row alone, no
/// network, so the ⋯ menu can ask cheaply.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusView {
    pub linked: bool,
    /// The user confirmed the first push for this note and doesn't want the
    /// confirmation again.
    pub confirmed: bool,
    /// The note moved (or the chosen space changed) since the last push.
    pub stale: bool,
    pub syncing: bool,
    pub synced_at: Option<i64>,
    pub last_error: Option<String>,
}

#[tauri::command]
pub fn anytype_config_get(state: State<AppState>) -> Result<anytype::ConfigView, String> {
    anytype::config_view(&state)
}

#[tauri::command]
pub fn anytype_config_set(state: State<AppState>, input: ConfigInput) -> Result<(), String> {
    anytype::apply_config(&state, input)
}

/// The Settings form's "Test connection": list spaces with the key the user
/// just typed (`api_key` given) or the saved one. Fails without storing
/// anything, so a typo never lands in the Keychain.
#[tauri::command]
pub async fn anytype_check(
    state: State<'_, AppState>,
    api_key: Option<String>,
    base_url: Option<String>,
) -> Result<Vec<SpaceView>, String> {
    let key = match api_key {
        Some(key) if !key.trim().is_empty() => key,
        _ => crate::keychain::read(&state.keychain_cache, anytype::ACCOUNT)?.ok_or_else(|| {
            format!("Add your Anytype API key in {} first.", anytype::SETTINGS)
        })?,
    };
    let base_url = match base_url {
        Some(url) if !url.trim().is_empty() => url,
        _ => anytype::base_url_or_default(&state)?,
    };
    let client = client::Client::new(&base_url, &key)?;
    client
        .list_spaces()
        .await
        .map(|spaces| {
            spaces
                .into_iter()
                .map(|s| SpaceView { id: s.id, name: s.name })
                .collect()
        })
        .map_err(|e| e.message())
}

/// The object types of the saved space, for the type picker.
#[tauri::command]
pub async fn anytype_list_types(state: State<'_, AppState>) -> Result<Vec<TypeView>, String> {
    let cfg = anytype::read_sync_config(&state)?;
    let client = client::Client::new(&cfg.base_url, &cfg.key)?;
    client
        .list_types(&cfg.space_id)
        .await
        .map(|types| {
            types
                .into_iter()
                .map(|t| TypeView { key: t.key, name: t.name })
                .collect()
        })
        .map_err(|e| e.message())
}

#[tauri::command]
pub fn anytype_status(
    app: AppHandle,
    state: State<AppState>,
    note_id: String,
) -> Result<StatusView, String> {
    let (note, link, space_id) = {
        let conn = state.db.lock();
        let note = db::get_note(&conn, &note_id).map_err(err)?;
        let link = db::anytype_link_get(&conn, &note_id).map_err(err)?;
        let space = db::get_setting(&conn, anytype::SPACE_ID)
            .map_err(err)?
            .unwrap_or_default();
        (note, link, space)
    };
    let syncing = app.state::<AppState>().anytype.is_queued(&note_id);
    Ok(match link {
        Some(link) => StatusView {
            linked: !link.object_id.is_empty(),
            confirmed: link.confirmed,
            stale: link.synced_updated_at != note.updated_at || link.space_id != space_id,
            syncing,
            synced_at: (link.synced_at > 0).then_some(link.synced_at),
            last_error: link.last_error,
        },
        None => StatusView {
            linked: false,
            confirmed: false,
            stale: false,
            syncing,
            synced_at: None,
            last_error: None,
        },
    })
}

/// Push one note. Awaits its outcome, so the caller's toast is the real
/// answer — not an "it's on its way".
#[tauri::command]
pub async fn anytype_sync_note(
    state: State<'_, AppState>,
    note_id: String,
    confirmed: Option<bool>,
) -> Result<anytype::SyncOutcome, String> {
    // Fail fast on configuration the user has to fix, rather than queueing a
    // push that errors a second later.
    anytype::read_sync_config(&state)?;
    let receiver = state
        .anytype
        .enqueue(&note_id, confirmed, false)
        .ok_or_else(|| "This note is already being synced.".to_string())?;
    receiver
        .await
        .map_err(|_| "The sync stopped before this note was done.".to_string())?
}

/// Queue every live, non-empty note for a push. Progress rides
/// `anytype_bulk` events; the count returns at once.
#[tauri::command]
pub fn anytype_sync_all(
    app: AppHandle,
    state: State<AppState>,
) -> Result<u32, String> {
    anytype::read_sync_config(&state)?;
    let ids: Vec<String> = {
        let conn = state.db.lock();
        db::list_notes(&conn, "")
            .map_err(err)?
            .into_iter()
            .filter(|note| note.deleted_at.is_none() && !anytype::doc::is_empty(note))
            .map(|note| note.id)
            .collect()
    };
    Ok(app.state::<AppState>().anytype.start_bulk(ids))
}

#[tauri::command]
pub fn anytype_sync_cancel(app: AppHandle) {
    app.state::<AppState>().anytype.cancel_bulk(&app);
}

/// While a bulk run is in flight, so Settings can pick its state back up
/// after the dialog closes and reopens.
#[tauri::command]
pub fn anytype_sync_state(state: State<AppState>) -> Option<anytype::BulkProgress> {
    state.anytype.bulk_progress()
}

/// Forget the mapping without touching the object in Anytype — "stop
/// tracking this note". The next push creates a new object.
#[tauri::command]
pub fn anytype_unlink_note(state: State<AppState>, note_id: String) -> Result<(), String> {
    let conn = state.db.lock();
    db::anytype_link_delete(&conn, &note_id).map_err(err)
}
