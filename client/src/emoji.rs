//! Turns text emoticons and :shortcodes: into emoji when a message is sent.
//!
//! `:)` -> 🙂, `<3` -> ❤, `:fire:` -> 🔥 and so on. Emoticons only convert when
//! they stand on their own (surrounded by spaces), so links and times like
//! `https://` or `10:30` are left alone.

/// Font family whose characters are invisible and one em wide (an emoji's first character).
pub const SPACE_FONT: &str = "emoji-space";
/// Font family whose characters are invisible and take no room (the rest of an emoji).
pub const ZERO_FONT: &str = "emoji-zero";

/// Adds a fallback emoji font (a trimmed copy of Symbola, ~0.9 MB) after egui's
/// built-in fonts, so newer emoji like 🤔 or 🙄 draw instead of showing a box.
pub fn install_font(ctx: &eframe::egui::Context) {
    use eframe::egui::{FontData, FontDefinitions, FontFamily};
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "emoji-symbola".into(),
        std::sync::Arc::new(FontData::from_static(include_bytes!(
            "../assets/emoji-symbola.ttf"
        ))),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        if let Some(list) = fonts.families.get_mut(&family) {
            list.push("emoji-symbola".into());
        }
    }
    // Invisible fonts that leave room for color emoji in messages (see
    // tools/gen_emoji_space_fonts.py and chat_text.rs).
    for (name, bytes) in [
        (
            SPACE_FONT,
            include_bytes!("../assets/emoji-space.ttf").as_slice(),
        ),
        (
            ZERO_FONT,
            include_bytes!("../assets/emoji-zero.ttf").as_slice(),
        ),
    ] {
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(FontData::from_static(bytes)),
        );
        fonts
            .families
            .insert(FontFamily::Name(name.into()), vec![name.into()]);
    }
    ctx.set_fonts(fonts);
}

/// Emoticons, matched as whole words.
const EMOTICONS: &[(&str, &str)] = &[
    (":)", "🙂"),
    (":-)", "🙂"),
    ("(:", "🙂"),
    (":]", "🙂"),
    (":D", "😄"),
    (":-D", "😄"),
    ("xD", "😆"),
    ("XD", "😆"),
    (":(", "🙁"),
    (":-(", "🙁"),
    ("):", "🙁"),
    (";)", "😉"),
    (";-)", "😉"),
    (":P", "😛"),
    (":p", "😛"),
    (":-P", "😛"),
    (":-p", "😛"),
    (":O", "😮"),
    (":o", "😮"),
    (":-O", "😮"),
    (":'(", "😢"),
    (":|", "😐"),
    (":-|", "😐"),
    (":/", "😕"),
    (":-/", "😕"),
    (":\\", "😕"),
    ("<3", "❤"),
    ("</3", "💔"),
    (":*", "😘"),
    (":-*", "😘"),
    ("B)", "😎"),
    ("8)", "😎"),
    ("^^", "😊"),
    ("^_^", "😊"),
    (">:(", "😠"),
    ("D:", "😧"),
    ("o/", "👋"),
];

/// `:name:` codes. The first ones also fill the picker.
pub const SHORTCODES: &[(&str, &str)] = &[
    ("joy", "😂"),
    ("rofl", "🤣"),
    ("smile", "😄"),
    ("slight_smile", "🙂"),
    ("grin", "😁"),
    ("sweat_smile", "😅"),
    ("wink", "😉"),
    ("blush", "😊"),
    ("heart_eyes", "😍"),
    ("kiss", "😘"),
    ("thinking", "🤔"),
    ("smirk", "😏"),
    ("neutral_face", "😐"),
    ("unamused", "😒"),
    ("roll_eyes", "🙄"),
    ("flushed", "😳"),
    ("sob", "😭"),
    ("cry", "😢"),
    ("angry", "😠"),
    ("rage", "😡"),
    ("scream", "😱"),
    ("sleeping", "😴"),
    ("sunglasses", "😎"),
    ("nerd", "🤓"),
    ("upside_down", "🙃"),
    ("skull", "💀"),
    ("clown", "🤡"),
    ("ghost", "👻"),
    ("thumbsup", "👍"),
    ("thumbsdown", "👎"),
    ("clap", "👏"),
    ("wave", "👋"),
    ("pray", "🙏"),
    ("ok_hand", "👌"),
    ("muscle", "💪"),
    ("eyes", "👀"),
    ("fire", "🔥"),
    ("100", "💯"),
    ("heart", "❤"),
    ("broken_heart", "💔"),
    ("sparkles", "✨"),
    ("star", "⭐"),
    ("tada", "🎉"),
    ("trophy", "🏆"),
    ("video_game", "🎮"),
    ("headphones", "🎧"),
    ("pizza", "🍕"),
    ("beer", "🍺"),
    ("coffee", "☕"),
    ("check", "✅"),
    ("x", "❌"),
    ("warning", "⚠"),
    ("question", "❓"),
    ("zzz", "💤"),
    ("poop", "💩"),
    ("rocket", "🚀"),
    // Aliases (not shown in the picker).
    ("+1", "👍"),
    ("-1", "👎"),
    ("thumbs_up", "👍"),
    ("thumbs_down", "👎"),
    ("laughing", "😆"),
    ("smiley", "😃"),
    ("frown", "🙁"),
    ("sad", "🙁"),
    ("hearts", "❤"),
    ("party", "🎉"),
    ("gg", "🏆"),
];

