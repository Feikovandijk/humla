//! Parakeet Ultra through the FluidAudio `speaker-diarize` sidecar.
//!
//! The sidecar is reused rather than adding a second signed binary: it already
//! links FluidAudio for diarization, and FluidAudio ships Parakeet TDT/Ultra
//! CoreML models in the same package. `asr-status`, `asr-download` and
//! `asr-delete` mirror the diarize lifecycle commands; `asr-serve` is a
//! long-lived process that loads the 600 MB model once and transcribes chunks
//! over stdin/stdout.
//!
//! Loading the model per chunk would dominate a fast transducer's runtime, so
//! the manager keeps one `asr-serve` child for the app session. Transcription
//! is already serialized by `AppState::transcribe_gate`, so the protocol needs
//! no request multiplexing.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub const MODEL_ID: &str = "parakeet-ultra";

/// The 25 languages Parakeet TDT v3 / Ultra covers. Norwegian is deliberately
/// absent; the adapter refuses those notes rather than emitting nonsense.
pub const SUPPORTED_LANGUAGES: &[&str] = &[
    "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu",
    "it", "lv", "lt", "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru",
    "uk",
];

pub fn supports_language(lang: &str) -> bool {
    lang == "auto" || SUPPORTED_LANGUAGES.contains(&lang)
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Word {
    pub text: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Transcription {
    pub text: String,
    #[serde(default)]
    pub words: Vec<Word>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Progress {
    model_id: String,
    fraction: f64,
    phase: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DownloadError {
    model_id: String,
    message: String,
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

pub struct ParakeetManager {
    sidecar: Option<PathBuf>,
    server: tokio::sync::Mutex<Option<Server>>,
    next_id: AtomicU64,
}

impl ParakeetManager {
    pub fn new(sidecar: Option<PathBuf>) -> Self {
        Self {
            sidecar,
            server: tokio::sync::Mutex::new(None),
            next_id: AtomicU64::new(1),
        }
    }

    /// Start `asr-serve` and wait for its ready line. Loading the model compiles
    /// it for the Neural Engine on first use; `asr-download` warms that away
    /// before a recording, so this is normally just a file load.
    pub async fn prewarm(&self) -> Result<()> {
        let mut guard = self.server.lock().await;
        self.ensure_server(&mut guard).await
    }

    pub async fn transcribe(&self, wav: &Path) -> Result<Transcription> {
        let mut guard = self.server.lock().await;
        for attempt in 0..2 {
            self.ensure_server(&mut guard).await?;
            let id = self.next_id.fetch_add(1, Ordering::Relaxed).to_string();
            let request = serde_json::json!({ "id": id, "path": wav.to_string_lossy() });
            let server = guard.as_mut().expect("ensure_server populated it");
            let write = async {
                server.stdin.write_all(request.to_string().as_bytes()).await?;
                server.stdin.write_all(b"\n").await?;
                server.stdin.flush().await
            }
            .await;
            if let Err(e) = write {
                if attempt == 0 {
                    *guard = None;
                    continue;
                }
                return Err(anyhow!("write to parakeet sidecar: {e}"));
            }

            let mut line = String::new();
            match server.stdout.read_line(&mut line).await {
                Ok(0) => {
                    if attempt == 0 {
                        *guard = None;
                        continue;
                    }
                    return Err(anyhow!("parakeet sidecar closed its stdout"));
                }
                Ok(_) => {}
                Err(e) => {
                    if attempt == 0 {
                        *guard = None;
                        continue;
                    }
                    return Err(anyhow!("read from parakeet sidecar: {e}"));
                }
            }
            match serde_json::from_str::<Transcription>(line.trim()) {
                Ok(result) => {
                    if let Some(err) = result.error {
                        return Err(anyhow!("parakeet: {err}"));
                    }
                    return Ok(result);
                }
                Err(e) => {
                    if attempt == 0 {
                        *guard = None;
                        continue;
                    }
                    return Err(anyhow!("parakeet sidecar payload: {e}: {}", line.trim()));
                }
            }
        }
        Err(anyhow!("parakeet sidecar failed"))
    }

    async fn ensure_server(&self, slot: &mut Option<Server>) -> Result<()> {
        if slot.is_some() {
            return Ok(());
        }
        let sidecar = self
            .sidecar
            .as_ref()
            .ok_or_else(|| anyhow!("speaker-diarize sidecar not found"))?;
        let mut child = Command::new(sidecar)
            .arg("asr-serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow!("spawn speaker-diarize asr-serve: {e}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("no parakeet stdin"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("no parakeet stdout"))?;
        let mut reader = BufReader::new(stdout);
        let mut ready = String::new();
        reader
            .read_line(&mut ready)
            .await
            .map_err(|e| anyhow!("read parakeet ready: {e}"))?;
        let value: serde_json::Value = serde_json::from_str(ready.trim())
            .map_err(|e| anyhow!("parakeet ready: {e}: {}", ready.trim()))?;
        if value.get("event").and_then(|v| v.as_str()) != Some("ready") {
            let message = value
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unexpected startup payload");
            return Err(anyhow!("parakeet sidecar: {message}"));
        }
        *slot = Some(Server {
            child,
            stdin,
            stdout: reader,
        });
        Ok(())
    }

    pub async fn shutdown(&self) {
        let mut guard = self.server.lock().await;
        if let Some(mut server) = guard.take() {
            let _ = server.child.start_kill();
            let _ = server.child.wait().await;
        }
    }
}

/// `speaker-diarize asr-status` → `ModelStatus` JSON. A missing sidecar means
/// "not available" rather than an error, matching diarization's behaviour.
pub async fn status(app: &AppHandle) -> Result<crate::diarize::ModelStatus> {
    let sidecar = match crate::diarize::sidecar_path(app) {
        Ok(p) => p,
        Err(_) => {
            return Ok(crate::diarize::ModelStatus {
                downloaded: false,
                needs_warm_up: false,
                warm_up_error: None,
                size_bytes: None,
                path: None,
            })
        }
    };
    let output = Command::new(&sidecar)
        .arg("asr-status")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("spawn speaker-diarize asr-status: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "{}",
            crate::diarize::sidecar_error(&stderr, output.status)
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| l.starts_with('{'))
        .ok_or_else(|| anyhow!("asr-status returned no JSON"))?;
    serde_json::from_str(line).map_err(|e| anyhow!("asr-status payload: {e}: {line}"))
}

static DOWNLOAD_TURN: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Download (or warm) Parakeet Ultra, forwarding progress to the UI. One
/// download at a time app-wide.
pub async fn download(app: &AppHandle) -> Result<()> {
    let _turn = DOWNLOAD_TURN.lock().await;
    if matches!(status(app).await, Ok(s) if s.downloaded) {
        return Ok(());
    }
    let sidecar = crate::diarize::sidecar_path(app)?;
    let mut child = Command::new(&sidecar)
        .arg("asr-download")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| anyhow!("spawn speaker-diarize asr-download: {e}"))?;
    let stdout = child.stdout.take().ok_or_else(|| anyhow!("no stdout"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow!("no stderr"))?;
    let stderr_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        let mut buf = String::new();
        while let Ok(Some(line)) = reader.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });
    let mut reader = BufReader::new(stdout).lines();
    let mut failure: Option<String> = None;
    let mut last_phase = String::new();
    let mut last_fraction = -1.0_f64;
    while let Ok(Some(line)) = reader.next_line().await {
        let Some(value) = json_from_line(&line) else {
            continue;
        };
        match value.get("event").and_then(|v| v.as_str()) {
            Some("progress") => {
                let phase = value
                    .get("phase")
                    .and_then(|v| v.as_str())
                    .unwrap_or("downloading")
                    .to_string();
                let fraction = value
                    .get("fraction")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                // FluidAudio emits a progress line per decoded chunk, which is
                // thousands per model. Throttle to 0.5% steps so the webview
                // isn't doing 10k renders for one download.
                if phase == last_phase && (fraction - last_fraction).abs() < 0.005 {
                    continue;
                }
                last_phase = phase.clone();
                last_fraction = fraction;
                let _ = app.emit(
                    "parakeet_progress",
                    Progress {
                        model_id: MODEL_ID.to_string(),
                        fraction,
                        phase,
                    },
                );
            }
            Some("failed") => {
                failure = Some(
                    value
                        .get("step")
                        .and_then(|v| v.as_str())
                        .unwrap_or("download")
                        .to_string(),
                );
            }
            _ => {}
        }
    }
    let exit = child.wait().await.map_err(|e| anyhow!("wait: {e}"))?;
    let stderr_text = stderr_handle.await.unwrap_or_default();
    if !exit.success() {
        let message = crate::diarize::sidecar_error(&stderr_text, exit);
        let message = match failure.as_deref() {
            Some("prepare") => format!("The model downloaded, but preparing it for the Neural Engine failed: {message}"),
            Some("download") => format!("Downloading the model failed: {message}"),
            _ => message,
        };
        let _ = app.emit(
            "parakeet_download_error",
            DownloadError {
                model_id: MODEL_ID.to_string(),
                message: message.clone(),
            },
        );
        return Err(anyhow!(message));
    }
    // Terminal progress marker: a surface other than Settings may be showing
    // the global download slice, and only this event (or the error above)
    // clears it when the download outlives the Settings mount.
    let _ = app.emit(
        "parakeet_progress",
        Progress {
            model_id: MODEL_ID.to_string(),
            fraction: 1.0,
            phase: "done".to_string(),
        },
    );
    Ok(())
}

pub async fn delete(app: &AppHandle) -> Result<()> {
    let sidecar = crate::diarize::sidecar_path(app)?;
    let output = Command::new(&sidecar)
        .arg("asr-delete")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("spawn speaker-diarize asr-delete: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "{}",
            crate::diarize::sidecar_error(&stderr, output.status)
        ));
    }
    Ok(())
}

fn json_from_line(line: &str) -> Option<serde_json::Value> {
    let line = line.trim();
    line.match_indices('{')
        .find_map(|(i, _)| serde_json::from_str::<serde_json::Value>(&line[i..]).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_support_matches_parakeet_v3() {
        assert!(supports_language("auto"));
        assert!(supports_language("en"));
        assert!(supports_language("de"));
        assert!(!supports_language("no"));
        assert!(!supports_language("nb"));
        assert!(!supports_language("nn"));
    }

    #[test]
    fn response_payload_parses_words() {
        let r: Transcription = serde_json::from_str(
            r#"{"text":"hi there","words":[{"text":"hi","start_ms":0,"end_ms":100}]}"#,
        )
        .unwrap();
        assert_eq!(r.text, "hi there");
        assert_eq!(r.words.len(), 1);
        assert_eq!(r.words[0].end_ms, 100);
    }
}
