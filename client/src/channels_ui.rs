//! Adding channels (admins, servers that allow it): the "+" next to each
//! channel list heading, the name box under it, and the updated lists.

use crate::theme::pal;
use crate::App;
use eframe::egui::{self, Align, Key, Layout, RichText, Sense, Stroke};

/// The name box for a new channel, if open.
#[derive(Default)]
pub(crate) struct NewChannel {
    /// Open under the voice list (`Some(true)`) or the text list.
    open: Option<bool>,
    name: String,
    focus: bool,
    /// Asked for a text channel: open it when it arrives.
    pub(crate) awaiting_text: bool,
}

/// A channel list heading, with a "+" for admins. Returns a channel to create
/// (name, voice) once its name is entered.
pub(crate) fn header(
    ui: &mut egui::Ui,
    voice: bool,
    can_add: bool,
    nc: &mut NewChannel,
) -> Option<(String, bool)> {
    let (title, tip) = if voice {
        ("Voice channels", "Add a voice channel")
    } else {
        ("Text channels", "Add a text channel")
    };
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(title).size(13.0).color(pal().faint).strong());
        if !can_add {
            return;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(6.0);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
            let color = if resp.hovered() || nc.open == Some(voice) {
                pal().text
            } else {
                pal().faint
            };
            let (c, d) = (rect.center(), 5.5);
            let stroke = Stroke::new(1.8_f32, color);
            let p = ui.painter();
            p.line_segment([c - egui::vec2(d, 0.0), c + egui::vec2(d, 0.0)], stroke);
            p.line_segment([c - egui::vec2(0.0, d), c + egui::vec2(0.0, d)], stroke);
            if resp.on_hover_text(tip).clicked() {
                if nc.open == Some(voice) {
                    nc.open = None;
                } else {
                    nc.open = Some(voice);
                    nc.name.clear();
                    nc.focus = true;
                }
            }
        });
    });
    ui.add_space(2.0);
    if nc.open != Some(voice) {
        return None;
    }

    let mut create = None;
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut nc.name)
                .hint_text(if voice { "Channel name" } else { "new-channel" })
                .char_limit(40)
                .desired_width(ui.available_width() - 8.0),
        );
        if nc.focus {
            r.request_focus();
            nc.focus = false;
        }
        if r.lost_focus() {
            let name = nc.name.trim();
            if ui.input(|i| i.key_pressed(Key::Enter)) && !name.is_empty() {
                create = Some((name.to_string(), voice));
                nc.awaiting_text = !voice;
            }
            nc.open = None;
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        ui.label(
            RichText::new("Enter to add, Esc to cancel")
                .size(12.0)
                .color(pal().faint),
        );
    });
    ui.add_space(4.0);
    create
}

impl App {
    /// An admin added a channel: new lists for everyone.
    pub(crate) fn on_channels(&mut self, text: Vec<String>, voice: Vec<String>) {
        let Some(sess) = self.session.as_mut() else {
            return;
        };
        let added = text
            .iter()
            .find(|c| !sess.text_channels.contains(c))
            .cloned();
        for c in &text {
            sess.history.entry(c.clone()).or_default();
        }
        sess.text_channels = text;
        sess.voice_channels = voice;
        if let Some(ch) = added.filter(|_| self.new_channel.awaiting_text) {
            self.new_channel.awaiting_text = false;
            self.select_text_channel(ch);
        }
    }
}
