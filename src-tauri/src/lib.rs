//! MyLLM Connect — desktop companion.
//!
//! v0.1 vertical slice: the proven pairing loop, in-process.
//!   #3 bearer auth proxy  ·  #4 tailscale serve  ·  #5 QR pairing
//! Driven from the window by a "Share with my phone" button.

mod pairing;
mod proxy;
mod tailscale;

use serde::Serialize;
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State, WindowEvent,
};
use tokio::sync::RwLock;

use proxy::ProxyState;

/// App-wide state shared with Tauri commands.
struct AppState {
    proxy: ProxyState,
    /// Cached https URL once tailscale serve is up.
    https_url: RwLock<Option<String>>,
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
async fn start_sharing(state: State<'_, AppState>) -> Result<ShareResult, String> {
    let ollama_up = proxy::ollama_up().await;

    // tailscale serve is blocking CLI work — run it off the async runtime.
    let ts = tokio::task::spawn_blocking(|| tailscale::start_serve(proxy::PROXY_PORT))
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
    #[cfg(windows)]
    let r = std::process::Command::new("cmd")
        .args(["/C", "start", "", &url])
        .spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

/// Rotate the access key (invalidates paired devices) and re-render the QR.
#[tauri::command]
async fn rotate_key(state: State<'_, AppState>) -> Result<ShareResult, String> {
    let new_token = tokio::task::spawn_blocking(pairing::rotate)
        .await
        .map_err(|e| e.to_string())?;
    *state.proxy.token.write().await = new_token;
    start_sharing(state).await
}

pub fn run() {
    let token = pairing::load_or_mint();
    let proxy_state = ProxyState::new(token);

    // Start the auth proxy immediately on a background tokio runtime so it's
    // listening before the user clicks Share.
    let proxy_for_server = proxy_state.clone();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async {
            if let Err(e) = proxy::serve(proxy_for_server).await {
                eprintln!("auth proxy stopped: {e}");
            }
        });
    });

    tauri::Builder::default()
        .manage(AppState {
            proxy: proxy_state,
            https_url: RwLock::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            start_sharing,
            get_status,
            rotate_key,
            open_url
        ])
        .setup(|app| {
            let status = MenuItem::with_id(app, "status", "Not sharing yet", false, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "Open MyLLM Connect", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&status, &open, &quit])?;

            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().unwrap().clone())
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
                })
                .build(app)?;
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
