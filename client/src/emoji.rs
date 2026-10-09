//! Turns text emoticons and :shortcodes: into emoji when a message is sent.
//!
//! `:)` -> 🙂, `<3` -> ❤, `:fire:` -> 🔥 and so on. Emoticons only convert when
//! they stand on their own (surrounded by spaces), so links and times like
//! `https://` or `10:30` are left alone.

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

/// How many of the shortcodes the picker shows.
pub const PICKER_COUNT: usize = 56;

pub fn picker() -> &'static [(&'static str, &'static str)] {
    &SHORTCODES[..PICKER_COUNT]
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
                if let Some((_, e)) = SHORTCODES
                    .iter()
                    .find(|(k, _)| valid && k.eq_ignore_ascii_case(name))
                {
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
    fn leaves_links_and_times_alone() {
        assert_eq!(convert("https://example.com/:)"), "https://example.com/:)");
        assert_eq!(convert("meet at 10:30:00"), "meet at 10:30:00");
        assert_eq!(convert("C:/Users"), "C:/Users");
        assert_eq!(convert("ratio 3:2"), "ratio 3:2");
        assert_eq!(convert("(:"), "🙂");
    }
}
