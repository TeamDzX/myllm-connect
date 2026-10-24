//! MyLLM Connect — desktop companion.
//!
//! The pairing loop, in-process:
//!   #3 bearer auth proxy  ·  #4 tailscale serve  ·  #5 QR pairing
//! plus the steady-state lifecycle (#8): resume sharing on launch, optional
//! login auto-start, keep-awake while sharing, and a periodic health check
//! that re-establishes `tailscale serve` after sleep or a network change.

mod awake;
mod ollama;
mod pairing;
mod proxy;
mod settings;
mod tailscale;

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent, Wry,
};
use tauri_plugin_autostart::ManagerExt;
use tokio::sync::RwLock;

use proxy::ProxyState;

/// Passed by the login item, so a login launch stays in the tray.
const AUTOSTART_ARG: &str = "--autostart";
/// How often to re-check the link while sharing.
const HEALTH_INTERVAL: Duration = Duration::from_secs(60);

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
    settings: Mutex<settings::Settings>,
    awake: awake::KeepAwake,
}

/// Tray items we update as state changes.
struct Tray {
    status: MenuItem<Wry>,
    stop: MenuItem<Wry>,
    login: CheckMenuItem<Wry>,
    awake: CheckMenuItem<Wry>,
}

#[derive(Serialize, Clone)]
struct ShareResult {
    /// Tailscale node state (tagged enum: not_installed / logged_out /
    /// stopped / serve_not_enabled / healthy / error).
    tailscale: tailscale::TailscaleState,
    /// Present when ready to pair.
    pair_url: Option<String>,
    qr_svg: Option<String>,
    model: Option<String>,
    ollama_up: bool,
}

/// User preferences as the window and tray show them.
#[derive(Serialize, Clone)]
struct Prefs {
    autostart: bool,
    keep_awake: bool,
    sharing: bool,
}

