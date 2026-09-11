#!/usr/bin/env python3
"""Regenerate every brand asset in assets/ from assets/hyprlay.svg.

That SVG is the single source for all sizes; outputs are committed, so
run this after editing it and commit the result.
"""
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(os.path.dirname(HERE), "assets")
SVG = os.path.join(ASSETS, "hyprlay.svg")

H_FILL = 'fill="#ffffff"'
H_DIMMED = 'fill="#a0a0a0"'
TRAY_PX = 48
APP_PX = (48, 64, 128, 256)
ICO_SIZES = [(px, px) for px in sorted(APP_PX + (32, 16), reverse=True)]


def raster(svg_text, px, out_path):
    with tempfile.NamedTemporaryFile("w", suffix=".svg", delete=False) as f:
        f.write(svg_text)
        src = f.name
    try:
        subprocess.run(
            ["rsvg-convert", "-w", str(px), "-h", str(px), "-o", out_path, src],
            check=True,
        )
    finally:
        os.unlink(src)


def main():
    if not shutil.which("rsvg-convert"):
        sys.exit("error: rsvg-convert not found (install librsvg)")
    try:
        from PIL import Image
    except ImportError:
        sys.exit("error: Pillow not found (needed for assets/hyprlay.ico)")

    with open(SVG) as f:
        svg = f.read()
    if svg.count(H_FILL) != 1:
        sys.exit(f"error: expected exactly one {H_FILL} (the H glyph) in {SVG}")

    raster(svg, TRAY_PX, os.path.join(ASSETS, "tray-connected.png"))
    raster(svg.replace(H_FILL, H_DIMMED, 1), TRAY_PX,
           os.path.join(ASSETS, "tray-disconnected.png"))
    for px in APP_PX:
        raster(svg, px, os.path.join(ASSETS, f"hyprlay-{px}.png"))

    master = os.path.join(ASSETS, "hyprlay-256.png")
    Image.open(master).save(
        os.path.join(ASSETS, "hyprlay.ico"), format="ICO", sizes=ICO_SIZES
    )
    print("wrote tray pair, hyprlay PNGs", APP_PX, "and hyprlay.ico from", SVG)


if __name__ == "__main__":
    main()
