# MyLLM Connect

[![Download MyLLM on the App Store](https://img.shields.io/badge/MyLLM-App_Store-0A84FF?logo=apple&logoColor=white)](https://apps.apple.com/gb/app/myllm-local-ai-agent/id6760704297)

**Run your own private AI backend on your Mac or PC, and connect the [MyLLM](https://apps.apple.com/gb/app/myllm-local-ai-agent/id6760704297) iOS app to it in one tap — over real HTTPS, from anywhere.**

MyLLM Connect is a small desktop companion (system-tray app for macOS and Windows) that turns "set up a local LLM server my phone can reach" from a multi-step, HTTPS-and-firewall headache into a single QR scan.

It does three things:

1. **Runs a local model server** on your machine (manages Ollama for you).
2. **Gives it a trusted HTTPS address** your iPhone accepts — privately, over your own [Tailscale](https://tailscale.com) mesh, with a valid certificate. No port-forwarding, no self-signed-cert warnings, works on your home WiFi and when you're out.
3. **Pairs to MyLLM with a QR code** — scan once and the app is configured with the address and a private access key.

Your prompts go straight from your phone to your own machine. Nothing runs in our cloud; we never see your data.

## Why HTTPS matters

iOS only trusts a *valid* certificate. A plain `http://192.168.x.x:11434` Ollama server is rejected by the app's transport security — which is why "just point the app at my PC" usually fails today. MyLLM Connect solves this by giving your machine a real, trusted HTTPS endpoint automatically.

## Locked to MyLLM

The endpoint is useless without the access key minted during pairing, and that key only lives inside your paired MyLLM app. The companion is free and open; the experience it unlocks is the MyLLM app. (See [`PAIRING_PROTOCOL.md`](PAIRING_PROTOCOL.md).)

## Status

Early — this repo currently contains the spec and the pairing protocol. Implementation tracked in issues. See [`SPEC.md`](SPEC.md).

## Not the federation host

This is the **personal backend** onramp: your phone, your server, your data. It is intentionally separate from the opticell *federation* host (sharing your LLM with other users), which has its own repo and its own legal/infra prerequisites. Federation may later appear here as an optional mode; v1 is personal-only.

## License

[Apache-2.0](LICENSE).
