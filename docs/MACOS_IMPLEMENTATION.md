# macOS implementation notes — MyLLM Connect

**For:** the macOS/app team · **From:** server team · **Status:** the full pairing
flow is built and has been validated against the live iOS app on macOS with
llama.cpp, and on Windows with Ollama. The Rust core is cross-platform; this doc
captures the macOS-specific build and release work.

## What the pairing actually does (read this first)

The companion pairs the phone to **the user's own local model on the machine the
companion runs on** — *not* to any shared/opticell server. It reads the **local**
hostname, the **local** machine's tailnet node, and a **local** Ollama or
llama.cpp server, mints a per-machine token, and renders a QR pointing at
`https://<this-mac>.<tailnet>.ts.net`. Phone → that Mac → that Mac's models.
Nothing transits our cloud. (This is the *personal backend*; federation/sharing
is the separate `myllm-host` workstream — keep the macOS copy free of any
"share with others" framing.)

So when QA scans on a given Mac, they get **that Mac's** model server. There is
no remote server selection — the machine *is* the server.

## Architecture (already implemented, `src-tauri/src/`)

All cross-platform Rust; runs as-is on macOS unless noted:

| File | Role | macOS notes |
|---|---|---|
| `backend.rs` | Detect Ollama/llama.cpp and translate llama.cpp OpenAI responses to the iOS Ollama contract | Loopback-only; defaults include llama.cpp ports 8080 and 8081. |
| `model_server.rs` | Backend-neutral readiness state for the first-run flow | **Portable.** |
| `proxy.rs` | Bearer auth proxy on `127.0.0.1:11435`; Ollama pass-through or llama.cpp translation; NDJSON streaming | **Portable.** |
| `tailscale.rs` | Detect CLI, `tailscale serve --bg 11435`, parse the https URL; states incl. `serve_not_enabled` (#4) | **CLI path differs — see below.** |
| `pairing.rs` | Token mint/persist/rotate, `myllm://pair` payload, QR→SVG (#5) | Token at `~/Library/Application Support/MyLLM Connect/token`. Portable. |
| `lib.rs` | Tauri commands `start_sharing` / `get_status` / `rotate_key` / `open_url`; proxy starts at launch; tray + window | **Tray behaves differently — see below.** |
| `ui/` | `index.html` + `main.js`: Share button → QR; per-state guidance panels | Portable (WKWebView). Verify the SVG QR renders. |

Cross-platform behavior to preserve:
- no/wrong token → **401**; correct token → **200**, locally *and* over the
  `https://…ts.net` endpoint;
- chat **streams** (NDJSON) through the proxy for both backends;
- token is minted + persisted by the app on first launch.

## macOS-specific work

### 1. Tailscale CLI path (`tailscale.rs::candidates`)
Already lists `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, and the app
bundle (`/Applications/Tailscale.app/Contents/MacOS/Tailscale`). **Verify on a
real Mac** — the **Mac App Store** Tailscale does *not* put a CLI on `PATH`; users
must run *"Tailscale → Install CLI"* (symlinks into `/usr/local/bin`). The
`cli_not_linked` state and UI guidance cover this case.

### 2. `tailscale serve` permissions
First `serve` on macOS can prompt for permission / require the daemon. Confirm the
`serve_not_enabled` path (tailnet HTTPS not enabled) surfaces the admin URL the
same way it does on Windows.

### 3. Tray / menubar (`lib.rs`)
- `show_menu_on_left_click(true)` opens the menubar menu.
- macOS uses `icons/tray-template.png` with `icon_as_template(true)` so the icon
  adapts to light and dark menubars.
- `ActivationPolicy::Accessory` hides the Dock icon, and closing the window hides
  it while the menubar process remains active.

### 4. `open_url` (`lib.rs`)
Uses `open <url>` on macOS — already correct.

### 5. Build / package / sign (you own this)
- `npm ci && npx tauri build` on macOS → `.dmg` + `.app` (targets already set in
  `tauri.conf.json`).
- **Signing + notarization** with your Apple Developer ID (the SPEC §9 open
  question). Unsigned = Gatekeeper blocks it.
- `tauri.conf.json` names the release signing identity. The matching Developer ID
  certificate must be present; override the identity with `null` only for local,
  ad-hoc-signed test builds. `minimumSystemVersion` is `10.15`.
- **CI:** `.github/workflows/release.yml` has a `macos` job that is **`if: false`**.
  Flip it to `true` and add the secrets it documents
  (`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`,
  `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID`) when you want CI to build the dmg.
  Until then you build locally on your Macs.

### 6. Icons
The icon set was generated from `brand/logo.png` (includes `icon.icns`). Replace
with final brand art before release if needed; regenerate with
`npx tauri icon brand/logo.png`.

## Definition of done (macOS, mirrors the Windows result)
From a clean Mac: install the `.dmg` → launch → click **Share with my phone** →
QR appears → scan in MyLLM → chat over HTTPS, Bearer-locked, against that Mac's
own Ollama or llama.cpp server, in under 5 minutes. Endpoint returns 401 without
the paired key.

## Open follow-ups (both platforms, not blocking)
- Start Menu / Applications shortcut + optional **auto-start on login**.
- A settings control for a non-default llama.cpp port; today use
  `MYLLM_LLAMA_CPP_URL` for ports other than 8080/8081.
