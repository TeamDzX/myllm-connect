# Architecture Decision Records

Short, append-only records of the significant choices. One ADR per decision.
Keep them brief: context, the decision, and why. Supersede rather than rewrite.

Template:

```
## ADR-NNN: <title>
- Status: proposed | accepted | superseded by ADR-MMM
- Date: YYYY-MM-DD
- Context: what forced a decision.
- Decision: what we chose.
- Consequences: what this makes easy/hard; what we're accepting.
```

---

## ADR-001: runtime & language — Tauri (Rust)
- Status: **accepted**
- Date: 2026-06-07
- Context: cross-platform desktop tray app (macOS + Windows now, Linux later).
  The app is mostly *not* UI: it's lifecycle state machines (Ollama, Tailscale,
  pairing) around an in-process HTTP auth proxy, with a small first-run wizard
  and a tray menu on top.
- Decision: **Tauri 2 (Rust).** The auth proxy — the one component with real
  correctness/security weight (bearer check, streaming pass-through, loopback
  binding) — lives in-process in Rust (hyper/axum) instead of being a sidecar
  or a Node HTTP layer. Tray + autostart are first-class Tauri plugins; the
  wizard is a small webview UI. Binary stays ~MBs (Electron ships a ~100MB+
  Chromium per install), Linux later is a build target not a port, and the
  language is shared with the opticell crates. .NET/WinUI was rejected as
  Windows-first (macOS tray story is weak); Flutter rejected because desktop
  tray/menubar support still leans on third-party plugins.
  The hand-run pairing proof (2026-06-07, `handrun/`) showed the protocol
  surface is small — a ~150-line Python proxy passed the live iOS app — so
  runtime choice is driven by packaging/footprint/signing, not framework
  features; that favours the smallest signed artifact.
- Consequences: Rust learning curve for contributors; webview differences
  (WebView2 vs WKWebView) need testing on both OSes — acceptable because the
  UI surface is deliberately tiny; CI needs macOS + Windows runners from day
  one (issue #1 acceptance). Tauri's updater plugin gives us the signed
  update channel needed by issue #9.

## ADR-002: model-server strategy — manage Ollama
- Status: **accepted**
- Date: 2026-06-07
- Context: the companion needs a local model server to expose. Options:
  manage an external Ollama vs bundle llama.cpp.
- Decision: **Manage Ollama** (detect, else guide install) for v1. Ollama owns
  the hard parts — GPU detection/offload across CUDA/Metal/ROCm, model
  registry, quantization choices, resident-model lifecycle — and the iOS app
  already speaks its API verbatim, so the companion adds zero translation
  (confirmed end-to-end by the hand-run proof: the live app chatted through a
  dumb pass-through proxy). Flow: probe `127.0.0.1:11434/api/version`; if
  absent, open the official installer and re-probe ("I've installed it →
  continue", same pattern as the Tailscale step); enforce a minimum version;
  pull curated models via `/api/pull` with cancellable progress (SPEC §4.1.2).
- Consequences: easy — least code, mature model/GPU story, model pulls and
  warm/cold status come free from the API. Accepting: a second third-party
  install in first-run (Tailscale + Ollama; both have polished installers),
  no control over Ollama's own UX/regressions, and version-skew risk
  (mitigated by the minimum-version gate). Bundling llama.cpp stays a later
  option if the double-install measurably hurts the <5-minute funnel.

## ADR-003: HTTPS transport — Tailscale-first
- Status: **accepted**
- Date: 2026-06-05
- Context: iOS only trusts a valid certificate; a plain-HTTP/self-signed home
  server is rejected by the app. We need a trusted HTTPS endpoint with no
  port-forwarding, and the product is privacy-first.
- Decision: **Tailscale-first.** Use `tailscale serve` to expose the bearer
  auth proxy as `https://<machine>.<tailnet>.ts.net` with a valid Let's Encrypt
  cert. Do **not** bundle Tailscale (its own signed install). A public
  Cloudflare-funnel fallback is **out of scope for v1**.
- Consequences: most private (WireGuard mesh, no third party in the path) and it
  works on the LAN and remotely. Cost: the phone installs the free Tailscale app
  once. Revisit a public fallback only if onboarding data shows it's needed.