/// Reflect the current state in the tray's status line and tooltip.
fn set_tray_status(app: &AppHandle, text: &str) {
    if let Some(tray) = app.try_state::<Tray>() {
        let _ = tray.status.set_text(text);
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

fn prefs(app: &AppHandle) -> Prefs {
    let state = app.state::<AppState>();
    let s = state.settings.lock().unwrap();
    Prefs {
        autostart: app.autolaunch().is_enabled().unwrap_or(false),
        keep_awake: s.keep_awake,
        sharing: s.sharing,
    }
}

/// Push preferences to the tray checkmarks and the window.
fn sync_prefs(app: &AppHandle) -> Prefs {
    let p = prefs(app);
    if let Some(tray) = app.try_state::<Tray>() {
        let _ = tray.login.set_checked(p.autostart);
        let _ = tray.awake.set_checked(p.keep_awake);
        let _ = tray.stop.set_enabled(p.sharing);
    }
    let _ = app.emit("prefs-changed", &p);
    p
}

/// Record whether sharing is on (persisted, so it resumes on launch) and
/// hold or release keep-awake to match.
fn set_sharing(state: &AppState, on: bool) {
    let keep_awake = {
        let mut s = state.settings.lock().unwrap();
        if s.sharing != on {
            s.sharing = on;
            settings::save(&s);
        }
        s.keep_awake
    };
    state.awake.set(on && keep_awake);
}

/// Start (or re-confirm) sharing: ensure Ollama is up, bring up tailscale serve,
/// and produce the pairing QR. The proxy itself is already listening (started at
/// launch), so this is idempotent.
#[tauri::command]
async fn start_sharing(app: AppHandle, state: State<'_, AppState>) -> Result<ShareResult, String> {
    let res = share(&state).await;
    after_share(&app, &state, &res);
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

/// Tray + persisted state after a share attempt. A failed attempt keeps the
/// user's intent to share, so the health check keeps retrying it.
fn after_share(app: &AppHandle, state: &AppState, res: &Result<ShareResult, String>) {
    match res {
        Ok(r) => {
            set_tray_status(app, tray_text(r));
            if r.pair_url.is_some() {
                set_sharing(state, true);
            }
        }
        Err(_) => set_tray_status(app, "Action needed"),
    }
    sync_prefs(app);
}

/// Stop sharing: take down `tailscale serve` and forget the intent to share.
#[tauri::command]
async fn stop_sharing(app: AppHandle, state: State<'_, AppState>) -> Result<Prefs, String> {
    stop_serving(&app, &state).await?;
    Ok(prefs(&app))
}

async fn stop_serving(app: &AppHandle, state: &AppState) -> Result<(), String> {
    let res = tokio::task::spawn_blocking(tailscale::stop_serve)
        .await
        .map_err(|e| e.to_string())?;
    if let Err(e) = res {
        set_tray_status(app, "Action needed: couldn't stop sharing");
        return Err(format!("Couldn't stop sharing: {e}"));
    }
    set_sharing(state, false);
    *state.https_url.write().await = None;
    set_tray_status(app, "Not sharing");
    sync_prefs(app);
    Ok(())
}

#[tauri::command]
fn get_prefs(app: AppHandle) -> Prefs {
    prefs(&app)
}

/// Turn login auto-start on or off (the OS stores this, not our settings).
#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<Prefs, String> {
    let launcher = app.autolaunch();
    let r = if on {
        launcher.enable()
    } else {
        launcher.disable()
    };
    let p = sync_prefs(&app);
    r.map(|_| p).map_err(|e| e.to_string())
}

#[tauri::command]
fn set_keep_awake(app: AppHandle, state: State<'_, AppState>, on: bool) -> Prefs {
    let sharing = {
        let mut s = state.settings.lock().unwrap();
        s.keep_awake = on;
        settings::save(&s);
        s.sharing
    };
    state.awake.set(on && sharing);
    sync_prefs(&app)
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

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Resume sharing if it was on when we last quit, then keep it healthy:
/// re-running `serve` is idempotent and recovers it after sleep/wake or a
/// network change. The window re-renders from `share-updated` if it's open.
fn spawn_lifecycle(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            let state = app.state::<AppState>();
            let sharing = state.settings.lock().unwrap().sharing;
            if sharing {
                let res = share(&state).await;
                after_share(&app, &state, &res);
                if let Ok(r) = res {
                    let _ = app.emit("share-updated", r);
                }
            }
            tokio::time::sleep(HEALTH_INTERVAL).await;
        }
    });
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

    let launched_at_login = std::env::args().any(|a| a == AUTOSTART_ARG);

    tauri::Builder::default()
        // Must be registered first: a second launch (e.g. opening the app while
        // the login item already started it) just brings this window forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }))
        // macOS uses a LaunchAgent (the plugin's default); Windows a Run key.
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args([AUTOSTART_ARG])
                .build(),
        )
        .manage(AppState {
            proxy: proxy_state,
            https_url: RwLock::new(None),
            port,
            proxy_error,
            settings: Mutex::new(settings::load()),
            awake: awake::KeepAwake::default(),
        })
        .invoke_handler(tauri::generate_handler![
            start_sharing,
            stop_sharing,
            get_status,
            rotate_key,
            open_url,
            ollama_status,
            is_onboarded,
            set_onboarded,
            get_prefs,
            set_autostart,
            set_keep_awake
        ])
        .setup(move |app| {
            // macOS: run as a menubar-only accessory — no Dock icon, no app menu
            // (equivalent to LSUIElement). The tray is the whole UI surface.
            #[cfg(target_os = "macos")]
            let _ = app
                .handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory);

            let status = MenuItem::with_id(app, "status", "Not sharing yet", false, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "Open MyLLM Connect", true, None::<&str>)?;
            let stop = MenuItem::with_id(app, "stop", "Stop sharing", false, None::<&str>)?;
            let login =
                CheckMenuItem::with_id(app, "login", "Open at login", true, false, None::<&str>)?;
            let awake = CheckMenuItem::with_id(
                app,
                "awake",
                "Keep computer awake while sharing",
                true,
                false,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[
                    &status,
                    &PredefinedMenuItem::separator(app)?,
                    &open,
                    &stop,
                    &PredefinedMenuItem::separator(app)?,
                    &login,
                    &awake,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?;
            app.manage(Tray {
                status,
                stop,
                login,
                awake,
            });

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
                    "open" => show_window(app),
                    "stop" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = app.state::<AppState>();
                            let _ = stop_serving(&app, &state).await;
                        });
                    }
                    // Check items toggle themselves on click; apply the change,
                    // then sync_prefs resets the mark to what actually took.
                    "login" => {
                        let on = !prefs(app).autostart;
                        let _ = set_autostart(app.clone(), on);
                    }
                    "awake" => {
                        let on = !prefs(app).keep_awake;
                        set_keep_awake(app.clone(), app.state::<AppState>(), on);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            // Tell macOS to tint the icon as a template (alpha-defined shape).
            #[cfg(target_os = "macos")]
            let tray = tray.icon_as_template(true);
            tray.build(app)?;

            sync_prefs(app.handle());
            // The window starts hidden (tauri.conf.json); a login launch stays
            // in the tray, a normal launch shows it.
            if !launched_at_login {
                show_window(app.handle());
            }
            spawn_lifecycle(app.handle().clone());
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
