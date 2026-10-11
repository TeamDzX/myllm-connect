//! Loopback bearer-auth proxy for the MyLLM iOS app.
//!
//! The phone speaks the Ollama endpoints it already knows. Ollama requests are
//! forwarded unchanged; llama.cpp requests are translated to its OpenAI-style
//! API and translated back to Ollama JSON/NDJSON on the response path.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::backend::{self, Backend, BackendKind, LlamaStreamTranslator};

pub const PROXY_PORT: u16 = 11435;
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct ProxyState {
    pub token: Arc<RwLock<String>>,
    client: reqwest::Client,
    backend: Arc<RwLock<Option<Backend>>>,
}

impl ProxyState {
    pub fn new(token: String) -> Self {
        Self {
            token: Arc::new(RwLock::new(token)),
            client: backend::client(),
            backend: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn refresh_backend(&self) -> Option<Backend> {
        let detected = backend::detect(&self.client).await;
        *self.backend.write().await = detected.clone();
        detected
    }

    async fn backend_or_detect(&self) -> Option<Backend> {
        if let Some(backend) = self.backend.read().await.clone() {
            return Some(backend);
        }
        self.refresh_backend().await
    }
}

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
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
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

    let Some(backend) = state.backend_or_detect().await else {
        return (
            StatusCode::BAD_GATEWAY,
            "no local Ollama or llama.cpp server found",
        )
            .into_response();
    };

    let method = req.method().clone();
    let path_q = req
        .uri()
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let mut headers = req.headers().clone();
    headers.remove(header::AUTHORIZATION);
    headers.remove(header::HOST);
    let body = match axum::body::to_bytes(req.into_body(), MAX_REQUEST_BYTES).await {
        Ok(body) => body,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response(),
    };

    match backend.kind {
        BackendKind::Ollama => {
            forward_ollama(&state, &backend, method, path_q, headers, body).await
        }
        BackendKind::LlamaCpp => forward_llama_cpp(&state, &backend, method, path_q, body).await,
    }
}

async fn forward_ollama(
    state: &ProxyState,
    backend: &Backend,
    method: Method,
    path_q: String,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let request = state
        .client
        .request(method, format!("{}{path_q}", backend.base_url))
        .headers(headers)
        .body(body);
    pass_through(backend.request(request).send().await).await
}

async fn forward_llama_cpp(
    state: &ProxyState,
    backend: &Backend,
    method: Method,
    path_q: String,
    body: axum::body::Bytes,
) -> Response {
    let path = path_q.split('?').next().unwrap_or(&path_q);
    match (method, path) {
        (Method::GET, "/api/tags") => llama_tags(state, backend).await,
        (Method::POST, "/api/chat") => llama_chat(state, backend, body).await,
        _ => (
            StatusCode::NOT_FOUND,
            "llama.cpp adapter supports GET /api/tags and POST /api/chat",
        )
            .into_response(),
    }
}

async fn llama_tags(state: &ProxyState, backend: &Backend) -> Response {
    let request = state.client.get(format!("{}/v1/models", backend.base_url));
    let response = match backend.request(request).send().await {
        Ok(response) => response,
        Err(_) => return (StatusCode::BAD_GATEWAY, "llama.cpp is unreachable").into_response(),
    };
    let status = response.status();
    let value = match response.json::<Value>().await {
        Ok(value) if status.is_success() => backend::llama_models_to_ollama(value),
        Ok(value) => return json_response(status, value),
        Err(_) => {
            return (StatusCode::BAD_GATEWAY, "invalid llama.cpp model response").into_response()
        }
    };
    json_response(StatusCode::OK, value)
}

async fn llama_chat(state: &ProxyState, backend: &Backend, body: axum::body::Bytes) -> Response {
    let source = match serde_json::from_slice::<Value>(&body) {
        Ok(value) => value,
        Err(_) => return (StatusCode::BAD_REQUEST, "chat body must be valid JSON").into_response(),
    };
    let fallback_model = source
        .get("model")
        .and_then(Value::as_str)
        .or(backend.model.as_deref())
        .unwrap_or("local-model")
        .to_string();
    let (request_body, streaming) = match backend::ollama_chat_to_openai(source) {
        Ok(result) => result,
        Err(error) => return (StatusCode::BAD_REQUEST, error).into_response(),
    };
    let request = state
        .client
        .post(format!("{}/v1/chat/completions", backend.base_url))
        .json(&request_body);
    let response = match backend.request(request).send().await {
        Ok(response) => response,
        Err(_) => return (StatusCode::BAD_GATEWAY, "llama.cpp is unreachable").into_response(),
    };
    let status = response.status();
    if !status.is_success() {
        return pass_through(Ok(response)).await;
    }

    if !streaming {
        return match response.json::<Value>().await {
            Ok(value) => match backend::openai_chat_to_ollama(value, &fallback_model) {
                Ok(value) => json_response(StatusCode::OK, value),
                Err(error) => (StatusCode::BAD_GATEWAY, error).into_response(),
            },
            Err(_) => (StatusCode::BAD_GATEWAY, "invalid llama.cpp chat response").into_response(),
        };
    }

    let mut source = response.bytes_stream();
    let stream = async_stream::stream! {
        let mut translator = LlamaStreamTranslator::new(fallback_model);
        while let Some(item) = source.next().await {
            match item {
                Ok(bytes) => {
                    for output in translator.push(&bytes) {
                        yield Ok::<Vec<u8>, std::io::Error>(output.into_bytes());
                    }
                }
                Err(error) => {
                    yield Err(std::io::Error::other(error));
                    return;
                }
            }
        }
        for output in translator.finish() {
            yield Ok(output.into_bytes());
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
}

async fn pass_through(result: Result<reqwest::Response, reqwest::Error>) -> Response {
    match result {
        Ok(response) => {
            let status = response.status();
            let mut builder = Response::builder().status(status);
            for (name, value) in response.headers() {
                if name == header::TRANSFER_ENCODING || name == header::CONTENT_LENGTH {
                    continue;
                }
                builder = builder.header(name, value);
            }
            builder
                .body(Body::from_stream(response.bytes_stream()))
                .unwrap_or_else(|_| StatusCode::BAD_GATEWAY.into_response())
        }
        Err(_) => (StatusCode::BAD_GATEWAY, "upstream unreachable").into_response(),
    }
}

fn json_response(status: StatusCode, value: Value) -> Response {
    match serde_json::to_vec(&value) {
        Ok(body) => Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
        Err(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            json!({ "error": "failed to encode response" }).to_string(),
        )
            .into_response(),
    }
}
