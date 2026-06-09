//! Tailscale detection + `tailscale serve` (issue #4).
//!
//! We do NOT bundle Tailscale (ADR-003). We locate the CLI, check the node
//! state, and drive `tailscale serve` to expose the loopback auth proxy as
//! `https://<machine>.<tailnet>.ts.net`. The states mirror what the pairing
//! proof surfaced — including `ServeNotEnabled`, the fresh-tailnet case the
//! original spec missed (a one-time admin-console enable, with a URL the CLI
//! hands us).

use std::path::PathBuf;
use std::process::Command;

use serde::Serialize;

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TailscaleState {
    /// CLI not found on disk.
    NotInstalled,
    /// Installed but the node is logged out.
    LoggedOut,
    /// Logged in, but HTTPS/Serve isn't enabled on the tailnet yet.
    /// `enable_url` is the admin-console link the CLI provides.
    ServeNotEnabled { enable_url: Option<String> },
    /// Serve is up; `url` is the public HTTPS endpoint.
    Healthy { url: String },
    /// Anything else (serve failed, transient) with a message.
    Error { message: String },
}

#[cfg(windows)]
fn candidates() -> Vec<PathBuf> {
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    vec![PathBuf::from(format!(r"{pf}\Tailscale\tailscale.exe"))]
}

#[cfg(not(windows))]
fn candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/bin/tailscale"),
        PathBuf::from("/usr/local/bin/tailscale"),
        PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
    ]
}

/// Path to the tailscale CLI, if present.
pub fn cli() -> Option<PathBuf> {
    candidates().into_iter().find(|p| p.exists())
}

fn run(cli: &PathBuf, args: &[&str]) -> std::io::Result<std::process::Output> {
    Command::new(cli).args(args).output()
}

/// Current node state without touching serve.
pub fn status() -> TailscaleState {
    let Some(cli) = cli() else {
        return TailscaleState::NotInstalled;
    };
    match run(&cli, &["status"]) {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).to_lowercase();
            let err = String::from_utf8_lossy(&out.stderr).to_lowercase();
            if text.contains("logged out") || err.contains("logged out") {
                TailscaleState::LoggedOut
            } else {
                // Healthy-ish; the real URL only exists after `serve`.
                TailscaleState::Healthy { url: String::new() }
            }
        }
        Err(e) => TailscaleState::Error {
            message: e.to_string(),
        },
    }
}

/// Parse the `https://...ts.net` URL out of `tailscale serve status`/serve output.
fn extract_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|tok| tok.starts_with("https://") && tok.contains(".ts.net"))
        .map(|s| s.trim_end_matches('/').to_string())
}

/// Parse the admin-console enable URL from a "Serve is not enabled" error.
fn extract_enable_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|tok| tok.starts_with("https://login.tailscale.com/"))
        .map(String::from)
}

/// Bring up `tailscale serve` for the loopback proxy and return the state.
pub fn start_serve(proxy_port: u16) -> TailscaleState {
    let Some(cli) = cli() else {
        return TailscaleState::NotInstalled;
    };

    // Already logged out? report it cleanly.
    if status() == TailscaleState::LoggedOut {
        return TailscaleState::LoggedOut;
    }

    let port = proxy_port.to_string();
    let out = match run(&cli, &["serve", "--bg", &port]) {
        Ok(o) => o,
        Err(e) => {
            return TailscaleState::Error {
                message: e.to_string(),
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let combined = format!("{stdout}\n{stderr}");

    if combined.to_lowercase().contains("serve is not enabled") {
        return TailscaleState::ServeNotEnabled {
            enable_url: extract_enable_url(&combined),
        };
    }

    // The URL may be in this output, or only in `serve status` — try both.
    if let Some(url) = extract_url(&combined) {
        return TailscaleState::Healthy { url };
    }
    if let Ok(s) = run(&cli, &["serve", "status"]) {
        let txt = String::from_utf8_lossy(&s.stdout);
        if let Some(url) = extract_url(&txt) {
            return TailscaleState::Healthy { url };
        }
    }

    TailscaleState::Error {
        message: "serve started but no https URL was found".into(),
    }
}
