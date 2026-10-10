//! The emoji picker and inserting an emoji into the message box.

use crate::theme::{self, pal};
use crate::{chat_text, App, RECENT};
use backroom::emoji;
use eframe::egui::{
    self, Align2, CornerRadius, FontId, Frame, Key, Margin, RichText, Sense, Stroke,
};

impl App {
    /// Put an emoji where the cursor is in the message box.
    pub(crate) fn insert_emoji(&mut self, ctx: &egui::Context, e: &str) {
        let id = egui::Id::new("composer");
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let len = self.composer.chars().count();
        let at = state
            .cursor
            .char_range()
            .map(|r| r.primary.index)
            .unwrap_or(len)
            .min(len);
        let byte = self
            .composer
            .char_indices()
            .nth(at)
            .map(|(b, _)| b)
            .unwrap_or(self.composer.len());
        self.composer.insert_str(byte, e);
        let cursor = egui::text::CCursor::new(at + e.chars().count());
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(cursor)));
        state.store(ctx, id);
        self.focus_composer = true;
    }

    pub(crate) fn emoji_picker(&mut self, ctx: &egui::Context) {
        if !self.emoji_open {
            return;
        }
        if self.emoji_tab == RECENT && self.s.recent_emoji.is_empty() {
            self.emoji_tab = 0;
        }
        const COLS: usize = 9;
        const CELL: f32 = 38.0;
        let mut chosen: Option<String> = None;
        let mut hovered: Option<(&'static str, &'static str)> = None;
        let just_opened = self.emoji_opened_frame == ctx.cumulative_frame_nr();
        let area = egui::Area::new(egui::Id::new("emoji_picker"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(self.emoji_anchor.right_top() + egui::vec2(0.0, -8.0))
            .constrain(true)
            .show(ctx, |ui| {
                Frame::popup(ui.style())
                    .fill(pal().raised)
                    .stroke(Stroke::new(1.0_f32, pal().line))
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| {
                        ui.set_width(COLS as f32 * CELL);
                        let search = ui.add(
                            egui::TextEdit::singleline(&mut self.emoji_search)
                                .hint_text(RichText::new("Search emoji").color(pal().faint))
                                .desired_width(f32::INFINITY)
                                .margin(Margin::symmetric(8, 6)),
                        );
                        // A new popup sizes itself on its first frame without taking
                        // input, so keep asking for focus for a few frames.
                        if ctx.cumulative_frame_nr() <= self.emoji_opened_frame + 3 {
                            search.request_focus();
                            ctx.request_repaint();
                        }
                        let enter = search.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.add_space(6.0);
                        // Tabs: recent, then one per group (its first emoji as the icon).
                        let searching = !self.emoji_search.trim().is_empty();
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            let mut tabs: Vec<(u8, &str, &str)> = Vec::new();
                            if !self.s.recent_emoji.is_empty() {
                                tabs.push((RECENT, "🕘", "Recently used"));
                            }
                            for (g, label) in emoji::GROUPS {
                                let icon = emoji::all().iter().find(|e| e.group == g).map_or("", |e| e.emoji);
                                tabs.push((g, icon, label));
                            }
                            for (g, icon, label) in tabs {
                                let (r, resp) = ui.allocate_exact_size(egui::vec2(30.0, 30.0), Sense::click());
                                let selected = !searching && self.emoji_tab == g;
                                if selected || resp.hovered() {
                                    ui.painter().rect_filled(r, CornerRadius::same(6), if selected { pal().raised_2 } else { theme::mix(pal().raised, pal().raised_2, 0.5) });
                                }
                                match backroom::twemoji::name_for(icon) {
                                    Some(n) => chat_text::paint(ui, &mut self.emoji.borrow_mut(), n, r.shrink(6.0)),
                                    None => {
                                        ui.painter().text(r.center(), Align2::CENTER_CENTER, icon, FontId::proportional(16.0), pal().muted);
                                    }
                                }
                                if resp.on_hover_text(label).clicked() {
                                    self.emoji_tab = g;
                                    self.emoji_search.clear();
                                }
                            }
                        });
                        ui.add_space(4.0);
                        let list: Vec<(&'static str, &'static str)> = if searching {
                            emoji::search(&self.emoji_search).into_iter().map(|e| (e.emoji, e.name)).collect()
                        } else if self.emoji_tab == RECENT {
                            self.s
                                .recent_emoji
                                .iter()
                                .filter_map(|r| {
                                    let bare = r.trim_end_matches('\u{FE0F}');
                                    emoji::all().iter().find(|e| e.emoji.trim_end_matches('\u{FE0F}') == bare)
                                })
                                .map(|e| (e.emoji, e.name))
                                .collect()
                        } else {
                            emoji::all().iter().filter(|e| e.group == self.emoji_tab).map(|e| (e.emoji, e.name)).collect()
                        };
                        let rows = list.len().div_ceil(COLS);
                        egui::ScrollArea::vertical()
                            .id_salt(("emoji_scroll", self.emoji_tab, searching))
                            .max_height(CELL * 7.0)
                            .min_scrolled_height(CELL * 7.0)
                            .auto_shrink([false, false])
                            .show_rows(ui, CELL, rows, |ui, range| {
                                ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
                                for row in range {
                                    ui.horizontal(|ui| {
                                        ui.spacing_mut().item_spacing.x = 0.0;
                                        for (e, name) in list.iter().skip(row * COLS).take(COLS) {
                                            let (r, resp) = ui.allocate_exact_size(egui::vec2(CELL, CELL), Sense::click());
                                            if resp.hovered() {
                                                ui.painter().rect_filled(r, CornerRadius::same(6), pal().raised_2);
                                                hovered = Some((e, name));
                                            }
                                            if let Some(n) = backroom::twemoji::name_for(e) {
                                                chat_text::paint(ui, &mut self.emoji.borrow_mut(), n, r.shrink(6.0));
                                            }
                                            if resp.clicked() {
                                                chosen = Some(e.to_string());
                                            }
                                        }
                                    });
                                }
                            });
                        if enter {
                            if let Some((e, _)) = list.first() {
                                chosen = Some(e.to_string());
                            }
                        }
                        if searching && list.is_empty() {
                            ui.label(RichText::new("No emoji found.").color(pal().faint));
                        }
                        // Footer: what's under the pointer, and its :code:.
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.set_min_height(30.0);
                            match hovered {
                                Some((e, name)) => {
                                    let (r, _) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), Sense::hover());
                                    if let Some(n) = backroom::twemoji::name_for(e) {
                                        chat_text::paint(ui, &mut self.emoji.borrow_mut(), n, r);
                                    }
                                    let code = emoji::code_for(e).map(|c| format!("  :{c}:")).unwrap_or_default();
                                    ui.label(RichText::new(format!("{name}{code}")).size(13.0).color(pal().muted));
                                }
                                None => {
                                    ui.label(
                                        RichText::new("Tip: type :) or :fire: and it turns into an emoji when sent.")
                                            .size(12.0)
                                            .color(pal().faint),
                                    );
                                }
                            }
                        });
                    });
            });
        if let Some(e) = chosen {
            self.insert_emoji(ctx, &e);
            let bare = e.trim_end_matches('\u{FE0F}').to_string();
            self.s
                .recent_emoji
                .retain(|r| r.trim_end_matches('\u{FE0F}') != bare);
            self.s.recent_emoji.insert(0, e);
            self.s.recent_emoji.truncate(27);
            self.s.save();
            self.emoji_open = false;
            self.emoji_search.clear();
            return;
        }
        let pressed_elsewhere = !just_opened
            && ctx.input(|i| i.pointer.any_pressed())
            && !area.response.contains_pointer()
            && !self
                .emoji_anchor
                .contains(ctx.input(|i| i.pointer.interact_pos().unwrap_or_default()));
        if pressed_elsewhere || ctx.input(|i| i.key_pressed(Key::Escape)) {
            self.emoji_open = false;
            self.emoji_search.clear();
        }
    }
}
