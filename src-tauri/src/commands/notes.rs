//! Note CRUD commands. Thin wrappers over `db::*`, surfaced to the
//! frontend via `commands::notes_*` (re-exported from the parent module).

use super::{err, DEFAULT_LANGUAGE};
use crate::db::{self, Note, NotePatch};
use crate::AppState;
use std::path::Path;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn notes_list(state: State<AppState>) -> Result<Vec<Note>, String> {
    let conn = state.db.lock();
    db::list_notes(&conn, "").map_err(err)
}

/// Every distinct speaker label in the library, with usage counters —
/// the suggestion source for the rename picker (#116 part 1).
///
/// Fetched once when the speaker strip mounts; the picker then filters and ranks
/// in TS per keystroke, so there is no IPC per keystroke.
#[tauri::command]
pub fn speaker_label_stats(state: State<AppState>) -> Result<Vec<db::SpeakerLabelStat>, String> {
    let conn = state.db.lock();
    db::speaker_label_stats(&conn, "").map_err(err)
}

#[tauri::command]
pub fn notes_get(state: State<AppState>, id: String) -> Result<Note, String> {
    let conn = state.db.lock();
    db::get_note(&conn, &id).map_err(err)
}

#[tauri::command]
pub fn notes_create(state: State<AppState>) -> Result<Note, String> {
    // New notes inherit the user's defaults for language + summary preset.
    // Both are overridable per-note from the note view; pre-feature notes
    // (empty language) fall back at transcribe / summary time.
    let note = {
        let conn = state.db.lock();
        let default_language = db::get_setting(&conn, "language")
            .map_err(err)?
            .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string());
        let default_preset = db::get_setting(&conn, "default_summary_preset")
            .map_err(err)?
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "meeting".to_string());
        db::create_note(&conn, &default_language, &default_preset, "").map_err(err)?
    };
    Ok(note)
}

#[tauri::command]
pub fn notes_update(state: State<AppState>, id: String, patch: NotePatch) -> Result<(), String> {
    {
        let conn = state.db.lock();
        db::update_note(&conn, &id, &patch).map_err(err)?;
    }
    Ok(())
}

#[tauri::command]
pub fn notes_delete(state: State<AppState>, id: String) -> Result<(), String> {
    {
        let conn = state.db.lock();
        db::delete_note(&conn, &id).map_err(err)?;
    }
    Ok(())
}

/// Notes in the Trash (soft-deleted, recoverable).
#[tauri::command]
pub fn notes_list_trash(state: State<AppState>) -> Result<Vec<Note>, String> {
    let conn = state.db.lock();
    db::list_trashed_notes(&conn, "").map_err(err)
}

/// Restore a note from the Trash.
#[tauri::command]
pub fn notes_restore(state: State<AppState>, id: String) -> Result<Note, String> {
    let note = {
        let conn = state.db.lock();
        db::restore_note(&conn, &id).map_err(err)?;
        db::get_note(&conn, &id).map_err(err)?
    };
    Ok(note)
}

/// Permanently delete a note from the Trash (hard delete, not recoverable).
///
/// Purge is the point of no return, so every file the note keeps on disk goes
/// with it ([`purge_note_assets`]). Soft delete deliberately does NOT touch these
/// — a Trash-restore must keep playback intact.
#[tauri::command]
pub fn notes_purge(app: AppHandle, state: State<AppState>, id: String) -> Result<(), String> {
    {
        let conn = state.db.lock();
        db::purge_note(&conn, &id).map_err(err)?;
    } // drop the db guard before touching the filesystem
    purge_note_assets_best_effort(&app, &id);
    Ok(())
}

/// For a note whose row is already gone: the purge has happened, so a failure to
/// remove its files is logged, not returned.
pub(crate) fn purge_note_assets_best_effort(app: &AppHandle, note_id: &str) {
    let Ok(base) = app.path().app_data_dir() else { return };
    if let Err(e) = purge_note_assets(&base, note_id) {
        eprintln!("purge: could not remove note {note_id}'s files: {e}");
    }
}

/// For the end of a chain that writes a note's files, after its last write: a
/// note purged while the chain ran must not get back what it wrote.
/// A trashed note keeps its row, and so its files.
pub(crate) fn purge_note_assets_if_gone(app: &AppHandle, note_id: &str) {
    let state = app.state::<AppState>();
    let gone = matches!(db::note_exists(&state.db.lock(), note_id), Ok(false));
    if gone {
        purge_note_assets_best_effort(app, note_id);
    }
}

/// Remove both directories a note keeps under the app data dir:
/// `recordings/<note_id>/` (retained audio, playback assets, timelines) and
/// `diagnostics/<note_id>/` (the diarize dumps, which hold the transcript text).
pub(crate) fn purge_note_assets(app_data_dir: &Path, note_id: &str) -> std::io::Result<()> {
    if !crate::sessions::is_safe_session_id(note_id) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "unsafe note id"));
    }
    let mut first_err = None;
    for root in ["recordings", "diagnostics"] {
        if let Err(e) = std::fs::remove_dir_all(app_data_dir.join(root).join(note_id)) {
            if e.kind() != std::io::ErrorKind::NotFound {
                first_err.get_or_insert(e);
            }
        }
    }
    first_err.map_or(Ok(()), Err)
}

