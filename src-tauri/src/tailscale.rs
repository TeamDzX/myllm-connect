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
    /// CLI not found on disk, and the GUI app isn't present either.
    NotInstalled,
    /// The Tailscale app is installed but its command-line tool isn't linked.
    /// Mac App Store installs don't add the CLI to a standard path — the user
    /// must run "Tailscale ▸ Install CLI" once. (macOS-specific.)
    CliNotLinked,
    /// Installed but the node is logged out.
    LoggedOut,
    /// Installed, but Tailscale is switched off or its background service
    /// isn't running — the user needs to open Tailscale and connect.
    Stopped,
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
        // Homebrew (Apple Silicon, then Intel), standalone pkg, then the
        // Mac App Store app bundle's embedded CLI.
        PathBuf::from("/opt/homebrew/bin/tailscale"),
        PathBuf::from("/usr/local/bin/tailscale"),
        PathBuf::from("/usr/bin/tailscale"),
        PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
    ]
}

/// Path to the tailscale CLI, if present.
pub fn cli() -> Option<PathBuf> {
    candidates().into_iter().find(|p| p.exists())
}

/// State to report when no CLI binary is found: on macOS, distinguish "the app
/// is installed but the CLI isn't linked" (the Mac App Store case — guide the
/// user to "Install CLI") from "not installed at all".
fn not_found_state() -> TailscaleState {
    #[cfg(target_os = "macos")]
    if std::path::Path::new("/Applications/Tailscale.app").exists() {
        return TailscaleState::CliNotLinked;
    }
    TailscaleState::NotInstalled
}

/// Run the CLI and capture its output. On Windows, suppress the console
/// window a console-subsystem child would otherwise flash on screen.
fn run(cli: &PathBuf, args: &[&str]) -> std::io::Result<std::process::Output> {
    let mut cmd = Command::new(cli);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output()
}

/// Map `tailscale status --json`'s `BackendState` to what the user must do.
fn state_from_backend(backend: &str) -> TailscaleState {
    match backend {
        // Healthy-ish; the real URL only exists after `serve`.
        "Running" => TailscaleState::Healthy { url: String::new() },
        "NeedsLogin" | "NeedsMachineAuth" => TailscaleState::LoggedOut,
        "Stopped" | "NoState" | "Starting" => TailscaleState::Stopped,
        other => TailscaleState::Error {
            message: format!("Tailscale is in an unexpected state ({other})"),
        },
    }
}

/// Current node state without touching serve.
pub fn status() -> TailscaleState {
    let Some(cli) = cli() else {
        return not_found_state();
    };
    match run(&cli, &["status", "--json"]) {
        Ok(out) => {
            // A non-JSON reply means the CLI couldn't reach the background
            // service (not running, or the app was quit).
            let parsed: Option<serde_json::Value> = serde_json::from_slice(&out.stdout).ok();
            match parsed
                .as_ref()
                .and_then(|v| v.get("BackendState"))
                .and_then(|b| b.as_str())
            {
                Some(backend) => state_from_backend(backend),
                None => TailscaleState::Stopped,
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
        return not_found_state();
    };

    // Logged out, switched off, or otherwise not ready? Report that cleanly
    // rather than letting `serve` fail with a less useful message.
    let current = status();
    if !matches!(current, TailscaleState::Healthy { .. }) {
        return current;
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

    if !out.status.success() {
        let detail = stderr.trim();
        return TailscaleState::Error {
            message: if detail.is_empty() {
                "Tailscale couldn't start sharing".into()
            } else {
                detail.lines().next().unwrap_or(detail).to_string()
            },
        };
    }

    TailscaleState::Error {
        message: "serve started but no https URL was found".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_states_map_to_actions() {
        assert!(matches!(
            state_from_backend("Running"),
            TailscaleState::Healthy { .. }
        ));
        assert_eq!(state_from_backend("NeedsLogin"), TailscaleState::LoggedOut);
        assert_eq!(state_from_backend("Stopped"), TailscaleState::Stopped);
        assert!(matches!(
            state_from_backend("Weird"),
            TailscaleState::Error { .. }
        ));
    }

    #[test]
    fn finds_ts_net_url() {
        let out = "Available within your tailnet:\n\nhttps://mac.tail1234.ts.net/\n|-- proxy http://127.0.0.1:11435";
        assert_eq!(
            extract_url(out).as_deref(),
            Some("https://mac.tail1234.ts.net")
        );
    }
}