/// The emoji emoticons turn into.
pub fn emoticon_emoji() -> impl Iterator<Item = &'static str> {
    EMOTICONS.iter().map(|(_, e)| *e)
}

/// How many of the shortcodes the picker shows.
pub const PICKER_COUNT: usize = 56;

pub fn picker() -> &'static [(&'static str, &'static str)] {
    &SHORTCODES[..PICKER_COUNT]
}

// ---------------------------------------------------------------- every emoji

/// One emoji in the picker (from emojibase, see tools/gen_emoji_list.py).
pub struct Entry {
    pub group: u8,
    pub emoji: &'static str,
    pub name: &'static str,
    /// Shortcodes, like "thumbsup" (GitHub/Slack style).
    pub codes: Vec<&'static str>,
    /// Extra words to search by.
    pub words: &'static str,
}

/// The picker's groups, in order, with their tab names.
pub const GROUPS: [(u8, &str); 9] = [
    (0, "Smileys & emotion"),
    (1, "People & body"),
    (3, "Animals & nature"),
    (4, "Food & drink"),
    (5, "Travel & places"),
    (6, "Activities"),
    (7, "Objects"),
    (8, "Symbols"),
    (9, "Flags"),
];

static LIST: &str = include_str!("../assets/emoji-list.tsv");

/// Every emoji, in Unicode's order.
pub fn all() -> &'static [Entry] {
    static ALL: std::sync::OnceLock<Vec<Entry>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        LIST.lines()
            .filter_map(|line| {
                let mut f = line.split('\t');
                Some(Entry {
                    group: f.next()?.parse().ok()?,
                    emoji: f.next()?,
                    name: f.next()?,
                    codes: f.next().unwrap_or("").split_whitespace().collect(),
                    words: f.next().unwrap_or(""),
                })
            })
            .collect()
    })
}

/// Emoji whose name, shortcode or tags match what was typed (best matches first).
pub fn search(query: &str) -> Vec<&'static Entry> {
    let q = query.trim().trim_matches(':').to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(u8, usize, &Entry)> = all()
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let rank = if e.codes.iter().any(|c| *c == q) || e.name == q {
                0
            } else if e.codes.iter().any(|c| c.starts_with(&q)) || e.name.starts_with(&q) {
                1
            } else if e.name.split(' ').any(|w| w.starts_with(&q)) {
                2
            } else if e.words.split(' ').any(|w| w.starts_with(&q)) {
                3
            } else if e.name.contains(&q) {
                4
            } else {
                return None;
            };
            Some((rank, i, e))
        })
        .collect();
    hits.sort_by_key(|h| (h.0, h.1));
    hits.into_iter().map(|h| h.2).collect()
}

/// The emoji for a shortcode like "octopus" (any emoji, not just the picker's).
pub fn by_code(code: &str) -> Option<&'static str> {
    let code = code.to_ascii_lowercase();
    all()
        .iter()
        .find(|e| e.codes.iter().any(|c| *c == code))
        .map(|e| e.emoji)
}

/// The main shortcode for an emoji, for hints (":thumbsup:").
pub fn code_for(emoji: &str) -> Option<&'static str> {
    let bare = emoji.trim_end_matches('\u{FE0F}');
    all()
        .iter()
        .find(|e| e.emoji.trim_end_matches('\u{FE0F}') == bare)
        .and_then(|e| e.codes.first().copied())
}

/// Convert emoticons and shortcodes in a message.
pub fn convert(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    // Walk word by word, keeping the original whitespace.
    let mut word_start: Option<usize> = None;
    for (i, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(s) = word_start.take() {
                out.push_str(&convert_word(&text[s..i]));
            }
            out.push(ch);
        } else if word_start.is_none() {
            word_start = Some(i);
        }
    }
    if let Some(s) = word_start {
        out.push_str(&convert_word(&text[s..]));
    }
    out
}

