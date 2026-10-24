//! MyLLM Connect — desktop companion.
//!
//! v0.1 vertical slice: the proven pairing loop, in-process.
//!   #3 bearer auth proxy  ·  #4 tailscale serve  ·  #5 QR pairing
//! Driven from the window by a "Share with my phone" button.

mod ollama;
mod pairing;
mod proxy;
mod tailscale;

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager, State, WindowEvent, Wry,
};
use tokio::sync::RwLock;

use proxy::ProxyState;

/// App-wide state shared with Tauri commands.
struct AppState {
    proxy: ProxyState,
    /// Cached https URL once tailscale serve is up.
    https_url: RwLock<Option<String>>,
    /// Port the proxy actually bound (the preferred one may have been taken).
    port: u16,
    /// Set when the proxy couldn't start at all — sharing would hand the phone
    /// a QR for a dead endpoint, so we surface this instead.
    proxy_error: Option<String>,
}

/// The tray's disabled first line, kept so we can update it as state changes.
struct TrayStatus(MenuItem<Wry>);

/// Reflect the current state in the tray's status line and tooltip.
fn set_tray_status(app: &AppHandle, text: &str) {
    if let Some(item) = app.try_state::<TrayStatus>() {
        let _ = item.0.set_text(text);
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(format!("MyLLM Connect — {text}")));
    }
}

/// One-line tray summary of a share attempt.
fn tray_text(res: &ShareResult) -> &'static str {
    if res.pair_url.is_some() {
        "Sharing — ready to pair"
    } else if !res.ollama_up || res.model.is_none() {
        "Action needed: local model"
    } else {
        "Action needed: secure link"
    }
}

#[derive(Serialize)]
struct ShareResult {
    /// Tailscale node state (tagged enum: not_installed / logged_out /
    /// serve_not_enabled / healthy / error).
    tailscale: tailscale::TailscaleState,
    /// Present when ready to pair.
    pair_url: Option<String>,
    qr_svg: Option<String>,
    model: Option<String>,
    ollama_up: bool,
}

/// Start (or re-confirm) sharing: ensure Ollama is up, bring up tailscale serve,
/// and produce the pairing QR. The proxy itself is already listening (started at
/// launch), so this is idempotent.
#[tauri::command]
async fn start_sharing(app: AppHandle, state: State<'_, AppState>) -> Result<ShareResult, String> {
    let res = share(&state).await;
    match &res {
        Ok(r) => set_tray_status(&app, tray_text(r)),
        Err(_) => set_tray_status(&app, "Action needed"),
    }
    res
}

async fn share(state: &AppState) -> Result<ShareResult, String> {
    if let Some(err) = &state.proxy_error {
        return Err(format!("MyLLM Connect couldn't start its local service: {err}. Quit any other copy of MyLLM Connect, then reopen it."));
    }
    let ollama_up = proxy::ollama_up().await;

    // tailscale serve is blocking CLI work — run it off the async runtime.
    let port = state.port;
    let ts = tokio::task::spawn_blocking(move || tailscale::start_serve(port))
        .await
        .map_err(|e| e.to_string())?;

    let model = proxy::first_model().await;

    if let tailscale::TailscaleState::Healthy { url } = &ts {
        *state.https_url.write().await = Some(url.clone());
        let token = state.proxy.token.read().await.clone();
        let pair = pairing::pair_url(url, &token, model.as_deref());
        let qr = pairing::qr_svg(&pair).ok();
        return Ok(ShareResult {
            tailscale: ts,
            pair_url: Some(pair),
            qr_svg: qr,
            model,
            ollama_up,
        });
    }

    Ok(ShareResult {
        tailscale: ts,
        pair_url: None,
        qr_svg: None,
        model,
        ollama_up,
    })
}

/// Just the current node state (no serve side effects) — for status polling.
#[tauri::command]
async fn get_status() -> Result<tailscale::TailscaleState, String> {
    tokio::task::spawn_blocking(tailscale::status)
        .await
        .map_err(|e| e.to_string())
}

/// Open an external URL in the user's default browser (Tailscale install /
/// admin-console enable links from the guidance panel).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    // Only the links the guidance panels use: web pages, and the Tailscale app.
    if !(url.starts_with("https://") || url == "/Applications/Tailscale.app") {
        return Err("refusing to open that link".into());
    }
    // Not `cmd /C start`: cmd would treat `&` in a URL as a command separator.
    #[cfg(windows)]
    let r = std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", &url])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// Local-model readiness for the first-run flow (installed / running / model).
#[tauri::command]
async fn ollama_status() -> ollama::OllamaState {
    ollama::status().await
}

/// Whether the first-run flow has been completed before.
#[tauri::command]
fn is_onboarded() -> bool {
    pairing::onboarded()
}

/// Record that the first-run flow is complete.
#[tauri::command]
fn set_onboarded() {
    pairing::mark_onboarded();
}

/// Rotate the access key (invalidates paired devices) and re-render the QR.
#[tauri::command]
async fn rotate_key(app: AppHandle, state: State<'_, AppState>) -> Result<ShareResult, String> {
    let new_token = tokio::task::spawn_blocking(pairing::rotate)
        .await
        .map_err(|e| e.to_string())?;
    *state.proxy.token.write().await = new_token;
    start_sharing(app, state).await
}

pub fn run() {
    let token = pairing::load_or_mint();
    let proxy_state = ProxyState::new(token);

    // Bind now, so a taken port is known before the user clicks Share; then
    // serve on a background tokio runtime.
    let (port, proxy_error) = match proxy::bind() {
        Ok((listener, port)) => {
            let proxy_for_server = proxy_state.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(async {
                    if let Err(e) = proxy::serve(listener, proxy_for_server).await {
                        eprintln!("auth proxy stopped: {e}");
                    }
                });
            });
            (port, None)
        }
        Err(e) => (proxy::PROXY_PORT, Some(e)),
    };

    tauri::Builder::default()
        .manage(AppState {
            proxy: proxy_state,
            https_url: RwLock::new(None),
            port,
            proxy_error,
        })
        .invoke_handler(tauri::generate_handler![
            start_sharing,
            get_status,
            rotate_key,
            open_url,
            ollama_status,
            is_onboarded,
            set_onboarded
        ])
        .setup(|app| {
            // macOS: run as a menubar-only accessory — no Dock icon, no app menu
            // (equivalent to LSUIElement). The tray is the whole UI surface.
            #[cfg(target_os = "macos")]
            let _ = app
                .handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory);

            let status = MenuItem::with_id(app, "status", "Not sharing yet", false, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "Open MyLLM Connect", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&status, &open, &quit])?;
            app.manage(TrayStatus(status));

            // Menubar icon: a monochrome *template* on macOS so it adapts to
            // light/dark menubars; the full-colour app icon on other platforms.
            #[cfg(target_os = "macos")]
            let tray_icon =
                tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;
            #[cfg(not(target_os = "macos"))]
            let tray_icon = app.default_window_icon().unwrap().clone();

            let tray = TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .tooltip("MyLLM Connect")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            // Tell macOS to tint the icon as a template (alpha-defined shape).
            #[cfg(target_os = "macos")]
            let tray = tray.icon_as_template(true);
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running MyLLM Connect");
}
