//! Explicit, one-shot note delivery. Credentials stay in Keychain; only the
//! opt-in flag lives in settings. No redirects or automatic delivery retries.
use super::err;
use crate::{db, html_text::html_to_text, sessions, AppState};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Url,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{Manager, State};

const ACCOUNT: &str = "webhook_config";

#[derive(Default, Serialize, Deserialize)]
pub struct WebhookHeader {
    name: String,
    value: String,
}

#[derive(Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    url: String,
    #[serde(default)]
    headers: Vec<WebhookHeader>,
}

fn credential(service: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(service, ACCOUNT)
        .map_err(|_| "Could not access webhook settings in Keychain.".into())
}

fn read_config(service: &str) -> Result<WebhookConfig, String> {
    match credential(service)?.get_password() {
        Ok(value) => serde_json::from_str(&value).map_err(|_| {
            "Stored webhook settings could not be read. Please save them again.".into()
        }),
        Err(keyring::Error::NoEntry) => Ok(WebhookConfig::default()),
        Err(_) => Err("Could not read webhook settings from Keychain.".into()),
    }
}

fn validate(config: &WebhookConfig) -> Result<(Url, HeaderMap), String> {
    let url =
        Url::parse(config.url.trim()).map_err(|_| "Enter a valid webhook URL.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("Webhook URL must start with https:// or http://.".into());
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("Use headers for authentication and remove any URL fragment.".into());
    }
    let mut headers = HeaderMap::new();
    for header in &config.headers {
        let name = HeaderName::from_bytes(header.name.trim().as_bytes())
            .map_err(|_| "Each header needs a valid name.".to_string())?;
        if matches!(
            name.as_str(),
            "host"
                | "content-length"
                | "content-type"
                | "connection"
                | "transfer-encoding"
                | "trailer"
                | "upgrade"
                | "expect"
        ) {
            return Err("Host, content type and transport headers are managed by Humla.".into());
        }
        let mut value = HeaderValue::from_str(&header.value).map_err(|_| {
            "Header values cannot contain line breaks or invalid characters.".to_string()
        })?;
        value.set_sensitive(true);
        if headers.insert(name, value).is_some() {
            return Err("Header names must be unique (ignoring capitalization).".into());
        }
    }
    Ok((url, headers))
}

#[tauri::command]
pub fn webhook_get_config(
    app: tauri::AppHandle,
    state: State<AppState>,
) -> Result<WebhookConfig, String> {
    let mut config = read_config(&app.config().identifier)?;
    config.enabled = db::get_setting(&state.db.lock(), "webhook_enabled")
        .map_err(err)?
        .as_deref()
        == Some("true");
    Ok(config)
}

#[tauri::command]
pub fn webhook_set_config(
    app: tauri::AppHandle,
    state: State<AppState>,
    mut config: WebhookConfig,
) -> Result<(), String> {
    config.url = config.url.trim().to_string();
    // Disabling must still work if an old endpoint is no longer valid.
    if config.enabled {
        validate(&config)?;
    }
    let value = serde_json::to_string(&config).map_err(err)?;
    credential(&app.config().identifier)?
        .set_password(&value)
        .map_err(|_| "Could not save webhook settings to Keychain.".to_string())?;
    db::set_setting(
        &state.db.lock(),
        "webhook_enabled",
        if config.enabled { "true" } else { "false" },
    )
    .map_err(err)
}

/// The current editor snapshot, including edits still waiting on autosave.
/// Identity and stored metadata always come from the database.
#[derive(Deserialize)]
pub struct WebhookDraft {
    title: String,
    body: String,
    transcript: String,
    summary: String,
    language: String,
    summary_preset: String,
    summary_provider: String,
}

