"""Generates the Kadro source icon (1024px) and the macOS tray template icon.
Run: python3 assets/make_icons.py, then: npx tauri icon assets/icon-source.png"""
from PIL import Image, ImageDraw
import os

HERE = os.path.dirname(os.path.abspath(__file__))
S = 1024

def brackets(draw, box, w, arm, color):
    x0, y0, x1, y1 = box
    for (cx, cy, dx, dy) in [(x0, y0, 1, 1), (x1, y0, -1, 1), (x0, y1, 1, -1), (x1, y1, -1, -1)]:
        draw.rectangle([min(cx, cx + dx * arm), min(cy, cy + dy * w), max(cx, cx + dx * arm), max(cy, cy + dy * w)], fill=color)
        draw.rectangle([min(cx, cx + dx * w), min(cy, cy + dy * arm), max(cx, cx + dx * w), max(cy, cy + dy * arm)], fill=color)

# App icon: gradient rounded square with white frame corners and a dot
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
top, bottom = (99, 102, 241), (168, 85, 247)
for y in range(S):
    t = y / (S - 1)
    gd.line([(0, y), (S, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle([64, 64, S - 64, S - 64], radius=200, fill=255)
icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
icon.paste(grad, (0, 0), mask)
d = ImageDraw.Draw(icon)
brackets(d, (240, 240, S - 240, S - 240), 64, 200, (255, 255, 255, 255))
d.ellipse([S / 2 - 70, S / 2 - 70, S / 2 + 70, S / 2 + 70], fill=(255, 255, 255, 255))
icon.save(os.path.join(HERE, "icon-source.png"))

# Tray icon (template for macOS: black on transparent, drawn at 4x then downsampled)
for size, name in [(44, "tray-template.png")]:
    big = Image.new("RGBA", (size * 4, size * 4), (0, 0, 0, 0))
    bd = ImageDraw.Draw(big)
    m = size * 4
    brackets(bd, (16, 16, m - 16, m - 16), 18, 60, (0, 0, 0, 255))
    bd.ellipse([m / 2 - 22, m / 2 - 22, m / 2 + 22, m / 2 + 22], fill=(0, 0, 0, 255))
    big.resize((size, size), Image.LANCZOS).save(os.path.join(HERE, "..", "src-tauri", "icons", name))
