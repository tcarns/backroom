//! Channel admin in the sidebar (servers that allow it): the "+" next to each
//! list heading and its name box, the right-click menu (rename, delete), the
//! delete confirmation, dragging channels into a new order, and the updated lists.

use crate::theme::pal;
use crate::App;
use eframe::egui::{
    self, Align, Color32, CornerRadius, CursorIcon, Frame, Key, Layout, Margin, RichText, Sense,
    Stroke,
};
use proto::{ChannelRenamed, ClientMsg};

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

/// Renaming, deleting and dragging channels (admins).
#[derive(Default)]
pub(crate) struct ChannelEdit {
    /// The rename box is open on this channel: (voice, name).
    renaming: Option<(bool, String)>,
    rename_to: String,
    rename_focus: bool,
    /// "Delete channel?" is showing for this channel: (voice, name).
    confirm_delete: Option<(bool, String)>,
    drag: Option<Drag>,
}

/// A channel being dragged to a new place in its list.
struct Drag {
    voice: bool,
    name: String,
    from: usize,
    /// The mouse button was let go this frame.
    dropped: bool,
}

/// What the channel lists asked for this frame.
#[derive(Default)]
pub(crate) struct ListActions {
    pub(crate) select_text: Option<String>,
    pub(crate) create: Option<(String, bool)>,
    pub(crate) join: Option<String>,
    pub(crate) open_pop: Option<crate::VolumePop>,
    /// (voice, old name, new name)
    pub(crate) rename: Option<(bool, String, String)>,
    /// (voice, name, new index)
    pub(crate) move_to: Option<(bool, String, usize)>,
}

/// The rename box in place of a channel row, when it's open on `ch`.
/// Returns the space it took, or `None` when the row should be drawn as usual.
pub(crate) fn rename_box(
    ui: &mut egui::Ui,
    ed: &mut ChannelEdit,
    voice: bool,
    ch: &str,
    act: &mut ListActions,
) -> Option<egui::Rect> {
    if ed.renaming.as_ref() != Some(&(voice, ch.to_string())) {
        return None;
    }
    let top = ui.cursor().top();
    ui.horizontal(|ui| {
        ui.add_space(4.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut ed.rename_to)
                .char_limit(40)
                .desired_width(ui.available_width() - 4.0),
        );
        if ed.rename_focus {
            r.request_focus();
            ed.rename_focus = false;
        }
        if r.lost_focus() {
            let to = ed.rename_to.trim();
            if ui.input(|i| i.key_pressed(Key::Enter)) && !to.is_empty() && to != ch {
                act.rename = Some((voice, ch.to_string(), to.to_string()));
            }
            ed.renaming = None;
        }
    });
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(
            RichText::new("Enter to rename, Esc to cancel")
                .size(12.0)
                .color(pal().faint),
        );
    });
    let x = ui.max_rect().x_range();
    Some(egui::Rect::from_x_y_ranges(x, top..=ui.cursor().top()))
}

/// Right-click menu and dragging for a channel row (admins on servers that allow it).
pub(crate) fn row_admin(
    resp: &egui::Response,
    ed: &mut ChannelEdit,
    voice: bool,
    ch: &str,
    index: usize,
) {
    resp.context_menu(|ui| {
        ui.set_min_width(150.0);
        if ui.button("Rename channel").clicked() {
            ed.renaming = Some((voice, ch.to_string()));
            ed.rename_to = ch.to_string();
            ed.rename_focus = true;
            ui.close();
        }
        if ui
            .button(RichText::new("Delete channel").color(pal().red))
            .clicked()
        {
            ed.confirm_delete = Some((voice, ch.to_string()));
            ui.close();
        }
    });
    if resp.drag_started() {
        ed.drag = Some(Drag {
            voice,
            name: ch.to_string(),
            from: index,
            dropped: false,
        });
    }
    if let Some(d) = ed
        .drag
        .as_mut()
        .filter(|d| d.voice == voice && d.name == ch)
    {
        if resp.dragged() {
            resp.ctx.set_cursor_icon(CursorIcon::Grabbing);
        }
        if resp.drag_stopped() {
            d.dropped = true;
        }
    }
}

/// While a channel of this list is dragged: a line where it would land, and
/// on release the move. `rows` are the channel rows in order, `end_y` is
/// just below the list.
pub(crate) fn drop_target(
    ui: &egui::Ui,
    ed: &mut ChannelEdit,
    voice: bool,
    rows: &[egui::Rect],
    end_y: f32,
    act: &mut ListActions,
) {
    let Some(d) = ed.drag.as_ref().filter(|d| d.voice == voice) else {
        return;
    };
    let (Some(pos), Some(first)) = (ui.ctx().pointer_latest_pos(), rows.first()) else {
        return;
    };
    // Rows above the pointer: the channel goes after them.
    let slot = rows.iter().filter(|r| r.center().y < pos.y).count();
    let to = if slot > d.from { slot - 1 } else { slot };
    if d.dropped {
        if to != d.from {
            act.move_to = Some((voice, d.name.clone(), to));
        }
        ed.drag = None;
        return;
    }
    if to != d.from {
        let y = match rows.get(slot) {
            Some(r) => r.top() - 1.0,
            None => end_y,
        };
        let x = first.x_range();
        ui.painter().hline(x, y, Stroke::new(2.0_f32, pal().accent));
    }
}

