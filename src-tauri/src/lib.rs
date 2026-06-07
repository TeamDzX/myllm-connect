//! MyLLM Connect — issue #1 hello-world: a tray app that launches on both OSes.
//! The tray states (green/yellow/red/grey, SPEC §4.2) are stubbed as a status
//! menu line until #3/#4 wire in the real proxy + Tailscale state machines.

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, WindowEvent,
};

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            // SPEC §5 copy rules: user-facing strings stay protocol-noun-free.
            let status =
                MenuItem::with_id(app, "status", "Not sharing yet", false, None::<&str>)?;
            let open =
                MenuItem::with_id(app, "open", "Open MyLLM Connect", true, None::<&str>)?;
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
            // Closing the window hides to tray; the app keeps running (SPEC §4.2).
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running MyLLM Connect");
}
