# SPEC — MyLLM Connect (personal backend companion)

**Status:** spec v1, 2026-06-05 · **Author:** opticell team

## 1. What we're building

A small, signed **system-tray desktop app for macOS and Windows** named **MyLLM
Connect** that lets a MyLLM buyer run their own private model server and connect
their iPhone to it **in one tap, over trusted HTTPS, from anywhere** — without
touching a terminal, a certificate, or a router.

Success = from a fresh install, a non-technical user reaches "my phone is chatting
with my own PC's model over HTTPS" in **under five minutes**.

This is the **personal backend** onramp. It is explicitly **not** the federation
contributor host (see §7).

## 2. Why this first

- **It's unblocked.** Phone ↔ the user's own machine moves no content to third
  parties, so none of the federation legal/infra prerequisites apply. Shippable now.
- **It removes the #1 setup failure.** iOS rejects plain-HTTP / self-signed servers,
  so "point the app at my PC" fails for normal users today. One-click trusted HTTPS
  fixes exactly that.
- **It preserves and reinforces the financial model.** The companion is free and
  open; it is only useful with the paid MyLLM app (the access key lives in the app).
  Free onramp → paid app.
- **It's cross-platform and doesn't fork the product.** One small companion; the iOS
  app stays the single client.

## 3. The product, in one diagram

```
[ Mac/PC: MyLLM Connect ]                         [ iPhone: MyLLM ]
  Ollama (127.0.0.1:11434)
     ▲
  bearer auth proxy  ── the lock (401 without the key)
     ▲
  tailscale serve  →  https://<machine>.<tailnet>.ts.net  ──► trusted by iOS
                                   │
                          QR(myllm://pair?url&token) ──scan──► configured in one tap
```

See [`PAIRING_PROTOCOL.md`](PAIRING_PROTOCOL.md) for the exact contract.

## 4. Functional requirements (v1)

### 4.1 First-run
1. **Welcome.** One screen: "Run a private AI on this computer and use it from your
   iPhone. Your data never leaves your machine." One **Get started** button.
2. **Pick a model.** 3–5 curated models with size/RAM hints and a "good for" line.
   Download with cancellable progress.
3. **Set up secure access.** Detect Tailscale; if absent, a one-button "Install
   Tailscale" that opens its installer, then "I've signed in → continue." Companion
   runs `tailscale serve` and confirms the HTTPS endpoint is live.
4. **Pair your phone.** Big QR + 6-char backup code. "Open MyLLM → Settings → Pair a
   Backend → scan."
5. **Connected.** Green status once the app verifies the endpoint.

### 4.2 Steady state
- Tray icon: green (connected) / yellow (reconnecting) / red (server or mesh down) /
  grey (off).
- One window: **Share with my phone** toggle, status line, current model + warm/cold,
  **Re-pair** (new QR / new device / rotate key), Settings (auto-start, model swap),
  Quit.
- No chat surface, no analytics dashboard, no peer browser.

### 4.3 Background
- Optional auto-start (off by default).
- On wake / network change: re-establish `tailscale serve`, restart Ollama if it
  died (one auto-retry), update tray status without user action.
- Rotating the key invalidates old devices immediately.

## 5. Copy rules

Same discipline as the federation host: **no protocol nouns** in user-facing strings
("Tailscale mesh", "bearer token", "reverse proxy" → "secure connection", "access
key", "the link to your phone"). Surface only the one thing the user can act on.

## 6. Technical constraints (non-binding recommendations)

- **Runtime:** **Tauri (Rust)** — small, cross-platform (macOS + Windows now, Linux
  trivial later), shares a language with the opticell crates. Record as ADR-001.
- **Model server:** manage **Ollama** (detect, else guide install) for v1 — mature
  GPU/model story, least code. Bundling llama.cpp is a later option. ADR-002.
- **HTTPS:** **Tailscale-first** (decided). `tailscale serve` for the private mesh
  cert; do **not** bundle Tailscale (own signed install) — guide the user to it.
  A public Cloudflare-funnel fallback is explicitly out of scope for v1.
- **Auth proxy:** a tiny in-process Rust HTTP layer that checks the bearer key and
  forwards to `127.0.0.1:11434`, bound to the Tailscale interface only.
- **Signing:** macOS notarization + Windows EV signing before public distribution
  (unsigned = SmartScreen/Gatekeeper friction that kills the funnel). Certs are an
  opticell open question.
- **Auto-update:** signed update channel (stable + beta).
- **Telemetry:** default OFF; if any, opt-in and limited to install/first-run/error
  counts — never prompts, models, or keys.

## 7. Out of scope (v1)

- **Federation / sharing your LLM with other users** — that's the separate
  `myllm-host` workstream (Windows + opticell M7 + counsel gates). May return here as
  an optional mode later.
- **Public tunnels** (Cloudflare funnel) — private Tailscale only for v1.
- **A chat UI in the companion** — it's a backend, not a client.
- **Multi-model serving, Linux GUI build, Microsoft Store / Mac App Store SKUs** —
  later.
- **Bundling Tailscale or llama.cpp** — guide/manage, don't bundle, in v1.

## 8. iOS app dependencies

The app needs a small, contained addition (the other half of the killer feature):
- camera permission, a QR scanner, a `myllm://pair` handler, and a "Pair a Backend"
  screen that configures the server provider (URL + key) and verifies it.
- This is unblocked and lives in the MyLLM app repo; build against
  [`PAIRING_PROTOCOL.md`](PAIRING_PROTOCOL.md).

## 9. Open questions (for opticell)

1. **Signing certs** — who holds the Apple Developer ID / Windows EV cert for the
   companion.
2. **Distribution URL** — `opticell-limited.com/myllm-connect`?
3. **Curated model list** — which 3–5 models ship as the first-run recommendations.
4. **Tailscale dependency stance** — confirm we're comfortable requiring the free
   Tailscale app on both ends for v1 (decided: yes, private-first), and the exact
   first-run wording when a user declines it.

## 10. Honest sequencing

1. **Now (unblocked):** iOS pairing flow + this spec + protocol. Testable against a
   hand-run `tailscale serve` + Ollama before the companion exists.
2. **Companion v1:** Tauri app implementing §4 against the protocol.
3. **Polish:** signing, auto-update, curated models, first-run copy review.
4. **Later:** Cloudflare-funnel fallback, federation mode, Linux/Store SKUs.
