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

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use tokio::sync::RwLock;

/// Preferred port; if something else holds it we try the next few.
pub const PROXY_PORT: u16 = 11435;
const PORT_ATTEMPTS: u16 = 10;
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
            // Connect timeout only: chat responses stream for as long as the
            // model generates, so an overall timeout would cut them off.
            client: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
        }
    }
}

/// Bind the loopback listener synchronously, so the caller knows the real port
/// (and any failure) before the UI offers to share. Falls back to the next few
/// ports when the preferred one is taken.
pub fn bind() -> Result<(std::net::TcpListener, u16), String> {
    let mut last_err = None;
    for port in PROXY_PORT..PROXY_PORT + PORT_ATTEMPTS {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => return Ok((l, port)),
            Err(e) => last_err = Some(e),
        }
    }
    Err(format!(
        "ports {}–{} are all in use ({})",
        PROXY_PORT,
        PROXY_PORT + PORT_ATTEMPTS - 1,
        last_err.map(|e| e.to_string()).unwrap_or_default()
    ))
}

/// Run the proxy on an already-bound listener (call from inside a tokio runtime).
pub async fn serve(listener: std::net::TcpListener, state: ProxyState) -> std::io::Result<()> {
    let app = Router::new()
        .route("/", any(forward))
        .route("/*path", any(forward))
        .with_state(state);
    listener.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(listener)?;
    axum::serve(listener, app).await
}

/// Compare without short-circuiting on the first differing byte, so response
/// timing doesn't leak how much of a guessed token was right.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn authorized(headers: &HeaderMap, token: &str) -> bool {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|got| constant_time_eq(got.as_bytes(), token.as_bytes()))
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

/// Shared client for quick local probes; short timeout so a hung Ollama shows
/// up as "not running" instead of freezing the UI.
fn probe_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default()
    })
}

/// Quick check that Ollama is reachable (used by the UI before sharing).
pub async fn ollama_up() -> bool {
    probe_client()
        .get(format!("{UPSTREAM}/api/tags"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// First model id from Ollama, if any (for the pairing payload's `models=`).
pub async fn first_model() -> Option<String> {
    let v: serde_json::Value = probe_client()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_compare() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
    }

    #[test]
    fn bind_falls_back_when_port_taken() {
        let (first, p1) = bind().expect("first bind");
        let (_second, p2) = bind().expect("second bind");
        assert_ne!(p1, p2);
        drop(first);
    }
}
