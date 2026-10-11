# llama.cpp backend compatibility

MyLLM Connect can use an existing llama.cpp server without changing the MyLLM
iOS app. The phone continues to call the Ollama-shaped pairing API; the desktop
companion translates those calls to llama.cpp's OpenAI-compatible API.

## Start llama.cpp

Use a build that provides `/v1/models` and `/v1/chat/completions`. Enable Jinja
templates when the model supports tools:

```sh
llama-server -m /path/to/model.gguf --host 127.0.0.1 --port 8080 --jinja
```

The `llama serve` command name used by current Homebrew builds works as well.
Ports 8080 and 8081 are auto-detected.

For another port, launch the companion with a loopback URL:

```sh
MYLLM_LLAMA_CPP_URL=http://127.0.0.1:8090 \
  "/Applications/MyLLM Connect.app/Contents/MacOS/myllm-connect"
```

If llama.cpp was started with `--api-key`, also set
`MYLLM_LLAMA_CPP_API_KEY`. The key is used only between the companion and the
loopback server; it is not placed in the pairing QR.

## Detection order

1. `MYLLM_LLAMA_CPP_URL`, when set and reachable.
2. Ollama on `127.0.0.1:11434`.
3. llama.cpp on `127.0.0.1:8080`, then `127.0.0.1:8081`.

Configured URLs must use HTTP(S) and resolve to `localhost` or a loopback IP.
This prevents the companion from becoming an authenticated proxy to arbitrary
network services.

## API mapping

| MyLLM/Ollama contract | llama.cpp upstream | Notes |
|---|---|---|
| `GET /api/tags` | `GET /v1/models` plus `/props` | Model IDs are returned in Ollama's `models` list; `default_generation_settings.n_ctx` is exposed as `context_window` when llama.cpp reports it. |
| `POST /api/chat` | `POST /v1/chat/completions` | Both non-streaming JSON and streaming SSE→NDJSON are supported. |

The chat adapter maps text, reasoning/thinking chunks, tool definitions and
calls, base64 image inputs, JSON output requests, token limits, temperature,
sampling options, stop sequences, and usage counts. Other Ollama endpoints are
not exposed through the llama.cpp adapter.

## Validation

The adapter has unit coverage for request/response translation, fragmented SSE
streams, tool-call argument assembly, image inputs, and loopback URL validation.
The feature was also exercised end-to-end on Apple Silicon with a live
llama.cpp model: unauthenticated HTTPS returned 401, authenticated model
discovery returned 200, and the MyLLM iOS app paired and received a streamed
chat response over Tailscale.