impl App {
    /// The channel lists changed (an admin added, renamed, deleted or moved one).
    pub(crate) fn on_channels(
        &mut self,
        text: Vec<String>,
        voice: Vec<String>,
        renamed: Option<ChannelRenamed>,
    ) {
        let Some(sess) = self.session.as_mut() else {
            return;
        };
        if let Some(r) = renamed {
            if r.voice {
                for ch in [
                    &mut self.voice_channel,
                    &mut self.joining,
                    &mut self.want_voice,
                ] {
                    if ch.as_deref() == Some(r.from.as_str()) {
                        *ch = Some(r.to.clone());
                    }
                }
            } else {
                let mut messages = sess.history.remove(&r.from).unwrap_or_default();
                for m in &mut messages {
                    m.channel = r.to.clone();
                }
                sess.history.insert(r.to.clone(), messages);
                if self.unread.remove(&r.from) {
                    self.unread.insert(r.to.clone());
                }
                if self.current_text == r.from {
                    self.current_text = r.to.clone();
                    self.s.last_text_channel = Some(r.to.clone());
                }
            }
        }
        let added = text
            .iter()
            .find(|c| !sess.text_channels.contains(c))
            .cloned();
        for c in &text {
            sess.history.entry(c.clone()).or_default();
        }
        sess.history.retain(|c, _| text.contains(c));
        self.unread.retain(|c| text.contains(c));
        let ed = &mut self.channel_edit;
        let gone = |x: &Option<(bool, String)>| {
            x.as_ref()
                .is_some_and(|(v, c)| !if *v { &voice } else { &text }.contains(c))
        };
        if gone(&ed.renaming) {
            ed.renaming = None;
        }
        if gone(&ed.confirm_delete) {
            ed.confirm_delete = None;
        }
        let first = text.first().cloned();
        let current_gone = !text.contains(&self.current_text);
        sess.text_channels = text;
        sess.voice_channels = voice;
        if let Some(ch) = added.filter(|_| self.new_channel.awaiting_text) {
            self.new_channel.awaiting_text = false;
            self.select_text_channel(ch);
        } else if let Some(ch) = first.filter(|_| current_gone) {
            self.select_text_channel(ch);
        }
    }

    /// Apply a drag right away (the server's lists follow) and send it.
    pub(crate) fn move_channel(&mut self, voice: bool, name: String, index: usize) {
        let Some(sess) = self.session.as_mut() else {
            return;
        };
        let list = if voice {
            &mut sess.voice_channels
        } else {
            &mut sess.text_channels
        };
        if let Some(pos) = list.iter().position(|c| *c == name) {
            let ch = list.remove(pos);
            list.insert(index.min(list.len()), ch);
        }
        self.net.send(ClientMsg::MoveChannel { name, voice, index });
    }

    /// "Delete channel?" after an admin picks Delete in a channel's menu.
    pub(crate) fn confirm_channel_delete(&mut self, ctx: &egui::Context) {
        let Some((voice, name)) = self.channel_edit.confirm_delete.clone() else {
            return;
        };
        let mut delete = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        let mut cancel = false;
        let (title, body) = if voice {
            (
                format!("Delete {name}?"),
                "This removes the voice channel for everyone. Anyone in it is moved out of voice.",
            )
        } else {
            (
                format!("Delete #{name}?"),
                "This removes the channel for everyone, with all its messages and files. It can't be undone.",
            )
        };
        let modal = egui::Modal::new(egui::Id::new("confirm_channel_delete"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(
                Frame::popup(&ctx.style())
                    .fill(pal().raised)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new(title).size(18.0).strong().color(pal().text));
                ui.add_space(4.0);
                ui.label(RichText::new(body).color(pal().muted));
                ui.add_space(14.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let del =
                        egui::Button::new(RichText::new("Delete").color(Color32::WHITE).strong())
                            .fill(pal().red)
                            .min_size(egui::vec2(84.0, 30.0));
                    if ui.add(del).clicked() {
                        delete = true;
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(egui::vec2(84.0, 30.0)))
                        .clicked()
                    {
                        cancel = true;
                    }
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if delete && !cancel {
            self.net.send(ClientMsg::DeleteChannel { name, voice });
        }
        if delete || cancel {
            self.channel_edit.confirm_delete = None;
        }
    }
}
