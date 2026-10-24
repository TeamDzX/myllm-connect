# Contributing to MyLLM Connect

Thanks for helping. MyLLM Connect is a small Tauri 2 tray app (Rust in
`src-tauri/src/`, plain HTML/JS in `ui/`) for macOS and Windows.

## Before you start

- Read [`SPEC.md`](SPEC.md) and [`PAIRING_PROTOCOL.md`](PAIRING_PROTOCOL.md):
  the pairing payload is shared with the MyLLM iOS app, so changes to it need
  agreement first.
- For anything bigger than a bug fix, open an issue describing the idea so we
  can agree on the approach before you write code.

## Building

```sh
npm ci
npx tauri dev        # run locally
npx tauri build      # installer for your OS
```

You'll need [Ollama](https://ollama.com) and [Tailscale](https://tailscale.com)
installed to exercise the full pairing flow.

## Pull requests

CI runs these on macOS and Windows; please run them before pushing:

```sh
cd src-tauri
cargo fmt --check
cargo clippy --no-deps --all-targets -- -D warnings
cargo test
```

Keep PRs focused on one change, and say in the description how you tested it
(and on which OS).

By contributing you agree your work is licensed under the project's
[Apache-2.0 licence](LICENSE).
