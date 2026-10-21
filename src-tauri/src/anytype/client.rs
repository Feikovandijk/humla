//! The Anytype local HTTP API (v1, `2025-11-08`), served by the Anytype
//! desktop app itself — the request never leaves the Mac. The base URL is
//! therefore pinned to loopback: a note must not be able to leave through a
//! "custom" endpoint. Errors are phrased for the user and never echo the API
//! key or the document that was sent.
//!
//! Pinned facts about the API, from `docs/research/anytype-api.md`:
//!   - create (`POST /v1/spaces/{id}/objects`) takes the Markdown body in
//!     `body`; update (`PATCH /v1/spaces/{id}/objects/{oid}`) takes it in
//!     `markdown`. The asymmetry is real — a test pins it.
//!   - update replaces the whole body; there is no partial patch in v1.
//!   - every write is rate limited (sustained ~1 rps): the sync queue paces
//!     itself, so the client only has to say so when Anytype answers 429.
//!   - the version is carried in the `Anytype-Version` header.

use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const API_VERSION: &str = "2025-11-08";
pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:31009";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectType {
    pub key: String,
    pub name: String,
}

/// What went wrong, in the vocabulary the sync worker acts on.
#[derive(Debug)]
pub enum ApiError {
    /// The object (or, on create, the space) is gone: 404 or 410. On update
    /// this means "recreate it", so it is its own variant rather than text.
    Gone,
    /// Everything else, already phrased for the user.
    Message(String),
}

impl ApiError {
    /// [`ApiError::Message`] by value — the common case in callers.
    pub fn message(self) -> String {
        match self {
            ApiError::Gone => "That object no longer exists in Anytype.".to_string(),
            ApiError::Message(m) => m,
        }
    }
}

pub struct Client {
    base: Url,
    key: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base_url: &str, key: &str) -> Result<Self, String> {
        let base = validate_base_url(base_url)?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("Humla/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| "Could not create the connection to Anytype.".to_string())?;
        Ok(Client {
            base,
            key: key.trim().to_string(),
            http,
        })
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let url = self.base.join(path).expect("path joined onto a valid base");
        self.http
            .request(method, url)
            .header("Authorization", format!("Bearer {}", self.key))
            .header("Anytype-Version", API_VERSION)
    }

    pub async fn list_spaces(&self) -> Result<Vec<Space>, ApiError> {
        let response = self
            .request(reqwest::Method::GET, "/v1/spaces?limit=100")
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status, body_message(response).await));
        }
        #[derive(Deserialize)]
        struct Page {
            data: Vec<Space>,
        }
        let page: Page = response
            .json()
            .await
            .map_err(|_| ApiError::Message("Anytype's answer couldn't be read.".to_string()))?;
        Ok(page.data)
    }

    pub async fn list_types(&self, space_id: &str) -> Result<Vec<ObjectType>, ApiError> {
        let response = self
            .request(
                reqwest::Method::GET,
                &format!("/v1/spaces/{space_id}/types?limit=100"),
            )
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status, body_message(response).await));
        }
        #[derive(Deserialize)]
        struct Page {
            data: Vec<ObjectType>,
        }
        let page: Page = response
            .json()
            .await
            .map_err(|_| ApiError::Message("Anytype's answer couldn't be read.".to_string()))?;
        Ok(page.data)
    }

    /// Create the object and return its id. The Markdown body goes in
    /// `body` — the create request's own field name (see the module docs).
    pub async fn create_object(
        &self,
        space_id: &str,
        type_key: &str,
        name: &str,
        markdown: &str,
    ) -> Result<String, ApiError> {
        let response = self
            .request(
                reqwest::Method::POST,
                &format!("/v1/spaces/{space_id}/objects"),
            )
            .json(&serde_json::json!({
                "type_key": type_key,
                "name": name,
                "body": markdown,
            }))
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status, body_message(response).await));
        }
        #[derive(Deserialize)]
        struct Created {
            object: ObjectId,
        }
        #[derive(Deserialize)]
        struct ObjectId {
            id: String,
        }
        let created: Created = response
            .json()
            .await
            .map_err(|_| ApiError::Message("Anytype's answer couldn't be read.".to_string()))?;
        Ok(created.object.id)
    }

    /// Replace the object's name and whole Markdown body — update takes it
    /// in `markdown`, not `body`.
    pub async fn update_object(
        &self,
        space_id: &str,
        object_id: &str,
        name: &str,
        markdown: &str,
    ) -> Result<(), ApiError> {
        let response = self
            .request(
                reqwest::Method::PATCH,
                &format!("/v1/spaces/{space_id}/objects/{object_id}"),
            )
            .json(&serde_json::json!({
                "name": name,
                "markdown": markdown,
            }))
            .send()
            .await
            .map_err(transport_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(status_error(status, body_message(response).await));
        }
        Ok(())
    }
}

