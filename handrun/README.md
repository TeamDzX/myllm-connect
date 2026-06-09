# Hand-run pairing proof

Validation artifact for the pairing protocol, run **2026-06-07** against the live
App Store build (MyLLM v2.6) — before any companion code existed. Result: **the
full scan → chat-over-HTTPS loop works.** Confirm sheet shown (token hidden),
`/api/tags` verified, model auto-selected, streaming chat over the mesh.

## What's here

- `auth_proxy.py` — minimal bearer auth proxy (issue #3 by hand): mints/persists a
  32-byte base64url token, requires `Authorization: Bearer <token>` (else 401),
  forwards to Ollama at `127.0.0.1:11434` with chunked/NDJSON streaming
  pass-through. `--rotate` mints a new token.
- `make_qr.py` — builds the `myllm://pair?v=1&…` payload per
  [`PAIRING_PROTOCOL.md`](../PAIRING_PROTOCOL.md) and renders the QR PNG.
- `token.txt` / `pair_qr.png` — generated locally, **git-ignored** (the token is a
  password; the QR contains it).

## How to reproduce

```
ollama serve                                   # model server
python auth_proxy.py                           # the lock, 127.0.0.1:11435
tailscale serve --bg 11435                     # trusted HTTPS via the mesh
python make_qr.py                              # QR -> scan in MyLLM
```

Acceptance checks that passed: no/wrong token → **401**; with token → **200**;
streaming `POST /api/chat` token-by-token through the proxy; same behavior over
`https://<machine>.<tailnet>.ts.net` with a valid Let's Encrypt cert from an
iPhone on the tailnet.

## Findings for the issues

1. **Issue #4 — missing state:** on a fresh tailnet, `tailscale serve` fails with
   *"Serve is not enabled on your tailnet"* plus an enable URL
   (`https://login.tailscale.com/f/serve?node=…`). The companion's Tailscale state
   machine needs a fifth state beyond not-installed / not-logged-in / serve-failed
   / healthy: **serve-not-enabled**, with the CLI-provided URL surfaced as the
   one-click fix.
2. **Issue #3 — binding topology:** when `tailscale serve` fronts the proxy, the
   proxy should bind to **loopback** (`127.0.0.1`), not the Tailscale interface —
   serve connects to its backend locally, and loopback is strictly tighter (the
   endpoint is unreachable from the LAN at all). The protocol's "bind to the
   Tailscale interface" wording should be read as "never `0.0.0.0`"; update it to
   cover the serve topology.
