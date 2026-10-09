//! Message text with color emoji and clickable links.
//!
//! The text is laid out by egui as one selectable label (so copying a message
//! gives the real text, emoji included). Each emoji's characters are laid out
//! invisibly, with room left after them, and its Twemoji picture is painted in
//! that room. A message that's nothing but a few emoji shows them big.

use crate::theme::pal;
use backroom::twemoji;
use eframe::egui::{
    self, text::LayoutJob, Color32, FontId, Rect, Sense, TextFormat, TextureHandle, TextureOptions,
    Vec2,
};
use std::collections::{HashMap, VecDeque};

/// Text size in messages (egui's Body style is set to this in theme.rs).
pub const TEXT: f32 = 15.0;
/// Emoji among text.
pub const INLINE: f32 = 22.0;
/// A message of only emoji (up to `JUMBO_MAX` of them).
pub const JUMBO: f32 = 44.0;
pub const JUMBO_MAX: usize = 27;
/// Emoji pictures kept as textures (least recently used dropped).
const MAX_TEXTURES: usize = 400;

#[derive(Default)]
pub struct EmojiCache {
    textures: HashMap<&'static str, TextureHandle>,
    order: VecDeque<&'static str>,
}

impl EmojiCache {
    pub fn get(&mut self, ctx: &egui::Context, name: &'static str) -> Option<TextureHandle> {
        if let Some(t) = self.textures.get(name) {
            return Some(t.clone());
        }
        let img = twemoji::decode(name)?;
        let tex = ctx.load_texture(format!("emoji-{name}"), img, TextureOptions::LINEAR);
        self.textures.insert(name, tex.clone());
        self.order.push_back(name);
        while self.order.len() > MAX_TEXTURES {
            if let Some(old) = self.order.pop_front() {
                self.textures.remove(old);
            }
        }
        Some(tex)
    }
}

/// Where links start and end in the text (byte ranges), and their address.
fn find_links(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = [text[from..].find("https://"), text[from..].find("http://")]
        .into_iter()
        .flatten()
        .min()
    {
        let start = from + i;
        let end = text[start..]
            .find(char::is_whitespace)
            .map(|e| start + e)
            .unwrap_or(text.len());
        let trimmed = text[start..end].trim_end_matches(|c: char| ").,!?;:'\"]".contains(c));
        let end = start + trimmed.len();
        if trimmed.len() > 8 {
            out.push((start, end));
        }
        from = end.max(start + 1);
        if from >= text.len() {
            break;
        }
    }
    out
}

/// Show a message's text. Returns the link that was clicked, if any.
pub fn show(ui: &mut egui::Ui, text: &str, cache: &mut EmojiCache) -> Option<String> {
    let found = twemoji::find(text);
    let jumbo = twemoji::only_emoji(text, &found).is_some_and(|n| n <= JUMBO_MAX);
    let size = if jumbo { JUMBO } else { INLINE };
    let links = find_links(text);

    let text_format = TextFormat {
        font_id: FontId::proportional(TEXT),
        color: pal().text,
        ..Default::default()
    };
    let link_format = TextFormat {
        color: pal().accent,
        ..text_format.clone()
    };
    // The emoji's characters stay in the text (so copying works) but are set in
    // invisible fonts: the first one exactly as wide as the picture, the rest with no width.
    let emoji_first = TextFormat {
        font_id: FontId::new(
            size,
            egui::FontFamily::Name(backroom::emoji::SPACE_FONT.into()),
        ),
        color: Color32::TRANSPARENT,
        line_height: Some(size + 2.0),
        ..Default::default()
    };
    let emoji_rest = TextFormat {
        font_id: FontId::new(
            size,
            egui::FontFamily::Name(backroom::emoji::ZERO_FONT.into()),
        ),
        color: Color32::TRANSPARENT,
        line_height: Some(size + 2.0),
        ..Default::default()
    };

    let mut job = LayoutJob {
        text: text.to_string(),
        wrap: egui::text::TextWrapping {
            max_width: ui.available_width(),
            ..Default::default()
        },
        ..Default::default()
    };
    let in_link = |b: usize| links.iter().any(|(s, e)| b >= *s && b < *e);
    let push_text = |job: &mut LayoutJob, from: usize, to: usize| {
        // Split plain text at link edges.
        let mut at = from;
        while at < to {
            let link = in_link(at);
            let mut next = at;
            while next < to && in_link(next) == link {
                next += text[next..].chars().next().map_or(1, char::len_utf8);
            }
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: at..next,
                format: if link {
                    link_format.clone()
                } else {
                    text_format.clone()
                },
            });
            at = next;
        }
    };
    let mut at = 0;
    for f in &found {
        push_text(&mut job, at, f.start);
        let first_len = text[f.start..].chars().next().map_or(1, char::len_utf8);
        job.sections.push(egui::text::LayoutSection {
            leading_space: 0.0,
            byte_range: f.start..f.start + first_len,
            format: emoji_first.clone(),
        });
        if f.start + first_len < f.end {
            job.sections.push(egui::text::LayoutSection {
                leading_space: 0.0,
                byte_range: f.start + first_len..f.end,
                format: emoji_rest.clone(),
            });
        }
        at = f.end;
    }
    push_text(&mut job, at, text.len());

    let galley = ui.fonts(|f| f.layout_job(job));
    let resp = ui.add(egui::Label::new(galley.clone()).sense(Sense::click()));
    let origin = resp.rect.min;

    // Paint the pictures.
    let ctx = ui.ctx().clone();
    let painter = ui.painter_at(resp.rect.expand(size));
    for f in &found {
        let Some(tex) = cache.get(&ctx, f.name) else {
            continue;
        };
        // Its room in the text: a little wider than the picture (so neighbors
        // don't touch), at the bottom of its row like the text.
        let at = galley.pos_from_cursor(egui::text::CCursor::new(f.char_start));
        let rect = Rect::from_min_size(
            egui::pos2(
                origin.x + at.left() + size * 0.04,
                origin.y + at.bottom() - size - 1.0,
            ),
            Vec2::splat(size),
        );
        painter.image(
            tex.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }

    // Links: pointing hand over them, open on click.
    if links.is_empty() {
        return None;
    }
    let hovered_link = resp.hover_pos().and_then(|p| {
        let c = galley.cursor_from_pos(p - origin);
        let byte = text
            .char_indices()
            .nth(c.index)
            .map(|(b, _)| b)
            .unwrap_or(text.len());
        links.iter().find(|(s, e)| byte >= *s && byte < *e).copied()
    });
    if let Some((s, e)) = hovered_link {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            return Some(text[s..e].to_string());
        }
        resp.on_hover_text(&text[s..e]);
    }
    None
}

/// An emoji picture at a given spot (the picker).
pub fn paint(ui: &egui::Ui, cache: &mut EmojiCache, name: &'static str, rect: Rect) {
    if let Some(tex) = cache.get(ui.ctx(), name) {
        ui.painter().image(
            tex.id(),
            rect,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links() {
        let t = "see https://example.com/a, and http://x.io!";
        let l = find_links(t);
        assert_eq!(l.len(), 2);
        assert_eq!(&t[l[0].0..l[0].1], "https://example.com/a");
        assert_eq!(&t[l[1].0..l[1].1], "http://x.io");
        assert!(find_links("http:// alone").is_empty());
        assert!(find_links("no links").is_empty());
    }
}
