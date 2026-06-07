"""Build the myllm://pair payload per PAIRING_PROTOCOL.md and render it as a QR."""

from pathlib import Path
from urllib.parse import quote, urlencode

import qrcode

HERE = Path(__file__).parent
token = (HERE / "token.txt").read_text(encoding="ascii").strip()

params = {
    "v": "1",
    "name": "Jarvis (desktop-corsair)",
    "url": "https://desktop-corsair.tailda54dd.ts.net",
    "token": token,
    "models": "jarvis:14b",
}
payload = "myllm://pair?" + urlencode(params, quote_via=quote, safe="")
print(payload.replace(token, token[:6] + "…"))  # never print the full token

img = qrcode.make(payload)
out = HERE / "pair_qr.png"
img.save(out)
print(f"QR written to {out}")
