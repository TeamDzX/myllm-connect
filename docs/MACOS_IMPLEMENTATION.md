# macOS implementation notes — MyLLM Connect

**For:** the macOS/app team · **From:** server team · **Status:** the full
pairing flow is **built and working on Windows** against the live iOS app. The
Rust core is cross-platform; this doc is the macOS-specific punch list so you can
build, sign, and ship the `.dmg`.

## What the pairing actually does (read this first)

The companion pairs the phone to **the user's own local model on the machine the
companion runs on** — *not* to any shared/opticell server. It reads the **local**
hostname, the **local** machine's tailnet node, and the **local** Ollama
(`127.0.0.1:11434`), mints a per-machine token, and renders a QR pointing at
`https://<this-mac>.<tailnet>.ts.net`. Phone → that Mac → that Mac's models.
Nothing transits our cloud. (This is the *personal backend*; federation/sharing
is the separate `myllm-host` workstream — keep the macOS copy free of any
"share with others" framing.)

So when QA scans on a given Mac, they get **that Mac's** Ollama. There is no
server selection — the machine *is* the server.

## Architecture (already implemented, `src-tauri/src/`)

All cross-platform Rust; runs as-is on macOS unless noted:

| File | Role | macOS notes |
|---|---|---|
| `proxy.rs` | Bearer auth proxy `127.0.0.1:11435` → Ollama `11434`; 401 without/wrong token; NDJSON streaming pass-through (#3) | **Portable, no change.** |
| `tailscale.rs` | Detect CLI, `tailscale serve --bg 11435`, parse the https URL; states incl. `serve_not_enabled` (#4) | **CLI path differs — see below.** |
| `pairing.rs` | Token mint/persist/rotate, `myllm://pair` payload, QR→SVG (#5) | Token at `~/Library/Application Support/MyLLM Connect/token`. Portable. |
| `lib.rs` | Tauri commands `start_sharing` / `get_status` / `rotate_key` / `open_url`; proxy starts at launch; tray + window | **Tray behaves differently — see below.** |
| `ui/` | `index.html` + `main.js`: Share button → QR; per-state guidance panels | Portable (WKWebView). Verify the SVG QR renders. |

The Windows-verified behaviour you should reproduce on macOS:
- no/wrong token → **401**; correct token → **200**, locally *and* over the
  `https://…ts.net` endpoint;
- chat **streams** (NDJSON) through the proxy;
- token is minted + persisted by the app on first launch.

## macOS-specific work

### 1. Tailscale CLI path (`tailscale.rs::candidates`)
Already lists `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, and the app
bundle (`/Applications/Tailscale.app/Contents/MacOS/Tailscale`). **Verify on a
real Mac** — the **Mac App Store** Tailscale does *not* put a CLI on `PATH`; users
must run *"Tailscale → Install CLI"* (symlinks into `/usr/local/bin`). If your
target users are MAS-Tailscale, the `not_installed` guidance copy should mention
enabling the CLI, and you may need to surface that as its own state.

### 2. `tailscale serve` permissions
First `serve` on macOS can prompt for permission / require the daemon. Confirm the
`serve_not_enabled` path (tailnet HTTPS not enabled) surfaces the admin URL the
same way it does on Windows.

### 3. Tray / menubar (`lib.rs`)
- `show_menu_on_left_click(true)` — confirm left-click opens the menu on the macOS
  menubar (Windows and macOS differ here).
- Menubar icons should be **monochrome template images** so they adapt to
  light/dark menubars. The current tray uses the full-colour app icon; swap in a
  template-rendered icon for macOS (`Image`/`set_icon_as_template`).
- `on_window_event` hides the window to the tray on close and keeps the app
  running — confirm this matches macOS expectations (you may also want to hide the
  Dock icon: set `LSUIElement`/`ActivationPolicy::Accessory` so it's menubar-only).

### 4. `open_url` (`lib.rs`)
Uses `open <url>` on macOS — already correct.

### 5. Build / package / sign (you own this)
- `npm ci && npx tauri build` on macOS → `.dmg` + `.app` (targets already set in
  `tauri.conf.json`).
- **Signing + notarization** with your Apple Developer ID (the SPEC §9 open
  question). Unsigned = Gatekeeper blocks it.
- `tauri.conf.json` → `bundle.macOS.signingIdentity` is `null` (placeholder); set
  it, plus `minimumSystemVersion` is `10.15`.
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
own Ollama, in under 5 minutes. Endpoint returns 401 without the paired key.

## Open follow-ups (both platforms, not blocking)
- Start Menu / Applications shortcut + optional **auto-start on login**.
- A real first-run wizard (#7) — current UI is the single Share view, not the
  multi-step onboarding in SPEC §4.1.
- Ollama detect/install guidance (#6) — currently assumes Ollama is already
  running; `start_sharing` reports `ollama_up: false` if not, but there's no
  install flow yet.
