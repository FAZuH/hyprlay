#!/usr/bin/env python3
"""Generate the two tray icons (RGBA PNG, 48x48) with an "H" glyph.
Run once; the PNGs are committed under assets/. No third-party deps."""
import zlib, struct, os

W = H = 48

def draw_h(color):
    """Return a W*H*4 RGBA bytearray with an "H" glyph on a transparent
    background. `color` is (r, g, b, a)."""
    r, g, b, a = color
    bar = 8       # stroke thickness of each bar
    margin = 10   # gap from the canvas edge to the vertical bars
    # Vertical bars span the full letter height.
    v_top, v_bot = margin, H - margin
    # Horizontal center bar, vertically centered.
    c_top = H // 2 - bar // 2
    c_bot = H // 2 + bar // 2
    left = margin
    right = W - margin - bar
    buf = bytearray(W * H * 4)
    for y in range(H):
        for x in range(W):
            on = False
            if v_top <= y < v_bot:
                if left <= x < left + bar:
                    on = True
                if right <= x < right + bar:
                    on = True
            if c_top <= y < c_bot and left + bar <= x < right:
                on = True
            i = (y * W + x) * 4
            if on:
                buf[i:i+4] = bytes((r, g, b, a))
            # else: leave transparent (alpha 0)
    return buf

def write_png(path, pixels):
    raw = bytearray()
    for y in range(H):
        raw.append(0)  # filter type 0 (none)
        raw += pixels[y * W * 4:(y + 1) * W * 4]
    comp = zlib.compress(bytes(raw), 9)
    def chunk(typ, data):
        body = typ + data
        return (struct.pack(">I", len(data)) + body
                + struct.pack(">I", zlib.crc32(body) & 0xffffffff))
    sig = b'\x89PNG\r\n\x1a\n'
    ihdr = struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0)  # 8-bit RGBA
    with open(path, "wb") as f:
        f.write(sig + chunk(b'IHDR', ihdr)
                + chunk(b'IDAT', comp) + chunk(b'IEND', b''))

here = os.path.dirname(os.path.abspath(__file__))
assets = os.path.join(os.path.dirname(here), "assets")
os.makedirs(assets, exist_ok=True)
write_png(os.path.join(assets, "tray-connected.png"),
          draw_h((255, 255, 255, 255)))    # white H
write_png(os.path.join(assets, "tray-disconnected.png"),
          draw_h((160, 160, 160, 255)))    # dim grey H
print("wrote", os.path.join(assets, "tray-connected.png"),
      os.path.join(assets, "tray-disconnected.png"))
