//! Color emoji (Twemoji, see assets/NOTICE.md): finding emoji in text and
//! getting their pictures.
//!
//! The pictures are packed into the app (`assets/twemoji.bin`, built by
//! `tools/pack_twemoji.py`) and decoded one at a time, only when shown.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

static PACK: &[u8] = include_bytes!("../assets/twemoji.bin");

struct Index {
    /// Name ("1f44d", "1f469-200d-1f4bb") -> (offset, length) in the data.
    entries: HashMap<&'static str, (u32, u32)>,
    data: &'static [u8],
    /// Characters an emoji can start with.
    starts: HashSet<char>,
    /// Longest emoji, in characters.
    longest: usize,
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let bad = || Index {
            entries: HashMap::new(),
            data: &[],
            starts: HashSet::new(),
            longest: 0,
        };
        if PACK.len() < 8 || &PACK[..4] != b"TWE1" {
            return bad();
        }
        let count = u32::from_le_bytes(PACK[4..8].try_into().unwrap()) as usize;
        let mut entries = HashMap::with_capacity(count);
        let mut starts = HashSet::new();
        let mut longest = 0;
        let mut at = 8;
        for _ in 0..count {
            let Some(&n) = PACK.get(at) else { return bad() };
            let n = n as usize;
            let Some(name) = PACK
                .get(at + 1..at + 1 + n)
                .and_then(|b| std::str::from_utf8(b).ok())
            else {
                return bad();
            };
            let nums = &PACK[at + 1 + n..at + 9 + n];
            let offset = u32::from_le_bytes(nums[..4].try_into().unwrap());
            let len = u32::from_le_bytes(nums[4..].try_into().unwrap());
            at += 9 + n;
            let chars: Vec<char> = name
                .split('-')
                .filter_map(|h| u32::from_str_radix(h, 16).ok().and_then(char::from_u32))
                .collect();
            if let Some(&c) = chars.first() {
                starts.insert(c);
            }
            longest = longest.max(chars.len());
            entries.insert(name, (offset, len));
        }
        Index {
            entries,
            data: &PACK[at..],
            starts,
            longest,
        }
    })
}

const VS16: char = '\u{FE0F}';
const ZWJ: char = '\u{200D}';

/// Twemoji's name for a run of characters: hex code points joined by "-",
/// leaving out the emoji-style selector (U+FE0F) unless it's a joined sequence.
fn name_of(chars: &[char], keep_vs16: bool) -> String {
    chars
        .iter()
        .filter(|&&c| keep_vs16 || c != VS16)
        .map(|c| format!("{:x}", *c as u32))
        .collect::<Vec<_>>()
        .join("-")
}

fn lookup(chars: &[char]) -> Option<&'static str> {
    let idx = index();
    let joined = chars.contains(&ZWJ);
    let try_name = |keep: bool| {
        let n = name_of(chars, keep);
        idx.entries.get_key_value(n.as_str()).map(|(k, _)| *k)
    };
    if joined {
        try_name(true).or_else(|| try_name(false))
    } else {
        try_name(false)
    }
}

/// An emoji found in some text.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// Byte range in the text.
    pub start: usize,
    pub end: usize,
    /// Index of its first character, and how many characters it spans.
    pub char_start: usize,
    pub chars: usize,
    /// Its picture's name.
    pub name: &'static str,
}

/// Every emoji in `text` that has a picture.
pub fn find(text: &str) -> Vec<Found> {
    let idx = index();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i].1;
        if !idx.starts.contains(&c) {
            i += 1;
            continue;
        }
        // Longest match first; the text may carry extra U+FE0F the names leave out.
        let max = (idx.longest * 2).min(chars.len() - i);
        let run: Vec<char> = chars[i..i + max].iter().map(|x| x.1).collect();
        let mut hit = None;
        for len in (1..=max).rev() {
            let cand = &run[..len];
            // Plain symbols like © ™ # or digits only count as emoji when asked
            // for (followed by U+FE0F or a keycap).
            if len == 1 && (c as u32) < 0x2190 {
                continue;
            }
            if cand.last() == Some(&ZWJ) {
                continue;
            }
            if let Some(name) = lookup(cand) {
                hit = Some((len, name));
                break;
            }
        }
        match hit {
            Some((mut len, name)) => {
                // Swallow a trailing U+FE0F that the name didn't need.
                if chars.get(i + len).is_some_and(|x| x.1 == VS16) {
                    len += 1;
                }
                let end = chars.get(i + len).map(|x| x.0).unwrap_or(text.len());
                out.push(Found {
                    start: chars[i].0,
                    end,
                    char_start: i,
                    chars: len,
                    name,
                });
                i += len;
            }
            None => i += 1,
        }
    }
    out
}

