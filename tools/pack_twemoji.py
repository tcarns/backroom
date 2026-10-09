"""Pack Twemoji's 72x72 PNGs into client/assets/twemoji.bin for the app.

Usage: python3 tools/pack_twemoji.py <path to twemoji/assets/72x72>

Get the images from https://github.com/jdecked/twemoji (graphics licensed
CC-BY 4.0; see client/assets/NOTICE.md). Each image is re-encoded as lossless
WebP (about 18% smaller than PNG) and stored with its name, which is the emoji's
code points in hex joined by "-" (e.g. "1f44d", "1f469-200d-1f4bb").

File layout (all numbers little-endian):
  b"TWE1", count: u32,
  count x [name_len: u8, name: ascii, offset: u32, len: u32]   (sorted by name)
  data (WebP images, offsets relative to the start of the data)
"""
import io, os, struct, sys
from PIL import Image

src = sys.argv[1]
out = os.path.join(os.path.dirname(__file__), "..", "client", "assets", "twemoji.bin")
names = sorted(f[:-4] for f in os.listdir(src) if f.endswith(".png"))
blobs = []
for n in names:
    im = Image.open(os.path.join(src, n + ".png")).convert("RGBA")
    b = io.BytesIO()
    im.save(b, "WEBP", lossless=True, quality=100, method=6)
    blobs.append(b.getvalue())
index = io.BytesIO()
offset = 0
for n, blob in zip(names, blobs):
    nb = n.encode("ascii")
    index.write(struct.pack("<B", len(nb)) + nb + struct.pack("<II", offset, len(blob)))
    offset += len(blob)
with open(out, "wb") as f:
    f.write(b"TWE1" + struct.pack("<I", len(names)))
    f.write(index.getvalue())
    for blob in blobs:
        f.write(blob)
print(f"{len(names)} emoji, {os.path.getsize(out) / 1048576:.2f} MB -> {out}")
