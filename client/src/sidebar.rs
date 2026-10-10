//! The main screen layout and the left sidebar: channel lists, voice panel, own user row.

use crate::theme::pal;
use crate::widgets::{icon_button, paint_avatar};
use crate::{channels_ui, members, App, VolumePop};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Layout, Margin, RichText, Sense,
    Stroke,
};
use proto::ClientMsg;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tuffcord::keys::{self};

impl App {
    pub(crate) fn main_screen(&mut self, ctx: &egui::Context) {
        let speaking: HashSet<u32> = {
            let mut s: HashSet<u32> = self.mixer.lock().speaking().into_iter().collect();
            if let Some(sess) = &self.session {
                if self.ctl.talking.load(Ordering::Relaxed) {
                    s.insert(sess.me_id);
                }
            }
            s
        };
        self.sidebar(ctx, &speaking);
        self.member_panel(ctx, &speaking);
        self.chat(ctx);
        self.confirm_open_dialog(ctx);
        self.confirm_delete_dialog(ctx);
        self.volume_popup(ctx);
        self.emoji_picker(ctx);
        self.image_viewer(ctx);
        self.drop_overlay(ctx);
        if self.settings_open {
            self.settings_window(ctx);
        }
    }

    pub(crate) fn sidebar(&mut self, ctx: &egui::Context, speaking: &HashSet<u32>) {
        egui::SidePanel::left("sidebar")
            .exact_width(270.0)
            .resizable(false)
            .frame(Frame::new().fill(pal().bg_deep))
            .show(ctx, |ui| {
                // Bottom: voice connection + me.
                egui::TopBottomPanel::bottom("me")
                    .frame(Frame::new().fill(pal().me_bg).inner_margin(Margin {
                        left: 12,
                        right: 8,
                        top: 8,
                        bottom: 10,
                    }))
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        if self.voice_channel.is_some() || self.joining.is_some() {
                            self.voice_panel(ui);
                            ui.add_space(6.0);
                        }
                        self.me_row(ui, speaking);
                    });
                egui::TopBottomPanel::top("brand")
                    .frame(Frame::new().inner_margin(Margin {
                        left: 16,
                        right: 12,
                        top: 14,
                        bottom: 4,
                    }))
                    .show_separator_line(false)
                    .show_inside(ui, |ui| {
                        let name = self
                            .session
                            .as_ref()
                            .map(|s| s.app_name.clone())
                            .unwrap_or_default();
                        ui.label(RichText::new(name).size(19.0).strong());
                    });
                egui::CentralPanel::default()
                    .frame(Frame::new().inner_margin(Margin::symmetric(8, 0)))
                    .show_inside(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink(false)
                            .show(ui, |ui| self.channel_lists(ui, speaking));
                    });
            });
    }

    pub(crate) fn row(
        ui: &mut egui::Ui,
        height: f32,
        selected: bool,
    ) -> (egui::Rect, egui::Response) {
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
        let fill = if selected {
            pal().raised_2
        } else if resp.hovered() {
            pal().raised
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
        (rect, resp)
    }

    pub(crate) fn channel_lists(&mut self, ui: &mut egui::Ui, speaking: &HashSet<u32>) {
        let can_add = self.is_admin() && self.session.as_ref().is_some_and(|s| s.add_channels);
        let Some(sess) = &self.session else { return };
        let mut select_text: Option<String> = None;
        let mut create: Option<(String, bool)> = None;
        let mut join: Option<String> = None;
        let mut open_pop: Option<VolumePop> = None;

        create = create.or(channels_ui::header(
            ui,
            false,
            can_add,
            &mut self.new_channel,
        ));
        for ch in &sess.text_channels {
            let selected = *ch == self.current_text;
            let unread = self.unread.contains(ch);
            let (rect, resp) = Self::row(ui, 30.0, selected);
            let p = ui.painter();
            p.text(
                rect.left_center() + egui::vec2(12.0, 0.0),
                Align2::LEFT_CENTER,
                "#",
                FontId::proportional(16.0),
                pal().faint,
            );
            let color = if selected || unread {
                pal().text
            } else {
                pal().muted
            };
            let font = if unread {
                FontId::new(15.0, egui::FontFamily::Proportional)
            } else {
                FontId::proportional(15.0)
            };
            p.text(
                rect.left_center() + egui::vec2(32.0, 0.0),
                Align2::LEFT_CENTER,
                ch,
                font,
                color,
            );
            if unread {
                p.circle_filled(rect.right_center() - egui::vec2(12.0, 0.0), 3.5, pal().text);
            }
            if resp.clicked() {
                select_text = Some(ch.clone());
            }
        }

        create = create.or(channels_ui::header(
            ui,
            true,
            can_add,
            &mut self.new_channel,
        ));
        for ch in &sess.voice_channels {
            let here = self.voice_channel.as_deref() == Some(ch.as_str());
            let members = sess.members(ch);
            let (rect, resp) = Self::row(ui, 30.0, false);
            let p = ui.painter();
            p.text(
                rect.left_center() + egui::vec2(10.0, 0.0),
                Align2::LEFT_CENTER,
                "🔊",
                FontId::proportional(14.0),
                if here { pal().accent } else { pal().faint },
            );
            p.text(
                rect.left_center() + egui::vec2(32.0, 0.0),
                Align2::LEFT_CENTER,
                ch,
                FontId::proportional(15.0),
                if here { pal().text } else { pal().muted },
            );
            if members.len() >= sess.max_per_voice {
                p.text(
                    rect.right_center() - egui::vec2(10.0, 0.0),
                    Align2::RIGHT_CENTER,
                    "Full",
                    FontId::proportional(12.0),
                    pal().faint,
                );
            }
            if self.joining.as_deref() == Some(ch.as_str()) {
                p.text(
                    rect.right_center() - egui::vec2(10.0, 0.0),
                    Align2::RIGHT_CENTER,
                    "Joining…",
                    FontId::proportional(12.0),
                    pal().accent,
                );
            }
            let resp = resp.on_hover_text(if here {
                format!("You're in {ch}")
            } else {
                format!("Join {ch}")
            });
            if resp.clicked() && !here {
                join = Some(ch.clone());
            }
            for m in members {
                let is_me = m.id == sess.me_id;
                ui.horizontal(|ui| {
                    ui.add_space(22.0);
                    let (rect, resp) = Self::row(ui, 30.0, false);
                    let talking = speaking.contains(&m.id);
                    let p = ui.painter();
                    paint_avatar(
                        p,
                        rect.left_center() + egui::vec2(18.0, 0.0),
                        11.0,
                        &m.name,
                        talking,
                        pal().bg_deep,
                    );
                    let label = if is_me {
                        format!("{} (you)", m.name)
                    } else {
                        m.name.clone()
                    };
                    let locally_muted = !is_me && self.s.local_mutes.contains(&m.name);
                    let color = if talking {
                        pal().accent
                    } else if locally_muted {
                        pal().faint
                    } else {
                        pal().muted
                    };
                    p.text(
                        rect.left_center() + egui::vec2(38.0, 0.0),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(14.5),
                        color,
                    );
                    // Status flags on the right: deafened shows muted too.
                    let x = members::paint_voice_flags(
                        p,
                        egui::pos2(rect.right() - 12.0, rect.center().y),
                        m.muted,
                        m.deafened,
                        12.0,
                    );
                    if locally_muted {
                        let c = egui::pos2(x, rect.center().y);
                        p.text(
                            c,
                            Align2::CENTER_CENTER,
                            "🔊",
                            FontId::proportional(12.0),
                            pal().red,
                        );
                        p.line_segment(
                            [c + egui::vec2(-6.0, -6.0), c + egui::vec2(6.0, 6.0)],
                            Stroke::new(1.5_f32, pal().red),
                        );
                    }
                    if !is_me {
                        let resp = resp.on_hover_text(format!("Volume for {}", m.name));
                        if resp.clicked() {
                            open_pop = Some(VolumePop {
                                name: m.name.clone(),
                                account: m.account,
                                pos: rect.right_top() + egui::vec2(8.0, 0.0),
                                opened_frame: ui.ctx().cumulative_frame_nr(),
                            });
                        }
                    }
                });
            }
        }

        ui.add_space(12.0);

        if let Some(ch) = select_text {
            self.select_text_channel(ch);
        }
        if let Some((name, voice)) = create {
            self.net.send(ClientMsg::CreateChannel { name, voice });
        }
        if let Some(ch) = join {
            self.join_voice(ch);
        }
        if let Some(p) = open_pop {
            self.pop = Some(p);
        }
    }

    pub(crate) fn select_text_channel(&mut self, ch: String) {
        if ch != self.current_text {
            self.player = None;
        }
        self.unread.remove(&ch);
        self.current_text = ch.clone();
        self.s.last_text_channel = Some(ch);
        self.focus_composer = true;
    }

    pub(crate) fn voice_panel(&mut self, ui: &mut egui::Ui) {
        let peers_connecting = self.joining.is_some();
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(
                dot.center(),
                4.5,
                if peers_connecting {
                    pal().accent
                } else {
                    pal().teal
                },
            );
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let status = if peers_connecting {
                    "Joining…".to_string()
                } else {
                    match self.ping {
                        Some(ms) => format!("Voice connected · {ms} ms"),
                        None => "Voice connected".to_string(),
                    }
                };
                ui.label(
                    RichText::new(status)
                        .size(13.0)
                        .strong()
                        .color(if peers_connecting {
                            pal().accent
                        } else {
                            pal().teal
                        }),
                );
                let ch = self
                    .voice_channel
                    .clone()
                    .or(self.joining.clone())
                    .unwrap_or_default();
                ui.label(RichText::new(ch).size(13.0).color(pal().muted));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, "📞", false, pal().red, "Leave voice").clicked() {
                    self.leave_voice();
                }
            });
        });
    }

    pub(crate) fn me_row(&mut self, ui: &mut egui::Ui, speaking: &HashSet<u32>) {
        let Some(sess) = &self.session else { return };
        let me_name = sess.me_name.clone();
        let talking = speaking.contains(&sess.me_id);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), Sense::hover());
            paint_avatar(
                ui.painter(),
                rect.center(),
                15.0,
                &me_name,
                talking,
                pal().me_bg,
            );
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(egui::Label::new(RichText::new(&me_name).strong()).truncate());
                let hint = if self.s.deafened {
                    "Deafened".to_string()
                } else if self.s.muted {
                    "Muted".to_string()
                } else if self.s.push_to_talk {
                    format!("Push to talk: {}", keys::key_name(self.s.ptt_key))
                } else if self.voice_channel.is_some() {
                    "Mic on".to_string()
                } else {
                    "Online".to_string()
                };
                ui.add(
                    egui::Label::new(RichText::new(hint).size(12.0).color(pal().faint)).truncate(),
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icon_button(ui, "⚙", false, pal().muted, "Settings").clicked() {
                    self.settings_open = !self.settings_open;
                }
                let deaf_tip = if self.s.deafened {
                    "Undeafen"
                } else {
                    "Deafen"
                };
                if icon_button(ui, "🎧", self.s.deafened, pal().muted, deaf_tip).clicked() {
                    self.toggle_deafen();
                }
                let muted = self.s.muted || self.s.deafened;
                if icon_button(
                    ui,
                    "🎤",
                    muted,
                    pal().muted,
                    if muted { "Unmute" } else { "Mute" },
                )
                .clicked()
                {
                    self.toggle_mute();
                }
            });
        });
    }
}
