//! Client CRUD commands (issue #43), plus the note→client assignment. Thin
//! wrappers over `db::*`, surfaced via `commands::clients_*` / `notes_set_client`.

use super::err;
use crate::db;
use crate::AppState;
use tauri::State;

#[tauri::command]
pub fn clients_list(state: State<AppState>) -> Result<Vec<db::Client>, String> {
    let conn = state.db.lock();
    db::list_clients(&conn, "").map_err(err)
}

#[tauri::command]
pub fn clients_create(state: State<AppState>, name: String) -> Result<db::Client, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Client name cannot be empty".into());
    }
    let client = {
        let conn = state.db.lock();
        db::create_client(&conn, trimmed, "").map_err(err)?
    };
    Ok(client)
}

#[tauri::command]
pub fn clients_rename(state: State<AppState>, id: String, name: String) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Client name cannot be empty".into());
    }
    {
        let conn = state.db.lock();
        db::rename_client(&conn, &id, trimmed).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn clients_delete(state: State<AppState>, id: String) -> Result<(), String> {
    {
        let conn = state.db.lock();
        db::delete_client(&conn, &id).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn notes_set_client(
    state: State<AppState>,
    id: String,
    client_id: Option<String>,
) -> Result<(), String> {
    {
        let conn = state.db.lock();
        db::set_note_client(&conn, &id, client_id.as_deref()).map_err(err)?;
    }
    Ok(())
}