fn convert_word(word: &str) -> String {
    if word.contains("://") || word.starts_with("www.") {
        return word.to_string();
    }
    if let Some((_, e)) = EMOTICONS.iter().find(|(k, _)| *k == word) {
        return e.to_string();
    }
    // Emoticon followed by trailing punctuation: "nice :)!" stays readable.
    for (k, e) in EMOTICONS {
        if let Some(rest) = word.strip_prefix(k) {
            if !rest.is_empty() && rest.chars().all(|c| matches!(c, '!' | '.' | ',' | '?')) {
                return format!("{e}{rest}");
            }
        }
    }
    replace_shortcodes(word)
}

fn replace_shortcodes(word: &str) -> String {
    if !word.contains(':') {
        return word.to_string();
    }
    let mut out = String::new();
    let mut rest = word;
    while let Some(start) = rest.find(':') {
        let after = &rest[start + 1..];
        match after.find(':') {
            Some(end) => {
                let name = &after[..end];
                let valid = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '+' || c == '-');
                let found = SHORTCODES
                    .iter()
                    .find(|(k, _)| valid && k.eq_ignore_ascii_case(name))
                    .map(|(_, e)| *e)
                    .or_else(|| if valid { by_code(name) } else { None });
                if let Some(e) = found {
                    out.push_str(&rest[..start]);
                    out.push_str(e);
                    rest = &after[end + 1..];
                } else {
                    // Not a known code: keep the first colon and look again from the next one.
                    out.push_str(&rest[..start + 1]);
                    rest = after;
                }
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoticons() {
        assert_eq!(convert("hey :)"), "hey 🙂");
        assert_eq!(convert(":D see you"), "😄 see you");
        assert_eq!(convert("i <3 this"), "i ❤ this");
        assert_eq!(convert("lol XD"), "lol 😆");
        assert_eq!(convert("nice :)!"), "nice 🙂!");
        assert_eq!(
            convert("line one :(\nline two ;)"),
            "line one 🙁\nline two 😉"
        );
    }

    #[test]
    fn shortcodes() {
        assert_eq!(convert("this is :fire: :100:"), "this is 🔥 💯");
        assert_eq!(convert(":thumbsup::thumbsup:"), "👍👍");
        assert_eq!(convert("gg:tada:"), "gg🎉");
        assert_eq!(convert(":notacode: stays"), ":notacode: stays");
        assert_eq!(convert("a:fire:b"), "a🔥b");
    }

    #[test]
    fn every_emoji_has_a_glyph() {
        use eframe::egui;
        let ctx = egui::Context::default();
        install_font(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let font = egui::FontId::proportional(15.0);
        let missing: Vec<String> = EMOTICONS
            .iter()
            .chain(SHORTCODES)
            .filter(|(_, e)| !ctx.fonts(|f| f.has_glyphs(&font, e)))
            .map(|(k, e)| format!("{k} {e}"))
            .collect();
        assert!(
            missing.is_empty(),
            "the built-in font has no glyph for: {missing:?}"
        );
    }

    #[test]
    fn every_emoji() {
        assert!(all().len() > 1800);
        assert_eq!(convert(":octopus: :taco:"), "🐙 🌮");
        assert_eq!(convert(":Thumbsup:"), "👍");
        assert_eq!(search("thumbs")[0].emoji.trim_end_matches('\u{FE0F}'), "👍");
        assert_eq!(search(":joy:")[0].emoji, "😂");
        assert!(search("pizza").iter().any(|e| e.emoji == "🍕"));
        assert!(search("zzzzzz").is_empty());
        assert_eq!(code_for("🔥"), Some("fire"));
        // Every group has emoji, and every emoji has a color picture.
        for (g, _) in GROUPS {
            assert!(all().iter().any(|e| e.group == g), "group {g}");
        }
        let missing: Vec<&str> = all()
            .iter()
            .filter(|e| crate::twemoji::name_for(e.emoji).is_none())
            .map(|e| e.emoji)
            .collect();
        assert!(
            missing.len() < 5,
            "{} without pictures: {missing:?}",
            missing.len()
        );
    }

    #[test]
    fn leaves_links_and_times_alone() {
        assert_eq!(convert("https://example.com/:)"), "https://example.com/:)");
        assert_eq!(convert("meet at 10:30:00"), "meet at 10:30:00");
        assert_eq!(convert("C:/Users"), "C:/Users");
        assert_eq!(convert("ratio 3:2"), "ratio 3:2");
        assert_eq!(convert("(:"), "🙂");
    }
}