/// Saved content revisions for a note, newest first (local version history).
#[tauri::command]
pub fn notes_revisions(state: State<AppState>, id: String) -> Result<Vec<db::NoteRevision>, String> {
    let conn = state.db.lock();
    db::list_note_revisions(&conn, &id).map_err(err)
}

/// Restore a note to a saved revision; the current state is snapshotted first so
/// it's undoable.
#[tauri::command]
pub fn notes_restore_revision(
    state: State<AppState>,
    id: String,
    revision_id: String,
) -> Result<Note, String> {
    let note = {
        let conn = state.db.lock();
        db::restore_note_revision(&conn, &id, &revision_id).map_err(err)?;
        db::get_note(&conn, &id).map_err(err)?
    };
    Ok(note)
}

#[tauri::command]
pub fn notes_move(
    state: State<AppState>,
    id: String,
    folder_id: Option<String>,
) -> Result<(), String> {
    {
        let conn = state.db.lock();
        db::move_note(&conn, &id, folder_id.as_deref()).map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::purge_note_assets;
    use std::fs;
    use std::io::ErrorKind;

    #[test]
    fn purge_removes_the_notes_recordings_and_diagnostics_dirs() {
        let base = tempfile::tempdir().unwrap();
        let note_id = "note-123";
        let dir = base.path().join("recordings").join(note_id);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("playback.wav"), b"fake").unwrap();
        fs::write(dir.join("mic-full.wav"), b"fake").unwrap();
        // The diarize dump holds every chunk's text: the transcript, in other words.
        let diag = base.path().join("diagnostics").join(note_id);
        fs::create_dir_all(&diag).unwrap();
        fs::write(diag.join("community1-mic.json"), br#"{"chunks":[{"text":"the price"}]}"#).unwrap();
        fs::write(diag.join("replay-timings.json"), b"{}").unwrap();
        let sibling = ["recordings", "diagnostics"].map(|root| base.path().join(root).join("note-456"));
        for d in &sibling {
            fs::create_dir_all(d).unwrap();
            fs::write(d.join("kept.json"), b"{}").unwrap();
        }

        purge_note_assets(base.path(), note_id).unwrap();

        assert!(!dir.exists(), "recordings/<note_id> should be gone after purge");
        assert!(!diag.exists(), "diagnostics/<note_id> should be gone after purge");
        for d in &sibling {
            assert!(d.join("kept.json").exists(), "a sibling note's {} must be untouched", d.display());
        }
    }

    #[test]
    fn purge_clears_every_root_before_reporting_a_failure() {
        let base = tempfile::tempdir().unwrap();
        // A plain file where the recordings dir should be, which remove_dir_all refuses.
        fs::create_dir_all(base.path().join("recordings")).unwrap();
        fs::write(base.path().join("recordings").join("note-123"), b"not a dir").unwrap();
        let diag = base.path().join("diagnostics").join("note-123");
        fs::create_dir_all(&diag).unwrap();
        fs::write(diag.join("community1-mic.json"), b"{}").unwrap();

        let e = purge_note_assets(base.path(), "note-123").unwrap_err();

        assert_ne!(e.kind(), ErrorKind::NotFound);
        assert!(!diag.exists(), "the transcript text goes even when the audio can't");
    }

    /// The id becomes a path segment under both roots. An empty id names each root
    /// itself and `..` the whole app data dir, so anything but a plain segment is
    /// refused before a single directory is removed.
    #[test]
    fn purge_refuses_an_id_that_is_not_a_plain_path_segment() {
        let base = tempfile::tempdir().unwrap();
        let app_data = base.path().join("app");
        let kept = ["recordings", "diagnostics"].map(|root| app_data.join(root).join("note-456"));
        for d in &kept {
            fs::create_dir_all(d).unwrap();
        }
        let outside = base.path().join("outside");
        fs::create_dir_all(&outside).unwrap();

        for id in ["", ".", "..", "note-456/..", "../../outside"] {
            let e = purge_note_assets(&app_data, id).unwrap_err();
            assert_eq!(e.kind(), ErrorKind::InvalidInput, "{id:?}");
        }

        for d in &kept {
            assert!(d.exists(), "{} must survive", d.display());
        }
        assert!(outside.exists(), "nothing outside the app data dir is reachable");
    }

    #[test]
    fn purge_is_ok_when_no_assets_exist() {
        let base = tempfile::tempdir().unwrap();
        // No recordings/ or diagnostics/ dir at all — a note that was never recorded.
        purge_note_assets(base.path(), "never-recorded").unwrap();
        // Idempotent: purging again is still fine.
        purge_note_assets(base.path(), "never-recorded").unwrap();

        // A note can have diagnostics and no recordings dir; the missing one must
        // not stop the other going.
        let diag = base.path().join("diagnostics").join("diagnosed-only");
        fs::create_dir_all(&diag).unwrap();
        fs::write(diag.join("nemotron3-mic.json"), b"{}").unwrap();
        purge_note_assets(base.path(), "diagnosed-only").unwrap();
        assert!(!diag.exists());
    }
}