/// The base URL must point at this machine: the whole point of Anytype's
/// local API is that the meeting never leaves the Mac through Humla, and a
/// configurable remote endpoint would quietly break that promise.
pub(crate) fn validate_base_url(base_url: &str) -> Result<Url, String> {
    let url = Url::parse(base_url.trim())
        .map_err(|_| "Enter a valid Anytype API address.".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("The Anytype API address must start with http:// or https://.".into());
    }
    let host = url.host_str().unwrap_or_default();
    if !matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]") {
        return Err(
            "The Anytype API runs on this Mac; its address must be 127.0.0.1, localhost or ::1."
                .into(),
        );
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("Remove the credentials and fragment from the Anytype API address.".into());
    }
    let path = url.path().trim_end_matches('/');
    if !path.is_empty() {
        return Err("The Anytype API address has no path — just the host and port.".into());
    }
    Ok(url)
}

/// Transport failures, phrased around the one setup fact the user can act
/// on: the API lives inside the Anytype app, so "can't reach it" almost
/// always means "Anytype isn't running".
fn transport_error(e: reqwest::Error) -> ApiError {
    ApiError::Message(if e.is_timeout() {
        "Anytype took too long to answer. The change may still have landed — check Anytype before syncing again.".to_string()
    } else if e.is_connect() {
        "Anytype isn’t running. Open it and try again.".to_string()
    } else {
        "Could not reach Anytype. Open it and try again.".to_string()
    })
}

/// Anytype's own error body is `{"code":…, "message":…}`; the message is the
/// useful part and it's short. Bounded anyway, so an unexpectedly chatty
/// server can't pour a page of text into a toast.
async fn body_message(response: reqwest::Response) -> Option<String> {
    let body = response.text().await.ok()?;
    let message = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|v| v.get("message").and_then(|m| m.as_str()).map(str::to_string))
        .unwrap_or_else(|| body.trim().to_string());
    if message.is_empty() {
        return None;
    }
    let mut chars = message.chars();
    let truncated: String = chars.by_ref().take(160).collect();
    Some(if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    })
}

