"""Renders Tachyon's bolt (assets/icons/bolt.svg) as the app icon: tachyon.ico and tray.png.

    python -I make_icon.py OUT_DIR
"""
import math
import os
import sys

from PIL import Image, ImageDraw

BLUE = (0, 103, 192, 255)  # Windows' default accent, as next to "Tachyon" in the toolbar
# bolt.svg: M13 2 L4 14 h7 l-1 8 9-12 h-7 z, in a 24x24 box, stroke 1.8 round.
POINTS = [(13, 2), (4, 14), (11, 14), (10, 22), (19, 10), (12, 10)]
STROKE = 1.8


def render(size, margin=1.0):
    scale = 8
    big = size * scale
    # Fit the 24-unit artwork with a little margin.
    unit = big / (24 - 2 * margin + 2 * margin)
    img = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    pts = [(x * unit, y * unit) for x, y in POINTS]
    draw.polygon(pts, fill=BLUE)
    width = STROKE * unit
    for a, b in zip(pts, pts[1:] + pts[:1]):
        draw.line([a, b], fill=BLUE, width=math.ceil(width))
    r = width / 2
    for x, y in pts:  # round joins
        draw.ellipse([x - r, y - r, x + r, y + r], fill=BLUE)
    return img.resize((size, size), Image.LANCZOS)


def main(out):
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    images = [render(s) for s in sizes]
    images[-1].save(os.path.join(out, "tachyon.ico"), sizes=[(s, s) for s in sizes], append_images=images[:-1])
    render(32).save(os.path.join(out, "tray.png"))
    render(256).save(os.path.join(out, "preview.png"))


if __name__ == "__main__":
    main(sys.argv[1])
