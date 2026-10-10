//! Popups over the main screen: delete confirmation, image viewer, drop overlay, volume popup, banner.

use crate::attach::ImgState;
use crate::theme::{self, pal};
use crate::widgets::{close_button, time_label};
use crate::{chat_text, App, Conn};
use tuffcord::images;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, RichText,
    Sense, Stroke,
};
use proto::ClientMsg;
use std::time::{Duration, Instant};

impl App {
    /// "Delete message?" after an admin clicks the trash can.
    pub(crate) fn confirm_delete_dialog(&mut self, ctx: &egui::Context) {
        let Some(m) = self.confirm_delete.clone() else {
            return;
        };
        let mut delete = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter));
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new("confirm_delete"))
            .backdrop_color(Color32::from_black_alpha(140))
            .frame(
                Frame::popup(&ctx.style())
                    .fill(pal().raised)
                    .corner_radius(CornerRadius::same(10))
                    .inner_margin(Margin::same(20)),
            )
            .show(ctx, |ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Delete message?").size(18.0).strong().color(pal().text));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "This removes it for everyone, along with any files attached to it. It can't be undone.",
                    )
                    .color(pal().muted),
                );
                ui.add_space(12.0);
                // What's being deleted, the way it looks in the chat.
                Frame::new()
                    .fill(pal().bg)
                    .stroke(Stroke::new(1.0_f32, pal().line))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&m.author).strong().color(pal().text));
                            ui.label(
                                RichText::new(time_label(m.ts)).size(12.0).color(pal().faint),
                            );
                        });
                        if !m.text.is_empty() {
                            let mut text: String = m.text.chars().take(300).collect();
                            if text.len() < m.text.len() {
                                text.push('…');
                            }
                            chat_text::show(ui, &text, &mut self.emoji.borrow_mut());
                        }
                        for a in &m.attachments {
                            ui.label(
                                RichText::new(format!(
                                    "File: {} ({})",
                                    a.name,
                                    images::size_label(a.size)
                                ))
                                .color(pal().muted),
                            );
                        }
                    });
                ui.add_space(14.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let del = egui::Button::new(
                        RichText::new("Delete").color(Color32::WHITE).strong(),
                    )
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
            let channel = if m.channel.is_empty() {
                self.current_text.clone()
            } else {
                m.channel.clone()
            };
            self.net.send(ClientMsg::DeleteMessage {
                channel,
                id: m.id.clone(),
            });
        }
        if delete || cancel {
            self.confirm_delete = None;
        }
    }

    /// Full-size view of an image, over everything else.
    pub(crate) fn image_viewer(&mut self, ctx: &egui::Context) {
        let Some(v) = &self.att.viewer else { return };
        let screen = ctx.screen_rect();
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let mut open_external = false;
        let white = Color32::WHITE;
        egui::Area::new(egui::Id::new("image_viewer"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, bg) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, CornerRadius::ZERO, Color32::from_black_alpha(220));
                let thumb = match self.att.images.get(&v.id) {
                    Some(ImgState::Ready(t)) => Some(t),
                    _ => None,
                };
                let area = rect.shrink2(egui::vec2(40.0, 70.0));
                let mut image_rect = egui::Rect::NOTHING;
                if let Some(t) = v.full.as_ref().or(thumb) {
                    let s = t.size_vec2();
                    let scale = (area.width() / s.x)
                        .min(area.height() / s.y)
                        .min(1.0 / ctx.pixels_per_point())
                        .max(0.01);
                    let size = s * scale;
                    image_rect = egui::Rect::from_center_size(area.center(), size);
                    egui::Image::new((t.id(), size))
                        .corner_radius(CornerRadius::same(6))
                        .paint_at(ui, image_rect);
                }
                if v.full.is_none() {
                    ui.painter().text(
                        egui::pos2(area.center().x, area.bottom() + 24.0),
                        Align2::CENTER_CENTER,
                        "Loading full size…",
                        FontId::proportional(13.0),
                        Color32::from_gray(190),
                    );
                }
                let bar = egui::Rect::from_min_size(
                    rect.min + egui::vec2(24.0, 16.0),
                    egui::vec2(rect.width() - 48.0, 36.0),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(bar)
                        .layout(Layout::left_to_right(Align::Center)),
                    |ui| {
                        ui.label(RichText::new(&v.name).color(white).strong());
                        ui.label(
                            RichText::new(images::size_label(v.size))
                                .color(Color32::from_gray(190)),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let btn = |t: &str| {
                                egui::Button::new(RichText::new(t).color(white))
                                    .fill(Color32::from_white_alpha(30))
                            };
                            if ui.add(btn("Close")).clicked() {
                                close = true;
                            }
                            if ui.add(btn("Open in photo viewer")).clicked() {
                                open_external = true;
                            }
                        });
                    },
                );
                let pos = ctx.input(|i| i.pointer.interact_pos());
                if bg.clicked() && pos.is_some_and(|p| !image_rect.contains(p) && !bar.contains(p))
                {
                    close = true;
                }
            });
        if open_external {
            let (id, name) = (v.id.clone(), v.name.clone());
            if let Some(a) = self.find_attachment(&id).filter(|_| self.att.any_files()) {
                self.open_file(&a);
            } else {
                match self.att.raw_bytes(&id) {
                    Some(bytes) => {
                        if let Err(e) = images::open_externally(&id, &name, &bytes) {
                            self.show_banner(
                                format!("Couldn't open the image ({e})."),
                                true,
                                Some(6),
                            );
                        }
                    }
                    None => self.show_banner(
                        "The image is still downloading. Try again in a moment.",
                        false,
                        Some(4),
                    ),
                }
            }
        }
        if close {
            self.att.close_viewer();
        }
    }

    /// Images dragged onto the window, and Ctrl+V with an image on the clipboard.
    pub(crate) fn handle_image_input(&mut self, ctx: &egui::Context) {
        if self.session.is_none() || self.conn == Conn::Offline {
            return;
        }
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            if !self.att.server_supports {
                self.show_banner(
                    "The server needs updating before files can be sent.",
                    false,
                    Some(6),
                );
                break;
            }
            if let Err(e) = self.att.add_file(path) {
                self.show_banner(e, false, Some(6));
                break;
            }
        }
        // egui swallows Ctrl+V when the clipboard holds only an image, but the key release
        // still arrives. Either signal counts; the debounce stops a double attach.
        // egui swallows the key press of Ctrl+V when the clipboard holds only an image (no text),
        // but the release still arrives. A V release with no matching press means "paste".
        let mut paste = false;
        ctx.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Paste(_) => paste = true,
                    egui::Event::Key {
                        key: Key::V,
                        pressed: true,
                        ..
                    } => self.v_press_seen = true,
                    egui::Event::Key {
                        key: Key::V,
                        pressed: false,
                        ..
                    } => {
                        if !self.v_press_seen {
                            paste = true;
                        }
                        self.v_press_seen = false;
                    }
                    _ => {}
                }
            }
        });
        if paste
            && self.att.server_supports
            && !self.settings_open
            && self
                .last_paste
                .is_none_or(|t| t.elapsed() > Duration::from_millis(500))
        {
            self.last_paste = Some(Instant::now());
            self.att.paste();
        }
        let (notes, finished) = self.att.poll(ctx);
        for (text, error) in notes {
            if error && self.conn == Conn::Online {
                // So whoever runs the server sees it in their log too.
                self.net.send(ClientMsg::Report {
                    kind: "files".into(),
                    message: text.clone(),
                });
            }
            self.show_banner(text, error, Some(if error { 15 } else { 6 }));
        }
        self.on_fetched(ctx, finished);
        // Uploads that finished: post their message.
        for msg in std::mem::take(&mut self.att.ready_to_post) {
            self.net.send(msg);
        }
        self.poll_player(ctx);
    }

    pub(crate) fn drop_overlay(&self, ctx: &egui::Context) {
        if !ctx.input(|i| !i.raw.hovered_files.is_empty()) {
            return;
        }
        let screen = ctx.screen_rect();
        egui::Area::new(egui::Id::new("drop_overlay"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(false)
            .show(ctx, |ui| {
                let p = ui.painter();
                p.rect_filled(
                    screen,
                    CornerRadius::ZERO,
                    theme::with_alpha(pal().bg_deep, 210),
                );
                let inner = screen.shrink(24.0);
                p.rect_stroke(
                    inner,
                    CornerRadius::same(16),
                    Stroke::new(2.0_f32, pal().accent),
                    egui::StrokeKind::Inside,
                );
                let text = if !self.att.server_supports {
                    "The server needs updating before files can be sent"
                } else if self.att.any_files() {
                    "Drop to send"
                } else {
                    "Drop to send this image"
                };
                p.text(
                    screen.center(),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(20.0),
                    pal().text,
                );
            });
    }

    pub(crate) fn volume_popup(&mut self, ctx: &egui::Context) {
        let Some(pop) = &self.pop else { return };
        let name = pop.name.clone();
        let account = pop.account;
        let pos = pop.pos;
        let mut vol = (self.s.volume(&name) * 100.0).round();
        let mut muted = self.s.local_mutes.contains(&name);
        let area = egui::Area::new(egui::Id::new("volume_pop"))
            .fixed_pos(pos)
            .order(egui::Order::Foreground)
            .constrain(true)
            .show(ctx, |ui| {
                Frame::popup(ui.style())
                    .fill(pal().raised)
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(RichText::new(&name).strong());
                        ui.add_space(4.0);
                        ui.label(RichText::new("Volume").size(13.0).color(pal().muted));
                        let changed_vol = ui
                            .add(
                                egui::Slider::new(&mut vol, 0.0..=200.0)
                                    .suffix("%")
                                    .step_by(1.0),
                            )
                            .changed();
                        let changed_mute = ui.checkbox(&mut muted, "Mute for me").changed();
                        let used_admin = match account {
                            Some(a) => self.admin_popup_buttons(ui, a, &name),
                            None => false,
                        };
                        (changed_vol, changed_mute, used_admin)
                    })
            });
        let (changed_vol, changed_mute, used_admin) = area.inner.inner;
        if used_admin {
            self.pop = None;
            return;
        }
        if changed_vol {
            self.s.volumes.insert(name.clone(), vol / 100.0);
        }
        if changed_mute {
            if muted {
                self.s.local_mutes.insert(name.clone());
            } else {
                self.s.local_mutes.remove(&name);
            }
        }
        if changed_vol || changed_mute {
            self.apply_gains();
            self.s.save();
        }
        // A quick click can press and release within one frame; don't close on the click that opened it.
        let just_opened = self
            .pop
            .as_ref()
            .is_some_and(|p| p.opened_frame == ctx.cumulative_frame_nr());
        let pressed_elsewhere = !just_opened
            && ctx.input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer();
        if pressed_elsewhere || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.pop = None;
        }
    }

    pub(crate) fn banner(&mut self, ctx: &egui::Context) {
        let Some(b) = &self.banner else { return };
        if b.until.is_some_and(|t| Instant::now() > t) {
            self.banner = None;
            return;
        }
        let (text, error) = (b.text.clone(), b.error);
        egui::Area::new(egui::Id::new("banner"))
            .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 14.0))
            .order(egui::Order::Tooltip)
            .interactable(true)
            .show(ctx, |ui| {
                Frame::new()
                    .fill(pal().raised_2)
                    .stroke(Stroke::new(
                        1.0_f32,
                        if error { pal().red } else { pal().line },
                    ))
                    .corner_radius(CornerRadius::same(18))
                    .inner_margin(Margin::symmetric(16, 8))
                    .shadow(ctx.style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        ui.set_max_width(560.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&text).size(13.5).color(pal().text))
                                    .wrap(),
                            );
                            if close_button(ui).clicked() {
                                self.banner = None;
                            }
                        });
                    });
            });
    }
}
