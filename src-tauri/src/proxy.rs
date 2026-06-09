//! In-process bearer auth proxy (issue #3 — "the lock").
//!
//! Listens on `127.0.0.1:PROXY_PORT`, requires `Authorization: Bearer <token>`,
//! and stream-forwards authorized requests to Ollama at `127.0.0.1:11434`.
//! Unauthorized → 401. Loopback-only: `tailscale serve` is the sole external
//! path in (see the pairing proof — loopback is tighter than binding the
//! Tailscale interface, the endpoint is unreachable on the LAN without the mesh).
//!
//! This is a direct port of `handrun/auth_proxy.py`, which we validated
//! end-to-end against the live iOS app.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use tokio::sync::RwLock;

pub const PROXY_PORT: u16 = 11435;
const UPSTREAM: &str = "http://127.0.0.1:11434";

/// Shared token; swappable at runtime for rotation.
#[derive(Clone)]
pub struct ProxyState {
    pub token: Arc<RwLock<String>>,
    client: reqwest::Client,
}

impl ProxyState {
    pub fn new(token: String) -> Self {
        Self {
            token: Arc::new(RwLock::new(token)),
            client: reqwest::Client::new(),
        }
    }
}

/// Spawn the proxy server on a background task. Returns once bound (or errors).
pub async fn serve(state: ProxyState) -> std::io::Result<()> {
    let app = Router::new()
        .route("/", any(forward))
        .route("/*path", any(forward))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", PROXY_PORT)).await?;
    axum::serve(listener, app).await
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        // constant-time-ish compare; tokens are short and high-entropy
        .map(|got| got.as_bytes() == token.as_bytes())
        .unwrap_or(false)
}

async fn forward(State(state): State<ProxyState>, req: Request) -> Response {
    {
        let token = state.token.read().await;
        if !authorized(req.headers(), &token) {
            return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
        }
    }

    // Rebuild the upstream URL: same path + query, pointed at Ollama.
    let path_q = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let url = format!("{UPSTREAM}{path_q}");

    let method = req.method().clone();
    let mut headers = req.headers().clone();
    headers.remove(axum::http::header::AUTHORIZATION);
    headers.remove(axum::http::header::HOST);

    let body = req.into_body();
    let body_bytes = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::BAD_GATEWAY, "bad request body").into_response(),
    };

    let upstream = state
        .client
        .request(method, &url)
        .headers(headers)
        .body(body_bytes)
        .send()
        .await;

    match upstream {
        Ok(resp) => {
            let status = resp.status();
            let mut builder = Response::builder().status(status);
            for (k, v) in resp.headers() {
                // skip hop-by-hop; reqwest already de-chunks, let axum re-frame
                if k == axum::http::header::TRANSFER_ENCODING {
                    continue;
                }
                builder = builder.header(k, v);
            }
            // Stream the body through so NDJSON chat streaming stays incremental.
            let stream = resp.bytes_stream();
            builder
                .body(Body::from_stream(stream))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "upstream unreachable").into_response(),
    }
}

/// Quick check that Ollama is reachable (used by the UI before sharing).
pub async fn ollama_up() -> bool {
    reqwest::Client::new()
        .get(format!("{UPSTREAM}/api/tags"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// First model id from Ollama, if any (for the pairing payload's `models=`).
pub async fn first_model() -> Option<String> {
    let v: serde_json::Value = reqwest::Client::new()
        .get(format!("{UPSTREAM}/api/tags"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    v.get("models")?
        .as_array()?
        .first()?
        .get("name")?
        .as_str()
        .map(String::from)
}

/// Suppress the unused-import warning on Uri if the router shape changes.
#[allow(dead_code)]
fn _uri_marker(_: Uri) {}