/// Only persisted sessions contribute timing. Unknown legacy values stay null;
/// note creation/update timestamps are never substituted for recording dates.
fn meeting_timing(entries: &[sessions::SessionEntry], in_progress: bool) -> Value {
    let starts: Vec<_> = entries
        .iter()
        .map(|e| chrono::DateTime::parse_from_rfc3339(&e.started_at).ok())
        .collect();
    let ends: Vec<_> = entries
        .iter()
        .zip(&starts)
        .map(|(e, start)| {
            let duration = i64::try_from(e.duration_ms).ok().filter(|v| *v > 0)?;
            start
                .as_ref()?
                .checked_add_signed(chrono::Duration::try_milliseconds(duration)?)
        })
        .collect();
    let known_starts = !entries.is_empty() && starts.iter().all(Option::is_some);
    let known_ends = !in_progress && !entries.is_empty() && ends.iter().all(Option::is_some);
    let duration =
        if !in_progress && !entries.is_empty() && entries.iter().all(|e| e.duration_ms > 0) {
            entries
                .iter()
                .try_fold(0u64, |sum, e| sum.checked_add(e.duration_ms))
        } else {
            None
        };
    json!({
        "started_at": known_starts.then(|| starts.iter().flatten().min().map(|d| d.to_rfc3339())).flatten(),
        "ended_at": known_ends.then(|| ends.iter().flatten().max().map(|d| d.to_rfc3339())).flatten(),
        "duration_ms": duration,
        "end_time_estimated": true,
        "recording_in_progress": in_progress,
        "sessions": entries.iter().enumerate().map(|(i, e)| json!({
            "id": e.id, "index": e.index,
            "started_at": starts[i].map(|d| d.to_rfc3339()),
            "ended_at": ends[i].map(|d| d.to_rfc3339()),
            "duration_ms": (e.duration_ms > 0).then_some(e.duration_ms),
            "streams": e.streams
        })).collect::<Vec<_>>()
    })
}

fn build_payload(mut note: db::Note, draft: Option<WebhookDraft>, timing: Value) -> Value {
    if let Some(draft) = draft {
        note.title = draft.title;
        note.body = draft.body;
        note.transcript = draft.transcript;
        note.summary = draft.summary;
        note.language = draft.language;
        note.summary_preset = draft.summary_preset;
        note.summary_provider = draft.summary_provider;
    }
    let turns = db::parse_speaker_turns(&note.transcript);
    let mut speakers = Vec::new();
    for turn in &turns {
        if let Some(label) = &turn.speaker {
            if !speakers.contains(label) {
                speakers.push(label.clone());
            }
        }
    }
    // Explicit allowlist: never export local audio paths, credentials or future
    // internal Note fields by accidentally serializing the whole DB record.
    json!({
        "schema_version": 1,
        "event": "note.sent",
        "sent_at": chrono::Utc::now().to_rfc3339(),
        "note": {
            "id": note.id, "title": note.title,
            "body": note.body, "body_text": html_to_text(&note.body),
            "transcript": note.transcript, "summary": note.summary,
            "speakers": speakers,
            "language": note.language, "detected_language": note.detected_language,
            "summary_preset": note.summary_preset, "summary_provider": note.summary_provider,
            "expected_speakers": note.expected_speakers,
            "folder_id": note.folder_id, "client_id": note.client_id,
            "workspace_id": note.workspace_id, "owner": note.owner, "private": note.private,
            "created_at": note.created_at, "updated_at": note.updated_at,
            "meeting": timing
        }
    })
}

async fn deliver(config: &WebhookConfig, payload: &Value) -> Result<u16, String> {
    let (url, headers) = validate(config)?;
    let body = serde_json::to_vec(payload)
        .map_err(|_| "Could not encode the webhook JSON.".to_string())?;
    let payload_bytes = body.len();
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "Could not create the webhook connection.".to_string())?;
    let response = client.post(url).headers(headers)
        .header(reqwest::header::CONTENT_TYPE, "application/json").body(body).send().await
        // reqwest errors can include URLs with embedded tokens. Never return
        // those, or a response body that could echo credentials/meeting data.
        .map_err(|e| if e.is_timeout() {
            "Webhook timed out. Delivery is unconfirmed; check the receiver before sending again.".to_string()
        } else {
            "Could not reach the webhook. Check its URL and your connection. Delivery is unconfirmed.".to_string()
        })?;
    let status = response.status();
    if !status.is_success() {
        if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
            return Err(format!("Webhook returned HTTP 413: the server or proxy rejected the JSON payload ({payload_bytes} bytes) as too large. Increase its request body limit. Humla has not truncated or retried the note."));
        }
        return Err(format!(
            "Webhook returned HTTP {}.{}",
            status.as_u16(),
            if status.is_redirection() {
                " Redirects are not followed; use the final URL."
            } else if matches!(status.as_u16(), 401 | 403) {
                " Check the endpoint and authentication headers."
            } else {
                " Check the webhook server logs."
            }
        ));
    }
    Ok(status.as_u16())
}

