"""Build the two invisible fonts that make room for color emoji in text.

Usage: python3 tools/gen_emoji_space_fonts.py
(reads client/assets/twemoji.bin, so run tools/pack_twemoji.py first)

  emoji-space.ttf: every character an emoji uses, as an empty glyph 1.08 em
                   wide. An emoji's first character is set in it, at the emoji's
                   size, so the text leaves room for its picture.
  emoji-zero.ttf:  the same characters with no width, for the rest of an emoji's
                   characters (joiners, skin tones, U+FE0F...).

The emoji's real characters stay in the text, so copying a message still gives
the emoji. Both fonts are tiny and contain no drawings at all.
"""
import os, struct
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen

root = os.path.join(os.path.dirname(__file__), "..", "client", "assets")
data = open(os.path.join(root, "twemoji.bin"), "rb").read()
assert data[:4] == b"TWE1"
count = struct.unpack("<I", data[4:8])[0]
at, chars = 8, set()
for _ in range(count):
    n = data[at]
    name = data[at + 1:at + 1 + n].decode()
    at += 9 + n
    chars.update(int(h, 16) for h in name.split("-"))
chars.update([0xFE0F, 0x200D, 0x20E3])
chars.update(range(0xE0020, 0xE0080))  # tag characters (flags of England etc.)


def build(path, advance, family):
    fb = FontBuilder(1000, isTTF=True)
    glyphs = [".notdef", "blank"]
    fb.setupGlyphOrder(glyphs)
    fb.setupCharacterMap({c: "blank" for c in sorted(chars)})
    empty = TTGlyphPen(None).glyph()
    fb.setupGlyf({g: empty for g in glyphs})
    fb.setupHorizontalMetrics({".notdef": (advance, 0), "blank": (advance, 0)})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": family, "styleName": "Regular"})
    fb.setupOS2(sTypoAscender=800, sTypoDescender=-200, usWinAscent=800, usWinDescent=200)
    fb.setupPost()
    fb.save(path)
    print(path, os.path.getsize(path), "bytes,", len(chars), "characters")


build(os.path.join(root, "emoji-space.ttf"), 1080, "Backroom Emoji Space")
build(os.path.join(root, "emoji-zero.ttf"), 0, "Backroom Emoji Zero")
