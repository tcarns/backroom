"""Build client/assets/emoji-list.tsv (the emoji picker's list) from emojibase.

Usage: python3 tools/gen_emoji_list.py <emojibase-data package folder>
Get it with: npm pack emojibase-data && tar xzf emojibase-data-*.tgz  (MIT license)

One line per emoji, in Unicode's order, skipping skin-tone pieces:
  group <TAB> emoji <TAB> name <TAB> shortcodes (space separated) <TAB> search words
"""
import json, os, sys

src = os.path.join(sys.argv[1], "en")
compact = json.load(open(os.path.join(src, "compact.json")))
github = json.load(open(os.path.join(src, "shortcodes", "github.json")))
out = os.path.join(os.path.dirname(__file__), "..", "client", "assets", "emoji-list.tsv")
rows = []
for e in compact:
    group = e.get("group")
    if group is None or group == 2:  # 2 = components (skin tones, hair)
        continue
    codes = github.get(e["hexcode"], [])
    if isinstance(codes, str):
        codes = [codes]
    words = " ".join(t for t in e.get("tags", []) if " " not in t)
    rows.append((e.get("order", 0), group, e["unicode"], e["label"], " ".join(codes), words))
rows.sort()
with open(out, "w", encoding="utf-8") as f:
    for _, group, uni, label, codes, words in rows:
        f.write(f"{group}\t{uni}\t{label}\t{codes}\t{words}\n")
print(len(rows), "emoji,", os.path.getsize(out) // 1024, "KB ->", out)
