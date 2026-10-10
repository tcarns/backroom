//! The chat pane: header, message list and the message box.

use crate::theme::{self, pal};
use crate::widgets::{
    attach_button, day_label, icon_button, paint_avatar, paint_trash, send_button, time_label,
};
use crate::{attach_ui, chat_text, App, Conn};
use eframe::egui::{
    self, Align, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, RichText, Sense, Stroke,
};
use proto::ClientMsg;
use tuffcord::{emoji, images};

impl App {
    pub(crate) fn chat(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(pal().bg))
            .show(ctx, |ui| {
                egui::TopBottomPanel::top("chat_head")
                    .frame(Frame::new().inner_margin(Margin {
                        left: 20,
                        right: 20,
                        top: 14,
                        bottom: 12,
                    }))
                    .show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("#").size(19.0).color(pal().faint));
                            ui.label(RichText::new(&self.current_text).size(18.0).strong());
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                self.members_button(ui);
                            });
                        });
                    });
                self.update_bar(ui);
                self.storage_bar(ui);
                egui::TopBottomPanel::bottom("composer")
                    .frame(Frame::new().inner_margin(Margin {
                        left: 20,
                        right: 20,
                        top: 8,
                        bottom: 18,
                    }))
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        self.composer(ui);
                    });
                egui::CentralPanel::default()
                    .frame(Frame::new())
                    .show_inside(ui, |ui| {
                        self.messages(ui);
                    });
            });
    }

    pub(crate) fn messages(&mut self, ui: &mut egui::Ui) {
        let mut acts = Vec::new();
        let scroll = self.auto_scroll(ui, ui.max_rect());
        let near_top = self.message_list(ui, scroll, &mut acts);
        self.after_message_list(near_top);
        let ctx = ui.ctx().clone();
        self.apply_acts(&ctx, acts);
    }

    /// `scroll`: how far auto scroll moves the list this frame (positive: down).
    /// Returns true when the top of the list is in view (time to load older messages).
    pub(crate) fn message_list(
        &self,
        ui: &mut egui::Ui,
        scroll: f32,
        acts: &mut Vec<attach_ui::Act>,
    ) -> bool {
        let Some(sess) = &self.session else {
            return false;
        };
        let anchor = sess.older.anchor.as_deref();
        let msgs = sess
            .history
            .get(&self.current_text)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        egui::ScrollArea::vertical()
            // Sticking to the newest message would undo auto scroll upwards.
            .stick_to_bottom(!self.autoscroll.active())
            .auto_shrink(false)
            .id_salt(&self.current_text)
            .show(ui, |ui| {
                self.older_note(ui);
                if scroll != 0.0 {
                    ui.scroll_with_delta_animation(
                        egui::vec2(0.0, -scroll),
                        egui::style::ScrollAnimation::none(),
                    );
                }
                ui.add_space(12.0);
                if msgs.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(format!(
                                "No messages in #{} yet. Write the first one.",
                                self.current_text
                            ))
                            .color(pal().faint),
                        );
                    });
                    return;
                }
                // Admins holding Shift get a delete button on the message under the pointer.
                let deleting = self.is_admin() && ui.input(|inp| inp.modifiers.shift);
                let row = ui.max_rect().x_range();
                let mut delete_target: Option<(egui::Rect, &proto::ChatMessage)> = None;
                let mut i = 0;
                while i < msgs.len() {
                    let first = &msgs[i];
                    let day = day_label(first.ts);
                    if i == 0 || day_label(msgs[i - 1].ts) != day {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.add_space(20.0);
                            let w = ui.available_width() - 20.0;
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(w, 18.0), Sense::hover());
                            let p = ui.painter();
                            let galley = p.layout_no_wrap(
                                day.clone(),
                                FontId::proportional(12.0),
                                pal().faint,
                            );
                            let tw = galley.size().x + 24.0;
                            let y = rect.center().y;
                            p.line_segment(
                                [
                                    egui::pos2(rect.left(), y),
                                    egui::pos2(rect.center().x - tw / 2.0, y),
                                ],
                                Stroke::new(1.0_f32, pal().line),
                            );
                            p.line_segment(
                                [
                                    egui::pos2(rect.center().x + tw / 2.0, y),
                                    egui::pos2(rect.right(), y),
                                ],
                                Stroke::new(1.0_f32, pal().line),
                            );
                            p.galley(rect.center() - galley.size() / 2.0, galley, pal().faint);
                        });
                        ui.add_space(6.0);
                    }
                    // Group consecutive messages from the same person within 5 minutes.
                    let mut j = i + 1;
                    while j < msgs.len()
                        && msgs[j].author_id == first.author_id
                        && msgs[j].author == first.author
                        && msgs[j].ts.saturating_sub(msgs[j - 1].ts) < 5 * 60_000
                        && day_label(msgs[j].ts) == day
                    {
                        j += 1;
                    }
                    let group_top = ui.cursor().top();
                    // Painted behind the first message (it covers the name row too).
                    let first_bg = ui.painter().add(egui::Shape::Noop);
                    ui.horizontal_top(|ui| {
                        ui.add_space(20.0);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::hover());
                        paint_avatar(
                            ui.painter(),
                            rect.center(),
                            19.0,
                            &first.author,
                            false,
                            pal().bg,
                        );
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 20.0).min(760.0));
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&first.author).strong().color(pal().text));
                                ui.label(
                                    RichText::new(time_label(first.ts))
                                        .size(12.0)
                                        .color(pal().faint),
                                );
                            });
                            for (k, m) in msgs[i..j].iter().enumerate() {
                                let top = if k == 0 { group_top } else { ui.cursor().top() };
                                let bg = if k == 0 {
                                    first_bg
                                } else {
                                    ui.painter().add(egui::Shape::Noop)
                                };
                                let body = ui
                                    .vertical(|ui| {
                                        if !m.text.is_empty() {
                                            let link = chat_text::show(
                                                ui,
                                                &m.text,
                                                &mut self.emoji.borrow_mut(),
                                            );
                                            if let Some(url) = link {
                                                acts.push(attach_ui::Act::Link(url));
                                            }
                                        }
                                        for a in &m.attachments {
                                            self.attachment_ui(ui, a, acts);
                                            ui.add_space(4.0);
                                        }
                                    })
                                    .response
                                    .rect;
                                // Older messages were just added above: keep this one in place.
                                if anchor == Some(m.id.as_str()) {
                                    ui.scroll_to_rect(body, Some(Align::TOP));
                                }
                                // The last one in a group also covers the bottom of the avatar.
                                let bottom = if k + 1 == j - i {
                                    body.bottom().max(group_top + 40.0)
                                } else {
                                    body.bottom()
                                };
                                let area = egui::Rect::from_x_y_ranges(
                                    row.min + 8.0..=row.max - 8.0,
                                    top - 2.0..=bottom + 2.0,
                                );
                                // A subtle highlight under the pointer; red when it would delete.
                                if ui.rect_contains_pointer(area) {
                                    let fill = if deleting {
                                        theme::mix(pal().bg, pal().red, 0.1)
                                    } else {
                                        theme::mix(pal().bg, pal().raised, 0.6)
                                    };
                                    ui.painter().set(
                                        bg,
                                        egui::Shape::rect_filled(area, CornerRadius::same(4), fill),
                                    );
                                    if deleting {
                                        delete_target = Some((area, m));
                                    }
                                }
                            }
                        });
                    });
                    ui.add_space(10.0);
                    i = j;
                }
                // Drawn after every message so it sits on top and gets the click.
                if let Some((area, m)) = delete_target {
                    // Fits inside a one-line message, so the pointer stays on it.
                    let r = egui::Rect::from_min_size(
                        egui::pos2(area.right() - 32.0, area.top() + 1.0),
                        egui::vec2(28.0, (area.height() - 2.0).min(24.0)),
                    );
                    let resp = ui
                        .interact(r, ui.id().with(("delete", &m.id)), Sense::click())
                        .on_hover_text("Delete message");
                    let p = ui.painter();
                    p.rect(
                        r,
                        CornerRadius::same(6),
                        if resp.hovered() {
                            pal().red
                        } else {
                            pal().raised
                        },
                        Stroke::new(1.0_f32, pal().line),
                        egui::StrokeKind::Inside,
                    );
                    paint_trash(
                        p,
                        r.center(),
                        if resp.hovered() {
                            Color32::WHITE
                        } else {
                            pal().red
                        },
                    );
                    if resp.clicked() {
                        acts.push(attach_ui::Act::AskDelete(m.clone()));
                    }
                }
            })
            .state
            .offset
            .y
            < 150.0
    }

    pub(crate) fn composer(&mut self, ui: &mut egui::Ui) {
        let online = self.conn == Conn::Online;

        // Files waiting to be sent, and uploads in progress.
        self.pending_ui(ui);

        Frame::new()
            .fill(pal().raised)
            .stroke(Stroke::new(1.0_f32, pal().line))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin {
                left: 4,
                right: 6,
                top: 6,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    // Attach a file.
                    let can_attach = online && self.att.server_supports;
                    let what = if self.att.any_files() {
                        "a file"
                    } else {
                        "an image"
                    };
                    let attach_tip = if !self.att.server_supports {
                        "The server needs updating before files can be sent".to_string()
                    } else if images::has_file_picker() {
                        format!(
                            "Send {what} (you can also drag one in, or paste an image with Ctrl+V)"
                        )
                    } else {
                        "Drag a file onto the window, or paste an image with Ctrl+V".to_string()
                    };
                    if attach_button(ui, can_attach)
                        .on_hover_text(attach_tip)
                        .clicked()
                        && can_attach
                    {
                        if let Err(e) = self.att.pick() {
                            self.show_banner(e, false, Some(6));
                        }
                    }
                    ui.add_space(6.0);

                    let buttons_w = 36.0 * 2.0 + 10.0;
                    let edit = egui::TextEdit::multiline(&mut self.composer)
                        .id(egui::Id::new("composer"))
                        .hint_text(
                            RichText::new(if online {
                                format!("Message #{}", self.current_text)
                            } else {
                                "Waiting for the connection…".into()
                            })
                            .color(pal().faint),
                        )
                        .desired_rows(1)
                        .desired_width(ui.available_width() - buttons_w)
                        .frame(false)
                        .char_limit(2000)
                        .return_key(Some(egui::KeyboardShortcut::new(
                            egui::Modifiers::SHIFT,
                            Key::Enter,
                        )));
                    let resp = ui.add(edit);
                    if self.focus_composer {
                        resp.request_focus();
                        self.focus_composer = false;
                    }
                    let enter = resp.has_focus()
                        && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);

                    let emoji_resp = icon_button(ui, "☺", false, pal().muted, "Emoji");
                    if emoji_resp.clicked() {
                        self.emoji_open = !self.emoji_open;
                        self.emoji_anchor = emoji_resp.rect;
                        self.emoji_opened_frame = ui.ctx().cumulative_frame_nr();
                    }
                    let send_clicked = send_button(ui).clicked();
                    if (enter || send_clicked) && online {
                        let text = emoji::convert(self.composer.trim());
                        if self.att.send(&self.net, &self.current_text, &text) {
                            self.composer.clear();
                        } else if !text.is_empty() {
                            self.net.send(ClientMsg::Chat {
                                channel: self.current_text.clone(),
                                text,
                            });
                            self.composer.clear();
                        }
                        resp.request_focus();
                    }
                });
            });
    }
}
