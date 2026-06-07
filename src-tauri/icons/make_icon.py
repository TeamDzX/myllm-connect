"""Placeholder app icon: blue rounded square, white 'M', green status dot.
Regenerate with: python make_icon.py && npx tauri icon src-tauri/icons/icon-source.png
Replace with real brand art before public release.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

S = 1024
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# iOS-ish rounded square, MyLLM blue
d.rounded_rectangle([64, 64, S - 64, S - 64], radius=200, fill=(10, 132, 255, 255))

# White "M"
font = None
for name in ("segoeuib.ttf", "arialbd.ttf"):
    try:
        font = ImageFont.truetype(name, 560)
        break
    except OSError:
        continue
d.text((S / 2, S / 2 - 30), "M", font=font, fill="white", anchor="mm")

# Green status dot, bottom-right
d.ellipse([660, 660, 880, 880], fill=(52, 199, 89, 255), outline="white", width=24)

out = Path(__file__).with_name("icon-source.png")
img.save(out)
print(f"wrote {out}")
