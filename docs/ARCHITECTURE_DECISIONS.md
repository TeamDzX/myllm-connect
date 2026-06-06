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

## ADR-001: runtime & language
- Status: **proposed** (decide in issue #1)
- Date: —
- Context: cross-platform desktop tray app (macOS + Windows now, Linux later).
- Decision: _TBD._ Opticell recommendation: **Tauri (Rust)** — small binary,
  cross-platform, shares a language with the opticell crates. Alternatives:
  Electron, .NET/WinUI, Flutter (see SPEC §6).
- Consequences: _fill in once chosen._

## ADR-002: model-server strategy
- Status: **proposed** (decide in issue #2)
- Date: —
- Context: the companion needs a local model server to expose.
- Decision: _TBD._ Recommendation: **manage Ollama** for v1 (detect, else guide
  install); bundling llama.cpp is a later option (SPEC §6).
- Consequences: _fill in once chosen._

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
