//! Readiness checks for the supported local model servers.

use serde::Serialize;

use crate::{backend::BackendKind, proxy::ProxyState};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelServerState {
    NotRunning,
    NoModel { backend: BackendKind },
    Ready { model: String, backend: BackendKind },
}

pub async fn status(proxy: &ProxyState) -> ModelServerState {
    match proxy.refresh_backend().await {
        Some(backend) => match backend.model {
            Some(model) => ModelServerState::Ready {
                model,
                backend: backend.kind,
            },
            None => ModelServerState::NoModel {
                backend: backend.kind,
            },
        },
        None => ModelServerState::NotRunning,
    }
}
