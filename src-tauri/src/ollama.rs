//! Ollama detection + readiness for the first-run flow (issue #6).
//!
//! We don't manage the Ollama process; we detect three things the user can act
//! on — is it installed, is it running, does it have a model — and the UI turns
//! each into one clear next step. The actual chat traffic goes through the
//! loopback proxy (see `proxy.rs`).

use std::path::PathBuf;

use serde::Serialize;

/// What the user needs to do next about their local model, if anything.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum OllamaState {
    /// No Ollama install found on disk.
    NotInstalled,
    /// Installed, but not answering on the local port yet.
    NotRunning,
    /// Running, but no model has been pulled.
    NoModel,
    /// Running with at least one model — ready to share.
    Ready { model: String },
}

#[cfg(target_os = "macos")]
fn candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/bin/ollama"),
        PathBuf::from("/opt/homebrew/bin/ollama"),
        PathBuf::from("/Applications/Ollama.app/Contents/Resources/ollama"),
    ]
}

#[cfg(windows)]
fn candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        v.push(PathBuf::from(format!(
            r"{local}\Programs\Ollama\ollama.exe"
        )));
    }
    v.push(PathBuf::from(r"C:\Program Files\Ollama\ollama.exe"));
    v
}

#[cfg(all(not(windows), not(target_os = "macos")))]
fn candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/local/bin/ollama"),
        PathBuf::from("/usr/bin/ollama"),
    ]
}

/// Whether an Ollama install exists on disk (binary or, on macOS, the app).
pub fn installed() -> bool {
    if candidates().iter().any(|p| p.exists()) {
        return true;
    }
    #[cfg(target_os = "macos")]
    if std::path::Path::new("/Applications/Ollama.app").exists() {
        return true;
    }
    false
}

/// Resolve the actionable state by probing the local endpoint, then disk.
pub async fn status() -> OllamaState {
    if crate::proxy::ollama_up().await {
        return match crate::proxy::first_model().await {
            Some(model) => OllamaState::Ready { model },
            None => OllamaState::NoModel,
        };
    }
    if installed() {
        OllamaState::NotRunning
    } else {
        OllamaState::NotInstalled
    }
}
