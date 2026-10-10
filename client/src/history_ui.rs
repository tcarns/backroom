//! Older messages, loaded a page at a time when the chat is scrolled to the
//! top (servers 0.10+ keep every message), and the bar that tells admins the
//! server is running out of room for files.

use crate::theme::{self, pal};
use crate::widgets::close_button;
use crate::App;
use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Sense};
use proto::{ChatMessage, ClientMsg};
use std::collections::{HashMap, HashSet};

pub(crate) struct Older {
    supported: bool,
    /// Per channel: where the next page ends (0: there's nothing older).
    /// Missing: not asked yet.
    next: HashMap<String, u64>,
    /// Waiting for a page for this channel.
    pending: Option<String>,
    /// Keep this message in view: older ones were just added above it.
    pub(crate) anchor: Option<String>,
}

impl Older {
    pub(crate) fn new(supported: bool) -> Self {
        Older {
            supported,
            next: HashMap::new(),
            pending: None,
            anchor: None,
        }
    }
}

impl App {
    /// After the message list is drawn: ask for older messages when its top is in view.
    pub(crate) fn after_message_list(&mut self, near_top: bool) {
        let Some(sess) = self.session.as_mut() else {
            return;
        };
        // The list hasn't scrolled to the anchor yet, so its top still looks in view.
        if sess.older.anchor.take().is_some() {
            return;
        }
        let older = &mut sess.older;
        if !near_top || !older.supported || older.pending.is_some() {
            return;
        }
        let channel = self.current_text.clone();
        let before = older.next.get(&channel).copied();
        if before == Some(0) {
            return;
        }
        older.pending = Some(channel.clone());
        self.net.send(ClientMsg::LoadOlder { channel, before });
    }

    /// A page of older messages arrived: add the ones not shown yet above the rest.
    pub(crate) fn on_older(
        &mut self,
        ctx: &egui::Context,
        channel: String,
        messages: Vec<ChatMessage>,
        start: u64,
    ) {
        let Some(sess) = self.session.as_mut() else {
            return;
        };
        if sess.older.pending.as_deref() == Some(channel.as_str()) {
            sess.older.pending = None;
        }
        sess.older.next.insert(channel.clone(), start);
        let list = sess.history.entry(channel.clone()).or_default();
        let have: HashSet<&str> = list.iter().map(|m| m.id.as_str()).collect();
        let new: Vec<ChatMessage> = messages
            .into_iter()
            .filter(|m| !have.contains(m.id.as_str()))
            .collect();
        if !new.is_empty() {
            if channel == self.current_text {
                sess.older.anchor = list.first().map(|m| m.id.clone());
            }
            list.splice(0..0, new);
        }
        ctx.request_repaint();
    }

    /// Top of the message list: loading, or the start of the channel.
    pub(crate) fn older_note(&self, ui: &mut egui::Ui) {
        let Some(sess) = &self.session else { return };
        let ch = &self.current_text;
        let text = if sess.older.pending.as_ref() == Some(ch) {
            "Loading older messages…".to_string()
        } else if sess.older.next.get(ch) == Some(&0) {
            format!("This is the start of #{ch}.")
        } else {
            return;
        };
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            ui.label(RichText::new(text).size(12.5).color(pal().faint));
        });
    }

    /// Admins: the server's storage warning, until it's fixed or closed.
    pub(crate) fn storage_bar(&mut self, ui: &mut egui::Ui) {
        let Some(sess) = self.session.as_mut() else {
            return;
        };
        let Some((text, urgent)) = sess.storage_warning.clone() else {
            return;
        };
        if sess.storage_hidden {
            return;
        }
        let color = if urgent {
            pal().red
        } else {
            theme::mix(pal().red, egui::Color32::from_rgb(240, 180, 40), 0.7)
        };
        egui::TopBottomPanel::top("storage_bar")
            .frame(
                Frame::new()
                    .fill(theme::mix(pal().raised_2, color, 0.22))
                    .inner_margin(Margin {
                        left: 20,
                        right: 14,
                        top: 8,
                        bottom: 8,
                    }),
            )
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.5, color);
                    ui.label(
                        RichText::new(if urgent {
                            "Server storage almost full."
                        } else {
                            "Server storage filling up."
                        })
                        .strong(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if close_button(ui)
                            .on_hover_text("Hide until it changes")
                            .clicked()
                        {
                            sess.storage_hidden = true;
                        }
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&text).color(pal().muted)).wrap(),
                            );
                        });
                    });
                });
            });
    }
}
