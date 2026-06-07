"""Hand-run bearer auth proxy for the MyLLM Connect pairing proof (issue #3, by hand).

Sits in front of Ollama (127.0.0.1:11434) and enforces
`Authorization: Bearer <token>` -- 401 otherwise. Streaming pass-through.

Binds to 127.0.0.1 only: `tailscale serve` is the sole way in from outside,
so the endpoint is unreachable on the LAN without the mesh + the token
(same effect the protocol asks for with "bind to the Tailscale interface").

Usage:
    python auth_proxy.py            # mints+persists token on first run
    python auth_proxy.py --rotate   # mint a new token (invalidates old)
"""

import http.client
import http.server
import json
import secrets
import socketserver
import sys
from pathlib import Path

LISTEN_HOST = "127.0.0.1"
LISTEN_PORT = 11435
UPSTREAM_HOST = "127.0.0.1"
UPSTREAM_PORT = 11434
TOKEN_FILE = Path(__file__).with_name("token.txt")

# Hop-by-hop headers must not be forwarded either direction.
HOP_BY_HOP = {
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailers", "transfer-encoding", "upgrade", "host",
}


def load_token(rotate: bool = False) -> str:
    if rotate or not TOKEN_FILE.exists():
        token = secrets.token_urlsafe(32)  # 32 random bytes, base64url
        TOKEN_FILE.write_text(token, encoding="ascii")
        print(f"{'Rotated' if rotate else 'Minted'} token -> {TOKEN_FILE}")
    return TOKEN_FILE.read_text(encoding="ascii").strip()


TOKEN = load_token(rotate="--rotate" in sys.argv)


class AuthProxyHandler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def _authorized(self) -> bool:
        auth = self.headers.get("Authorization", "")
        return auth.startswith("Bearer ") and secrets.compare_digest(
            auth.removeprefix("Bearer "), TOKEN
        )

    def _reject(self) -> None:
        body = json.dumps({"error": "unauthorized"}).encode()
        self.send_response(401)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _forward(self) -> None:
        if not self._authorized():
            self._reject()
            return

        length = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(length) if length else None

        upstream = http.client.HTTPConnection(UPSTREAM_HOST, UPSTREAM_PORT)
        headers = {
            k: v for k, v in self.headers.items()
            if k.lower() not in HOP_BY_HOP and k.lower() != "authorization"
        }
        upstream.request(self.command, self.path, body=body, headers=headers)
        resp = upstream.getresponse()

        self.send_response(resp.status)
        for k, v in resp.getheaders():
            if k.lower() not in HOP_BY_HOP:
                self.send_header(k, v)
        chunked = resp.getheader("Transfer-Encoding", "").lower() == "chunked"
        if chunked:
            self.send_header("Transfer-Encoding", "chunked")
        self.end_headers()

        # Stream the body through unbuffered (NDJSON chat streaming relies on this).
        while True:
            chunk = resp.read(8192)
            if not chunk:
                break
            if chunked:
                self.wfile.write(f"{len(chunk):X}\r\n".encode())
                self.wfile.write(chunk)
                self.wfile.write(b"\r\n")
            else:
                self.wfile.write(chunk)
            self.wfile.flush()
        if chunked:
            self.wfile.write(b"0\r\n\r\n")
        upstream.close()

    do_GET = do_POST = do_DELETE = do_HEAD = _forward

    def log_message(self, fmt, *args):  # quieter: one line per request
        status = args[1] if len(args) > 1 else "?"
        print(f"{self.command} {self.path} -> {status}")


class ThreadingHTTPServer(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True


if __name__ == "__main__":
    print(f"Bearer auth proxy: {LISTEN_HOST}:{LISTEN_PORT} -> {UPSTREAM_HOST}:{UPSTREAM_PORT}")
    ThreadingHTTPServer((LISTEN_HOST, LISTEN_PORT), AuthProxyHandler).serve_forever()