fn status_error(status: StatusCode, message: Option<String>) -> ApiError {
    match status.as_u16() {
        404 | 410 => ApiError::Gone,
        401 | 403 => ApiError::Message(
            "The API key was rejected. Create a new one in Anytype → Settings → API Keys and save it in Settings here.".to_string(),
        ),
        429 => ApiError::Message(
            "Anytype is limiting how fast it accepts changes. Wait a moment and try again."
                .to_string(),
        ),
        400 => ApiError::Message(match message {
            Some(m) => format!("Anytype rejected the document: {m}"),
            None => "Anytype rejected the document.".to_string(),
        }),
        s if s >= 500 => ApiError::Message(match message {
            Some(m) => format!("Anytype failed (HTTP {s}): {m}"),
            None => format!("Anytype failed (HTTP {s}). Check that it is running."),
        }),
        s => ApiError::Message(format!("Anytype answered HTTP {s}.")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn the_base_url_stays_on_this_machine() {
        for url in [
            "https://api.anytype.io",
            "http://192.168.1.20:31009",
            "http://evil.example.com",
            "http://127.0.0.1.evil.example.com",
            "file:///tmp/x",
            "http://user:secret@127.0.0.1:31009",
            "http://127.0.0.1:31009/#frag",
            "http://127.0.0.1:31009/v1",
            "not a url",
        ] {
            assert!(validate_base_url(url).is_err(), "{url} must be refused");
        }
        for url in [
            "http://127.0.0.1:31009",
            "http://localhost:31009",
            "https://127.0.0.1:31009/",
            "http://[::1]:31009",
        ] {
            assert!(validate_base_url(url).is_ok(), "{url} must be accepted");
        }
    }

    /// One scripted HTTP response on a random loopback port, and the request
    /// it received. The same pattern the webhook tests use — which the
    /// loopback-only rule above exists to keep possible.
    async fn stub_server(
        status: &str,
        body: &str,
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&request).contains("\r\n\r\n") {
                    // Reads until close would hang on keep-alive-less bodies;
                    // Content-Length is small in every test, so one completed
                    // head plus a body chunk is enough.
                    if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..pos]).to_lowercase();
                        let len: usize = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .unwrap_or("0")
                            .parse()
                            .unwrap();
                        if request.len() >= pos + 4 + len {
                            break;
                        }
                    }
                }
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (url, task)
    }

    #[tokio::test]
    async fn create_sends_body_and_update_sends_markdown() {
        let (url, received) = stub_server(
            "201 Created",
            r#"{"object":{"id":"obj-1","name":"T"}}"#,
        )
        .await;
        let client = Client::new(&url, "key-123").unwrap();
        let id = client
            .create_object("space-1", "page", "Title", "# Body")
            .await
            .unwrap();
        assert_eq!(id, "obj-1");
        let request = received.await.unwrap();
        assert!(request.starts_with("POST /v1/spaces/space-1/objects HTTP/1.1"));
        let head = request.split("\r\n\r\n").next().unwrap().to_lowercase();
        assert!(head.contains("authorization: bearer key-123"));
        assert!(head.contains("anytype-version: 2025-11-08"));
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["body"], "# Body");
        assert_eq!(body["type_key"], "page");
        assert_eq!(body["name"], "Title");
        assert!(body.get("markdown").is_none(), "create must not send `markdown`");

        let (url, received) = stub_server("200 OK", "{}").await;
        let client = Client::new(&url, "key-123").unwrap();
        client
            .update_object("space-1", "obj-1", "Title", "# New body")
            .await
            .unwrap();
        let request = received.await.unwrap();
        assert!(request.starts_with("PATCH /v1/spaces/space-1/objects/obj-1 HTTP/1.1"));
        let body: serde_json::Value =
            serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["markdown"], "# New body");
        assert!(body.get("body").is_none(), "update must not send `body`");
    }

    #[tokio::test]
    async fn gone_and_statuses_map_to_the_actions_they_call_for() {
        for status in ["404 Not Found", "410 Gone"] {
            let (url, received) = stub_server(status, "{}").await;
            let client = Client::new(&url, "k").unwrap();
            let result = client.update_object("s", "o", "n", "b").await;
            assert!(matches!(result, Err(ApiError::Gone)), "{status} must read as Gone");
            received.await.unwrap();
        }
        let (url, received) = stub_server(
            "401 Unauthorized",
            r#"{"code":"unauthorized","message":"Unauthorized","status":401}"#,
        )
        .await;
        let client = Client::new(&url, "k").unwrap();
        let err = client.list_spaces().await.unwrap_err().message();
        assert!(err.contains("API key was rejected"), "{err}");
        received.await.unwrap();

        let (url, received) =
            stub_server("429 Too Many Requests", r#"{"message":"slow down"}"#).await;
        let client = Client::new(&url, "k").unwrap();
        let err = client.list_spaces().await.unwrap_err().message();
        assert!(err.contains("limiting"), "{err}");
        received.await.unwrap();
    }

    #[test]
    fn an_unreachable_api_means_anytype_is_not_running() {
        // Port 1 on loopback refuses the connection.
        let client = Client::new("http://127.0.0.1:1", "k").unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let err = rt.block_on(client.list_spaces()).unwrap_err().message();
        assert!(err.contains("isn’t running"), "{err}");
    }
}