#[tauri::command]
pub async fn webhook_send_note(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    note_id: String,
    draft: Option<WebhookDraft>,
) -> Result<u16, String> {
    let note = {
        let conn = state.db.lock();
        if db::get_setting(&conn, "webhook_enabled")
            .map_err(err)?
            .as_deref()
            != Some("true")
        {
            return Err("Enable the webhook in Settings → General first.".into());
        }
        let note = db::get_note(&conn, &note_id).map_err(err)?;
        if note.deleted_at.is_some() {
            return Err("Restore this note before sending it.".into());
        }
        note
    };
    let config = read_config(&app.config().identifier)?;
    let app_dir = app.path().app_data_dir().map_err(err)?;
    let entries: Vec<_> = sessions::resolve_sessions(&sessions::recordings_dir(&app_dir, &note_id))
        .into_iter()
        .map(|(entry, _)| entry)
        .collect();
    let in_progress = state.recording.lock().note_id.as_deref() == Some(&note_id);
    let payload = build_payload(note, draft, meeting_timing(&entries, in_progress));
    let result = deliver(&config, &payload).await;
    // Last delivery only: no endpoint, headers, note identifiers or content.
    let diagnostic = json!({
        "attempted_at": payload["sent_at"],
        "payload_bytes": serde_json::to_vec(&payload).map(|b| b.len()).unwrap_or(0),
        "http_status": result.as_ref().ok(),
        "error": result.as_ref().err()
    });
    let dir = app_dir.join("diagnostics");
    if std::fs::create_dir_all(&dir).is_ok() {
        let _ = std::fs::write(dir.join("webhook-last.json"), diagnostic.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn config(url: &str) -> WebhookConfig {
        WebhookConfig {
            enabled: true,
            url: url.into(),
            headers: vec![WebhookHeader {
                name: "Authorization".into(),
                value: "Bearer test-token".into(),
            }],
        }
    }

    #[test]
    fn validates_urls_and_headers_without_exposing_values() {
        for url in [
            "file:///tmp/secret",
            "not-a-url",
            "https://user:secret@example.com",
            "https://example.com/#secret",
        ] {
            assert!(validate(&config(url)).is_err());
        }
        assert!(validate(&config("https://example.com/hook?token=secret")).is_ok());
        let mut cfg = config("http://127.0.0.1/hook");
        cfg.headers[0].value = "secret\r\nInjected: true".into();
        let error = validate(&cfg).unwrap_err();
        assert!(!error.contains("secret"));
        cfg.headers[0].value = "secret".into();
        cfg.headers.push(WebhookHeader {
            name: "authorization".into(),
            value: "other".into(),
        });
        assert!(validate(&cfg).is_err());
        for name in [
            "Content-Type",
            "Host",
            "Content-Length",
            "Transfer-Encoding",
            "bad name",
        ] {
            cfg.headers = vec![WebhookHeader {
                name: name.into(),
                value: "secret".into(),
            }];
            assert!(validate(&cfg).is_err());
        }
    }

    #[test]
    fn payload_uses_editor_snapshot_and_exports_speakers_without_local_paths() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("test.sqlite")).unwrap();
        let mut note = db::create_note(&conn, "en", "meeting", "").unwrap();
        let id = note.id.clone();
        note.audio_path = Some("/private/recording.wav".into());
        let payload = build_payload(
            note,
            Some(WebhookDraft {
                title: "Unsaved title".into(),
                body: "<p>Plan &amp; ship</p>".into(),
                transcript: "Intro\nÅse: Hello\nYou: Hi\nÅse: https://example.com\n12:30 standup"
                    .into(),
                summary: "- Ship it".into(),
                language: "no".into(),
                summary_preset: "meeting".into(),
                summary_provider: "".into(),
            }),
            meeting_timing(&[], false),
        );
        assert_eq!(payload["schema_version"], 1);
        assert_eq!(payload["note"]["id"], id);
        assert_eq!(payload["note"]["title"], "Unsaved title");
        assert_eq!(payload["note"]["body_text"], "Plan & ship");
        assert_eq!(payload["note"]["speakers"], json!(["Åse", "You"]));
        assert_eq!(
            payload["note"]["transcript"],
            "Intro\nÅse: Hello\nYou: Hi\nÅse: https://example.com\n12:30 standup"
        );
        assert!(payload["note"].get("turns").is_none());
        assert!(payload["note"].get("audio_path").is_none());
        assert!(!payload.to_string().contains("/private/"));
        assert!(payload["note"]["meeting"]["duration_ms"].is_null());
    }

    #[test]
    fn meeting_times_cover_multiple_takes_without_counting_gaps() {
        let entries: Vec<sessions::SessionEntry> = serde_json::from_value(json!([
            {"id":"take-1", "index":1, "startedAt":"2026-10-12T09:00:00Z", "durationMs":60000},
            {"id":"take-2", "index":2, "startedAt":"2026-10-12T09:05:00Z", "durationMs":120000}
        ]))
        .unwrap();
        let timing = meeting_timing(&entries, false);
        assert_eq!(timing["started_at"], "2026-10-12T09:00:00+00:00");
        assert_eq!(timing["ended_at"], "2026-10-12T09:07:00+00:00");
        assert_eq!(timing["duration_ms"], 180000);
        assert_eq!(
            timing["sessions"][0]["ended_at"],
            "2026-10-12T09:01:00+00:00"
        );
        assert_eq!(timing["end_time_estimated"], true);
        let live = meeting_timing(&entries, true);
        assert!(live["ended_at"].is_null());
        assert!(live["duration_ms"].is_null());
        assert_eq!(live["recording_in_progress"], true);
        let mut unknown = entries;
        unknown[0].started_at.clear();
        unknown[0].duration_ms = 0;
        let timing = meeting_timing(&unknown, false);
        assert!(timing["started_at"].is_null());
        assert!(timing["ended_at"].is_null());
        assert!(timing["duration_ms"].is_null());
        assert_eq!(timing["sessions"][1]["duration_ms"], 120000);
        unknown[0].started_at = "invalid date".into();
        unknown[0].duration_ms = u64::MAX;
        let timing = meeting_timing(&unknown, false);
        assert!(timing["duration_ms"].is_null());
        assert!(timing["ended_at"].is_null());
    }

    async fn receiver(status: &str, extra: &str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/hook?token=private-url-token",
            listener.local_addr().unwrap()
        );
        let response =
            format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n{extra}\r\n");
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buf = [0u8; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                    let length: usize = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, task)
    }

    #[tokio::test]
    async fn posts_json_and_auth_headers_and_accepts_204() {
        let (url, request) = receiver("204 No Content", "").await;
        let payload = json!({"note": {"id": "stable-id", "transcript": "Åse: Hello"}});
        assert_eq!(deliver(&config(&url), &payload).await.unwrap(), 204);
        let raw = request.await.unwrap();
        let (headers, body) = raw.split_once("\r\n\r\n").unwrap();
        assert!(headers.starts_with("POST /hook?"));
        assert!(headers
            .to_lowercase()
            .contains("authorization: bearer test-token"));
        assert!(headers
            .to_lowercase()
            .contains("content-type: application/json"));
        assert_eq!(serde_json::from_str::<Value>(body).unwrap(), payload);
    }

    #[tokio::test]
    async fn reports_http_failures_and_does_not_follow_redirects() {
        for status in [
            "401 Unauthorized",
            "413 Content Too Large",
            "500 Internal Server Error",
            "302 Found",
        ] {
            let (url, request) = receiver(
                status,
                "Location: http://127.0.0.1:1/should-not-be-called\r\n",
            )
            .await;
            let error = deliver(&config(&url), &json!({})).await.unwrap_err();
            assert!(error.contains(&status[..3]));
            assert!(!error.contains("private-url-token"));
            if status.starts_with("302") {
                assert!(error.contains("Redirects are not followed"));
            }
            if status.starts_with("413") {
                assert!(error.contains("2 bytes"));
                assert!(error.contains("request body limit"));
                assert!(!error.contains("authentication"));
            }
            request.await.unwrap();
        }
    }
}
