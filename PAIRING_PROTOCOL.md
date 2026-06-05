# MyLLM Connect — Pairing Protocol v1

This is the **contract** between the desktop companion and the MyLLM iOS app. Both
sides implement against this document so they can be built independently. It is
deliberately small.

## Goal

Get the iOS app talking to the user's own model server over a **trusted HTTPS
endpoint** with a **bearer access key**, via a single QR scan — and make the
endpoint useless to anything but the paired app.

## Roles

- **Companion** — the Mac/PC tray app. Runs the model server, brings up the HTTPS
  endpoint, mints the access key, renders the pairing payload as a QR.
- **App** — MyLLM on iOS. Scans the QR (or opens the deep link), confirms with the
  user, stores the endpoint + key, and uses them as a standard server provider.

## The endpoint (companion side)

1. Run the model server locally (Ollama) on `127.0.0.1:11434`.
2. Run a tiny **auth proxy** in front of it that requires
   `Authorization: Bearer <token>` and forwards authorized requests to
   `127.0.0.1:11434`. Unauthorized requests get `401`. **This proxy is the lock.**
3. Expose the auth proxy over **Tailscale Serve** as HTTPS:
   `https://<machine>.<tailnet>.ts.net/`. Tailscale provisions a valid
   Let's Encrypt certificate that iOS trusts natively, and routes directly over the
   LAN when both devices are on the same network, or over the mesh when remote.

The app therefore talks to the same Ollama HTTP API it already supports
(`GET /api/tags`, `POST /api/chat`) — now at an HTTPS base URL, with a Bearer key.
No new server API is introduced.

## The pairing payload

A single URL using MyLLM's existing custom scheme, so scanning it (or tapping it on
the device) opens the app directly:

```
myllm://pair?v=1
  &name=<url-encoded display name, e.g. "Alex's Mac mini">
  &url=<url-encoded https base, e.g. https://mac-mini.tailnet-1234.ts.net>
  &token=<url-encoded bearer access key>
  &models=<optional comma-separated model ids, e.g. qwen2.5:14b,llama3.1:8b>
  &fp=<optional sha256 fingerprint of the cert, reserved for future pinning>
```

- `v` — protocol version (`1`).
- `token` — a high-entropy secret (≥ 32 random bytes, base64url). It is a password;
  treat it as one. It is shown only as part of the QR on the user's own screen.
- The QR encodes this exact URL. The companion also shows a short **backup code**
  (6 chars) the user can type if the camera path fails; it maps to the same payload
  over the local network during the pairing window.

## The flow

```
Companion                                  App (MyLLM on iOS)
  pick model, start Ollama
  start auth proxy (mints token)
  tailscale serve  → https URL + cert
  render QR(myllm://pair?…)        ──scan──►  parse payload
                                              show confirm sheet (name + url only;
                                                token never displayed)
                                   ◄─user taps "Connect"──
                                              save as a server provider:
                                                providerName = server
                                                serverURL    = url
                                                apiKey       = token
                                              GET {url}/api/tags  (verify, expect 200)
                                              select first model (or models[0])
  steady-state: green "Connected"            chat works over HTTPS, Bearer-locked
```

## Confirmation, not silent apply

Like every other `myllm://` install link, pairing **always shows a confirmation
sheet** before writing anything. The sheet shows the backend name and URL; it never
displays the token. The user taps **Connect** to apply.

## Rotation & unpair

- **Rotate:** the companion can mint a new token and re-render the QR; the old token
  is invalidated immediately. Re-scan to reconnect.
- **Unpair:** removing the server in the app stops using it; rotating on the
  companion locks out any device still holding the old key.

## Security notes

- The token is the only credential; the HTTPS URL alone grants nothing (`401`).
- The auth proxy should bind to the Tailscale interface, not `0.0.0.0`, so the
  endpoint isn't even reachable on the local LAN without both the mesh and the token.
- Tailnet traffic is end-to-end WireGuard; no third party sees prompt content on the
  private path. (A future public-funnel option would still require the token but
  would terminate TLS at the edge — out of scope for v1, and would be disclosed.)
- `fp` (cert fingerprint) is reserved so the app can optionally pin the endpoint in a
  later version; v1 relies on the public-CA trust chain.

## iOS side — what v1 needs (new)

- `NSCameraUsageDescription` (for the QR scanner).
- A QR scanner view (AVFoundation metadata capture).
- A `myllm://pair` handler alongside the existing `install-tool` / `install-skill`
  handlers, staging a confirmation sheet.
- On confirm: configure the server provider (URL + key) and verify with `/api/tags`.

## Companion side — what v1 needs

- Detect/instal/manage Ollama; pull the chosen model.
- Mint token; run the bearer auth proxy → `127.0.0.1:11434`.
- Drive `tailscale serve` (and prompt the user to install/sign in to Tailscale once).
- Render the QR + backup code; show tray status (connected / reconnecting / off).
