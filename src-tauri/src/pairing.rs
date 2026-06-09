//! Token minting + the `myllm://pair` payload + QR rendering (issue #5).

use std::path::PathBuf;

use base64::Engine;
use qrcode::render::svg;
use qrcode::{EcLevel, QrCode};
use rand::RngCore;

/// 32 random bytes, base64url — a password. Matches PAIRING_PROTOCOL.md.
pub fn mint_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Where the token persists between runs (per-user app data).
pub fn token_path() -> PathBuf {
    let base = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("MyLLM Connect").join("token")
}

/// Load the persisted token, or mint+persist a fresh one.
pub fn load_or_mint() -> String {
    let path = token_path();
    if let Ok(t) = std::fs::read_to_string(&path) {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return t;
        }
    }
    let token = mint_token();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, &token);
    token
}

/// Overwrite with a fresh token (rotation); returns the new value.
pub fn rotate() -> String {
    let path = token_path();
    let token = mint_token();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, &token);
    token
}

fn display_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "my computer".into())
}

/// Build the `myllm://pair?...` deep link per PAIRING_PROTOCOL.md.
pub fn pair_url(https_url: &str, token: &str, model: Option<&str>) -> String {
    let enc = |s: &str| urlencoding::encode(s).into_owned();
    let mut url = format!(
        "myllm://pair?v=1&name={}&url={}&token={}",
        enc(&display_name()),
        enc(https_url),
        enc(token),
    );
    if let Some(m) = model {
        url.push_str(&format!("&models={}", enc(m)));
    }
    url
}

/// Render a pairing payload as an inline SVG string (for the webview).
pub fn qr_svg(payload: &str) -> Result<String, String> {
    let code =
        QrCode::with_error_correction_level(payload, EcLevel::M).map_err(|e| e.to_string())?;
    let svg = code
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(svg::Color("#0a0a0a"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(svg)
}