/// If the text is nothing but emoji (and spaces), how many — for showing them big.
pub fn only_emoji(text: &str, found: &[Found]) -> Option<usize> {
    if found.is_empty() {
        return None;
    }
    let mut last = 0;
    for f in found {
        if !text[last..f.start].trim().is_empty() {
            return None;
        }
        last = f.end;
    }
    text[last..].trim().is_empty().then_some(found.len())
}

/// The packed picture (WebP) for an emoji name.
pub fn picture(name: &str) -> Option<&'static [u8]> {
    let idx = index();
    let &(offset, len) = idx.entries.get(name)?;
    idx.data.get(offset as usize..(offset + len) as usize)
}

/// Decode an emoji's picture (72x72).
pub fn decode(name: &str) -> Option<eframe::egui::ColorImage> {
    let bytes = picture(name)?;
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::WebP)
        .ok()?
        .to_rgba8();
    Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
        [img.width() as usize, img.height() as usize],
        img.as_raw(),
    ))
}

/// Picture name for a single emoji string (e.g. from the picker).
pub fn name_for(emoji: &str) -> Option<&'static str> {
    find(emoji).first().map(|f| f.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(t: &str) -> Vec<&'static str> {
        find(t).into_iter().map(|f| f.name).collect()
    }

    #[test]
    fn pack_loads() {
        assert!(index().entries.len() > 3500, "{}", index().entries.len());
        let img = decode("1f44d").unwrap();
        assert_eq!(img.size, [72, 72]);
    }

    #[test]
    fn finds_emoji() {
        assert_eq!(names("nice 👍 job"), ["1f44d"]);
        // Skin tones, joined sequences, flags, keycaps, hearts with or without U+FE0F.
        assert_eq!(names("👍🏽"), ["1f44d-1f3fd"]);
        assert_eq!(names("👩‍💻"), ["1f469-200d-1f4bb"]);
        assert_eq!(names("🇺🇸🇬🇧"), ["1f1fa-1f1f8", "1f1ec-1f1e7"]);
        assert_eq!(names("1️⃣"), ["31-20e3"]);
        assert_eq!(names("❤"), ["2764"]);
        assert_eq!(names("❤️"), ["2764"]);
        assert_eq!(names("❤️‍🔥"), ["2764-fe0f-200d-1f525"]);
        // Plain text symbols stay text.
        assert!(names("© 2026 ™ #1 50%").is_empty());
        assert_eq!(names("©️"), ["a9"]);
        // Byte and char positions.
        let f = &find("hé 👍🏽!")[0];
        assert_eq!(
            (f.char_start, f.chars, &"hé 👍🏽!"[f.start..f.end]),
            (3, 2, "👍🏽")
        );
        let f = &find("❤️ok")[0];
        assert_eq!(f.chars, 2);
    }

    #[test]
    fn emoji_only() {
        let t = "🔥 🔥  💯";
        assert_eq!(only_emoji(t, &find(t)), Some(3));
        let t = "fire 🔥";
        assert_eq!(only_emoji(t, &find(t)), None);
        assert_eq!(only_emoji("hi", &find("hi")), None);
    }

    #[test]
    fn every_picker_emoji_has_a_picture() {
        let missing: Vec<&str> = crate::emoji::SHORTCODES
            .iter()
            .map(|(_, e)| *e)
            .chain(crate::emoji::emoticon_emoji())
            .filter(|e| name_for(e).is_none())
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }
}
