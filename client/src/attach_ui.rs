//! Drawing attachments in the chat (images, GIFs, videos with the player,
//! audio, other files), the files waiting to be sent, and what happens when
//! they're clicked.
//!
//! Drawing happens with the app borrowed immutably, so clicks are collected as
//! [`Act`]s and carried out afterwards in [`App::apply_acts`].

use crate::attach::{self, Fetch, ImgState, Then};
use crate::theme::{self, pal};
use crate::App;
use backroom::files as xfer;
use backroom::media::{self, Player};
use eframe::egui::{
    self, Align2, Color32, CornerRadius, FontId, Frame, Margin, Rect, RichText, Sense, Stroke,
    TextureHandle, TextureOptions, Vec2,
};
use proto::files::{duration_label, extension, kind_of, runs_code, size_label, Kind};
use proto::Attachment;

/// Something clicked or needed in the chat, done after drawing.
pub enum Act {
    /// Fetch an image (or poster) shown at this size.
    Image(String, String, Vec2),
    Gif(Attachment, Vec2),
    View(Attachment),
    Play(Attachment),
    Toggle,
    Seek(f64),
    Volume(f32),
    Open(Attachment),
    Save(Attachment),
    Link(String),
    Repaint(std::time::Duration),
}

/// The video or audio playing now.
pub struct Playing {
    pub id: String,
    pub player: Player,
    pub frame: Option<TextureHandle>,
    pub failed: Option<String>,
}

const CARD_W: f32 = 420.0;

fn card_frame() -> Frame {
    Frame::new()
        .fill(pal().raised)
        .stroke(Stroke::new(1.0_f32, pal().line))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(10))
}

/// A round play/pause button. Returns the click.
fn round_button(
    ui: &mut egui::Ui,
    center: egui::Pos2,
    radius: f32,
    playing: bool,
    solid: bool,
) -> egui::Response {
    let rect = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    let resp = ui.interact(
        rect,
        ui.id().with(("round", center.x as i32, center.y as i32)),
        Sense::click(),
    );
    let p = ui.painter();
    let fill = if solid {
        Color32::from_black_alpha(if resp.hovered() { 200 } else { 150 })
    } else if resp.hovered() {
        pal().accent
    } else {
        theme::mix(pal().accent, pal().raised, 0.15)
    };
    p.circle_filled(center, radius, fill);
    let ink = if solid { Color32::WHITE } else { pal().bg_deep };
    if playing {
        let w = radius * 0.22;
        let h = radius * 0.8;
        for dx in [-radius * 0.22, radius * 0.22] {
            p.rect_filled(
                Rect::from_center_size(center + egui::vec2(dx, 0.0), Vec2::new(w, h)),
                CornerRadius::same(1),
                ink,
            );
        }
    } else {
        let s = radius * 0.45;
        p.add(egui::Shape::convex_polygon(
            vec![
                center + egui::vec2(-s * 0.75, -s),
                center + egui::vec2(s * 1.1, 0.0),
                center + egui::vec2(-s * 0.75, s),
            ],
            ink,
            Stroke::NONE,
        ));
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A seek bar. Returns where it was clicked or dragged to (0..1).
fn seek_bar(ui: &mut egui::Ui, rect: Rect, fraction: f32, on_dark: bool) -> Option<f32> {
    let resp = ui.interact(
        rect,
        ui.id().with(("seek", rect.min.x as i32, rect.min.y as i32)),
        Sense::click_and_drag(),
    );
    let p = ui.painter();
    let track = Rect::from_center_size(rect.center(), Vec2::new(rect.width(), 4.0));
    let (bg, fg) = if on_dark {
        (Color32::from_white_alpha(70), Color32::WHITE)
    } else {
        (pal().line, pal().accent)
    };
    p.rect_filled(track, CornerRadius::same(2), bg);
    let done = Rect::from_min_size(
        track.min,
        Vec2::new(track.width() * fraction.clamp(0.0, 1.0), 4.0),
    );
    p.rect_filled(done, CornerRadius::same(2), fg);
    if resp.hovered() || resp.dragged() {
        p.circle_filled(egui::pos2(done.right(), track.center().y), 6.0, fg);
    }
    if resp.clicked() || resp.dragged() {
        let x = resp.interact_pointer_pos()?.x;
        return Some(((x - track.left()) / track.width()).clamp(0.0, 1.0));
    }
    None
}

/// Why the last download of this file failed, if it did.
fn failure(att: &attach::Attachments, a: &Attachment) -> Option<String> {
    match att.fetches.get(&a.id) {
        Some(Fetch::Failed(e)) => Some(e.clone()),
        _ => None,
    }
}

fn progress_label(att: &attach::Attachments, a: &Attachment) -> Option<String> {
    if !matches!(att.fetches.get(&a.id), Some(Fetch::Downloading)) {
        return None;
    }
    let (done, total) = att
        .progress
        .lock()
        .get(&a.id)
        .copied()
        .unwrap_or((0, a.size));
    let pct = (done * 100).checked_div(total).unwrap_or(0);
    Some(format!("Downloading {pct}%"))
}

impl App {
    /// One attachment in a message.
    pub fn attachment_ui(&self, ui: &mut egui::Ui, a: &Attachment, acts: &mut Vec<Act>) {
        if a.expired {
            card_frame().show(ui, |ui| {
                ui.set_width((ui.available_width()).min(CARD_W - 20.0));
                ui.label(RichText::new(&a.name).color(pal().muted).size(14.0));
                ui.label(
                    RichText::new("This file was cleared to make room on the server.")
                        .color(pal().faint)
                        .size(12.5),
                );
            });
            return;
        }
        match kind_of(&a.mime) {
            Kind::Image | Kind::Gif => self.image_ui(ui, a, acts),
            Kind::Video => self.video_ui(ui, a, acts),
            Kind::Audio => self.audio_ui(ui, a, acts),
            Kind::File => self.file_ui(ui, a, acts),
        }
    }

    fn image_ui(&self, ui: &mut egui::Ui, a: &Attachment, acts: &mut Vec<Act>) {
        let max = Vec2::new(
            ui.available_width().min(attach::THUMB_MAX.x),
            attach::THUMB_MAX.y,
        );
        let size = attach::display_size(a.width, a.height, max);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let visible = ui.is_rect_visible(rect);
        let gif = kind_of(&a.mime) == Kind::Gif && self.att.any_files();
        if let Some(anim) = self.att.anims.get(&a.id) {
            let (tex, next) = anim.current();
            egui::Image::new((tex.id(), size))
                .corner_radius(CornerRadius::same(8))
                .paint_at(ui, rect);
            if visible && anim.frames.len() > 1 {
                acts.push(Act::Repaint(next));
            }
        } else {
            match self.att.images.get(&a.id) {
                Some(ImgState::Ready(tex)) => {
                    egui::Image::new((tex.id(), size))
                        .corner_radius(CornerRadius::same(8))
                        .paint_at(ui, rect);
                }
                state => {
                    let painter = ui.painter();
                    painter.rect_filled(rect, CornerRadius::same(8), pal().raised);
                    painter.rect_stroke(
                        rect,
                        CornerRadius::same(8),
                        Stroke::new(1.0_f32, pal().line),
                        egui::StrokeKind::Inside,
                    );
                    let label = match state {
                        Some(ImgState::Failed(msg)) => msg.as_str(),
                        _ => "Loading…",
                    };
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(13.0),
                        pal().faint,
                    );
                    if state.is_none() && visible {
                        if gif {
                            acts.push(Act::Gif(a.clone(), size));
                        } else {
                            acts.push(Act::Image(a.id.clone(), a.name.clone(), size));
                        }
                    }
                }
            }
        }
        if gif {
            badge(ui, rect, "GIF");
        }
        let resp = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("{} · {}", a.name, size_label(a.size)));
        if resp.clicked() {
            acts.push(Act::View(a.clone()));
        }
        resp.context_menu(|ui| {
            if ui.button("Save as…").clicked() {
                acts.push(Act::Save(a.clone()));
                ui.close();
            }
        });
    }

    fn video_ui(&self, ui: &mut egui::Ui, a: &Attachment, acts: &mut Vec<Act>) {
        let max = Vec2::new(
            ui.available_width().min(attach::VIDEO_MAX.x),
            attach::VIDEO_MAX.y,
        );
        let size = attach::display_size(a.width, a.height, max);
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let playing = self.player.as_ref().filter(|p| p.id == a.id);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(8), Color32::BLACK);

        // Picture: the playing frame, else the poster.
        let poster_tex = a
            .poster
            .as_ref()
            .and_then(|p| match self.att.images.get(&p.id) {
                Some(ImgState::Ready(t)) => Some(t),
                _ => None,
            });
        let frame_tex = playing.and_then(|p| p.frame.as_ref());
        if let Some(t) = frame_tex.or(poster_tex) {
            egui::Image::new((t.id(), size))
                .corner_radius(CornerRadius::same(8))
                .paint_at(ui, rect);
        } else if let Some(p) = &a.poster {
            if !self.att.images.contains_key(&p.id) && ui.is_rect_visible(rect) {
                acts.push(Act::Image(p.id.clone(), "poster.jpg".into(), size));
            }
        }

        let status = playing.map(|p| p.player.status());
        let failed = playing
            .and_then(|p| p.failed.clone())
            .or(status.as_ref().and_then(|s| s.error.clone()));
        let is_playing = status.as_ref().is_some_and(|s| s.playing);
        let hovered = resp.hovered() || ui.rect_contains_pointer(rect);

        if let Some(err) = failed {
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, CornerRadius::same(8), Color32::from_black_alpha(170));
            painter.text(
                rect.center() - egui::vec2(0.0, 14.0),
                Align2::CENTER_CENTER,
                err,
                FontId::proportional(13.0),
                Color32::WHITE,
            );
            let b = Rect::from_center_size(
                rect.center() + egui::vec2(0.0, 16.0),
                Vec2::new(200.0, 28.0),
            );
            if ui
                .put(
                    b,
                    egui::Button::new(
                        RichText::new("Open in your video player").color(Color32::WHITE),
                    )
                    .fill(Color32::from_white_alpha(40)),
                )
                .clicked()
            {
                acts.push(Act::Open(a.clone()));
            }
        } else if let Some(st) = status {
            // Click the picture to pause or play.
            if resp.clicked() {
                acts.push(Act::Toggle);
            }
            if hovered || !is_playing {
                let bar =
                    Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 40.0), rect.max);
                let painter = ui.painter_at(rect);
                painter.rect_filled(
                    bar,
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 8,
                        se: 8,
                    },
                    Color32::from_black_alpha(150),
                );
                let r = round_button(
                    ui,
                    egui::pos2(bar.left() + 22.0, bar.center().y),
                    13.0,
                    is_playing,
                    true,
                );
                if r.clicked() {
                    acts.push(Act::Toggle);
                }
                let time = format!(
                    "{} / {}",
                    duration_label((st.position * 1000.0) as u64),
                    duration_label((st.duration * 1000.0) as u64)
                );
                let painter = ui.painter_at(rect);
                let tg = painter.layout_no_wrap(time, FontId::proportional(12.5), Color32::WHITE);
                let tx = bar.left() + 44.0;
                painter.galley(
                    egui::pos2(tx, bar.center().y - tg.size().y / 2.0),
                    tg.clone(),
                    Color32::WHITE,
                );
                let right = bar.right() - 70.0;
                let seek = Rect::from_min_max(
                    egui::pos2(tx + tg.size().x + 12.0, bar.top() + 8.0),
                    egui::pos2(right, bar.bottom() - 8.0),
                );
                if seek.width() > 30.0 {
                    let frac = if st.duration > 0.0 {
                        (st.position / st.duration) as f32
                    } else {
                        0.0
                    };
                    if let Some(f) = seek_bar(ui, seek, frac, true) {
                        acts.push(Act::Seek(f as f64 * st.duration));
                    }
                }
                self.volume_button(ui, egui::pos2(bar.right() - 52.0, bar.center().y), acts);
                let open = Rect::from_center_size(
                    egui::pos2(bar.right() - 20.0, bar.center().y),
                    Vec2::splat(26.0),
                );
                let o = ui.interact(open, ui.id().with(("open", &a.id)), Sense::click());
                paint_open_icon(
                    ui.painter(),
                    open.center(),
                    if o.hovered() {
                        pal().accent
                    } else {
                        Color32::WHITE
                    },
                );
                if o.on_hover_text("Open in your video player").clicked() {
                    acts.push(Act::Open(a.clone()));
                }
            }
        } else {
            // Not playing: big play button, or download progress.
            match progress_label(&self.att, a) {
                Some(label) => {
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(
                        Rect::from_center_size(rect.center(), Vec2::new(170.0, 34.0)),
                        CornerRadius::same(17),
                        Color32::from_black_alpha(170),
                    );
                    painter.text(
                        rect.center(),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(13.5),
                        Color32::WHITE,
                    );
                }
                None => {
                    let b = round_button(ui, rect.center(), 26.0, false, true);
                    if b.clicked() || resp.clicked() {
                        acts.push(Act::Play(a.clone()));
                    }
                }
            }
            if a.duration_ms > 0 {
                badge(ui, rect, &duration_label(a.duration_ms));
            }
        }
        resp.on_hover_text(format!("{} · {}", a.name, size_label(a.size)))
            .context_menu(|ui| {
                if ui.button("Open in your video player").clicked() {
                    acts.push(Act::Open(a.clone()));
                    ui.close();
                }
                if ui.button("Save as…").clicked() {
                    acts.push(Act::Save(a.clone()));
                    ui.close();
                }
            });
    }

    fn volume_button(&self, ui: &mut egui::Ui, center: egui::Pos2, acts: &mut Vec<Act>) {
        let rect = Rect::from_center_size(center, Vec2::splat(26.0));
        let resp = ui.interact(
            rect,
            ui.id().with(("vol", center.x as i32, center.y as i32)),
            Sense::click(),
        );
        let muted = self.s.media_volume <= 0.001;
        let glyph = if muted { "🔇" } else { "🔊" };
        ui.painter().text(
            center,
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(14.0),
            if resp.hovered() {
                pal().accent
            } else {
                Color32::WHITE
            },
        );
        let resp = resp.on_hover_text(if muted {
            "Unmute"
        } else {
            "Mute (scroll to change the volume)"
        });
        if resp.clicked() {
            acts.push(Act::Volume(if muted { 0.8 } else { 0.0 }));
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                acts.push(Act::Volume(
                    (self.s.media_volume + scroll / 400.0).clamp(0.0, 1.0),
                ));
            }
        }
    }

    fn audio_ui(&self, ui: &mut egui::Ui, a: &Attachment, acts: &mut Vec<Act>) {
        let playing = self.player.as_ref().filter(|p| p.id == a.id);
        let status = playing.map(|p| p.player.status());
        card_frame()
            .show(ui, |ui| {
                let w = (ui.available_width()).min(CARD_W - 20.0);
                ui.set_width(w);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
                    let is_playing = status.as_ref().is_some_and(|s| s.playing);
                    if progress_label(&self.att, a).is_some() {
                        ui.put(r, egui::Spinner::new().size(22.0).color(pal().accent));
                    } else if round_button(ui, r.center(), 18.0, is_playing, false).clicked() {
                        acts.push(if playing.is_some() {
                            Act::Toggle
                        } else {
                            Act::Play(a.clone())
                        });
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(RichText::new(&a.name).color(pal().text).size(14.0))
                                .truncate(),
                        );
                        let sub = match (&status, progress_label(&self.att, a)) {
                            (_, Some(p)) => p,
                            (None, None) if failure(&self.att, a).is_some() => {
                                failure(&self.att, a).unwrap_or_default()
                            }
                            (Some(s), _) if s.error.is_some() => {
                                s.error.clone().unwrap_or_default()
                            }
                            (Some(s), _) => format!(
                                "{} / {}",
                                duration_label((s.position * 1000.0) as u64),
                                duration_label((s.duration * 1000.0) as u64)
                            ),
                            _ if a.duration_ms > 0 => format!(
                                "{} · {}",
                                duration_label(a.duration_ms),
                                size_label(a.size)
                            ),
                            _ => size_label(a.size),
                        };
                        ui.label(RichText::new(sub).size(12.5).color(pal().faint));
                        if let Some(s) = &status {
                            let (bar, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width() - 40.0, 14.0),
                                Sense::hover(),
                            );
                            let frac = if s.duration > 0.0 {
                                (s.position / s.duration) as f32
                            } else {
                                0.0
                            };
                            if let Some(f) = seek_bar(ui, bar, frac, false) {
                                acts.push(Act::Seek(f as f64 * s.duration));
                            }
                        }
                    });
                });
            })
            .response
            .on_hover_text(format!("{} · {}", a.name, size_label(a.size)))
            .context_menu(|ui| {
                if ui.button("Open in your music player").clicked() {
                    acts.push(Act::Open(a.clone()));
                    ui.close();
                }
                if ui.button("Save as…").clicked() {
                    acts.push(Act::Save(a.clone()));
                    ui.close();
                }
            });
    }

    fn file_ui(&self, ui: &mut egui::Ui, a: &Attachment, acts: &mut Vec<Act>) {
        card_frame().show(ui, |ui| {
            let w = (ui.available_width()).min(CARD_W - 20.0);
            ui.set_width(w);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(40.0, 46.0), Sense::hover());
                let p = ui.painter();
                p.rect_filled(
                    r,
                    CornerRadius::same(6),
                    theme::mix(pal().accent, pal().raised, 0.75),
                );
                p.rect_stroke(
                    r,
                    CornerRadius::same(6),
                    Stroke::new(1.0_f32, theme::mix(pal().accent, pal().raised, 0.4)),
                    egui::StrokeKind::Inside,
                );
                let ext = extension(&a.name).to_uppercase();
                let ext = if ext.is_empty() {
                    "FILE".to_string()
                } else {
                    ext.chars().take(4).collect()
                };
                p.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    ext,
                    FontId::proportional(11.0),
                    pal().accent,
                );
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.set_max_width(w - 170.0);
                    ui.add(
                        egui::Label::new(RichText::new(&a.name).color(pal().text).size(14.0))
                            .truncate(),
                    );
                    let sub = progress_label(&self.att, a)
                        .or_else(|| failure(&self.att, a))
                        .unwrap_or_else(|| size_label(a.size));
                    ui.label(RichText::new(sub).size(12.5).color(pal().faint));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Save").on_hover_text("Save a copy").clicked() {
                        acts.push(Act::Save(a.clone()));
                    }
                    let tip = if runs_code(&a.name) {
                        "This is a program or script. You'll be asked first."
                    } else {
                        "Open with the program Windows uses for this kind of file"
                    };
                    if ui.button("Open").on_hover_text(tip).clicked() {
                        acts.push(Act::Open(a.clone()));
                    }
                });
            });
        });
    }

    /// Files waiting to be sent, and uploads in progress, above the message box.
    pub fn pending_ui(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        let mut cancel = None;
        if !self.att.pending.is_empty() || self.att.preparing > 0 {
            ui.horizontal_wrapped(|ui| {
                for (i, p) in self.att.pending.iter().enumerate() {
                    card_frame().inner_margin(Margin::same(6)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            match &p.preview {
                                Some(t) => {
                                    let s = t.size_vec2();
                                    let scale = (48.0 / s.x.max(s.y)).min(1.0);
                                    ui.add(
                                        egui::Image::new((t.id(), s * scale))
                                            .corner_radius(CornerRadius::same(6)),
                                    );
                                }
                                None => {
                                    let (r, _) = ui
                                        .allocate_exact_size(Vec2::new(40.0, 46.0), Sense::hover());
                                    let label = match p.kind {
                                        Kind::Video => "VIDEO".to_string(),
                                        Kind::Audio => "AUDIO".to_string(),
                                        _ => {
                                            let e = extension(&p.name).to_uppercase();
                                            if e.is_empty() {
                                                "FILE".into()
                                            } else {
                                                e.chars().take(4).collect()
                                            }
                                        }
                                    };
                                    ui.painter().rect_filled(
                                        r,
                                        CornerRadius::same(6),
                                        theme::mix(pal().accent, pal().raised, 0.75),
                                    );
                                    ui.painter().text(
                                        r.center(),
                                        Align2::CENTER_CENTER,
                                        label,
                                        FontId::proportional(10.0),
                                        pal().accent,
                                    );
                                }
                            }
                            ui.vertical(|ui| {
                                ui.set_max_width(150.0);
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.add(
                                    egui::Label::new(RichText::new(&p.name).size(13.0)).truncate(),
                                );
                                let mut sub = size_label(p.size);
                                if p.duration_ms > 0 {
                                    sub = format!("{} · {sub}", duration_label(p.duration_ms));
                                }
                                ui.label(RichText::new(sub).size(12.0).color(pal().faint));
                            });
                            if crate::close_button(ui)
                                .on_hover_text("Don't send this")
                                .clicked()
                            {
                                remove = Some(i);
                            }
                        });
                    });
                }
                if self.att.preparing > 0 {
                    ui.add(egui::Spinner::new().color(pal().muted));
                    ui.label(
                        RichText::new("Getting it ready…")
                            .size(13.0)
                            .color(pal().muted),
                    );
                }
            });
            ui.add_space(6.0);
        }
        for s in &self.att.sending {
            ui.horizontal(|ui| {
                let done = s.done.load(std::sync::atomic::Ordering::Relaxed);
                let frac = if s.total > 0 {
                    done as f32 / s.total as f32
                } else {
                    0.0
                };
                ui.label(
                    RichText::new(format!("Sending {}", s.label))
                        .size(13.0)
                        .color(pal().muted),
                );
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_width(180.0)
                        .desired_height(8.0)
                        .fill(pal().accent),
                );
                ui.label(
                    RichText::new(format!("{} of {}", size_label(done), size_label(s.total)))
                        .size(12.5)
                        .color(pal().faint),
                );
                if ui.small_button("Cancel").clicked() {
                    cancel = Some(s.id);
                }
            });
        }
        if !self.att.sending.is_empty() {
            ui.add_space(4.0);
        }
        if let Some(i) = remove {
            self.att.remove_pending(i);
        }
        if let Some(j) = cancel {
            self.att.cancel_sending(j);
        }
    }

    // ------------------------------------------------------------ doing things

    pub fn apply_acts(&mut self, ctx: &egui::Context, acts: Vec<Act>) {
        let ppp = ctx.pixels_per_point();
        for act in acts {
            match act {
                Act::Image(id, name, size) => {
                    self.att.request_image(&id, &name, size, ppp, &self.net)
                }
                Act::Gif(a, size) => self.att.request_gif(&a, size, ppp, &self.net),
                Act::View(a) => self.att.open_viewer(&a, &self.net),
                Act::Play(a) => {
                    if let Some(path) = self.att.fetched(&a.id).cloned() {
                        self.start_playing(ctx, &a, path);
                    } else {
                        self.att.fetch(&a, Then::Play);
                        if let Some(path) = self.att.fetched(&a.id).cloned() {
                            self.att.then.remove(&a.id);
                            self.start_playing(ctx, &a, path);
                        }
                    }
                }
                Act::Toggle => {
                    if let Some(p) = &self.player {
                        let st = p.player.status();
                        if st.playing {
                            p.player.pause();
                        } else {
                            p.player.play();
                        }
                    }
                }
                Act::Seek(t) => {
                    if let Some(p) = &self.player {
                        p.player.seek(t);
                    }
                }
                Act::Volume(v) => {
                    self.s.media_volume = v;
                    if let Some(p) = &self.player {
                        p.player.set_volume(v);
                    }
                    self.s.save();
                }
                Act::Open(a) => {
                    if runs_code(&a.name) {
                        self.confirm_open = Some(a);
                    } else {
                        self.open_file(&a);
                    }
                }
                Act::Save(a) => {
                    if self.att.any_files() {
                        self.att.save_as(&a);
                    } else {
                        self.show_banner("The server needs updating before files can be saved this way. Open the image and use \"Open in photo viewer\".", false, Some(6));
                    }
                }
                Act::Link(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
                Act::Repaint(after) => ctx.request_repaint_after(after),
            }
        }
    }

    /// Download if needed, then open with the system's program.
    pub fn open_file(&mut self, a: &Attachment) {
        if let Some(path) = self.att.fetched(&a.id).cloned() {
            self.open_now(&path, &a.name);
            return;
        }
        self.att.fetch(a, Then::Open);
        if let Some(path) = self.att.fetched(&a.id).cloned() {
            self.att.then.remove(&a.id);
            self.open_now(&path, &a.name);
        }
    }

    pub fn open_now(&mut self, path: &std::path::Path, name: &str) {
        if let Err(e) = xfer::open_with_system(path, name) {
            self.show_banner(format!("Couldn't open {name} ({e})."), true, Some(6));
        }
    }

    pub fn start_playing(&mut self, ctx: &egui::Context, a: &Attachment, path: std::path::PathBuf) {
        // One at a time.
        self.player = None;
        if !media::supported() {
            self.open_now(&path, &a.name);
            return;
        }
        let ppp = ctx.pixels_per_point().clamp(1.0, 2.0);
        let size = if kind_of(&a.mime) == Kind::Video {
            let s = attach::display_size(a.width, a.height, attach::VIDEO_MAX) * ppp;
            [s.x.round() as u32, s.y.round() as u32]
        } else {
            [0, 0]
        };
        match Player::open(&path, size, self.s.media_volume, self.wake.clone()) {
            Ok(player) => {
                self.player = Some(Playing {
                    id: a.id.clone(),
                    player,
                    frame: None,
                    failed: None,
                })
            }
            Err(e) => {
                self.show_banner(
                    format!(
                        "Couldn't play {} here ({e}). Opening it in your player instead.",
                        a.name
                    ),
                    false,
                    Some(6),
                );
                self.open_now(&path, &a.name);
            }
        }
    }

    /// Each frame: take a new video frame, notice when playback fails.
    pub fn poll_player(&mut self, ctx: &egui::Context) {
        let Some(p) = &mut self.player else { return };
        if let Some(img) = p.player.take_frame() {
            match &mut p.frame {
                Some(t) if t.size() == img.size => t.set(img, TextureOptions::LINEAR),
                _ => {
                    p.frame = Some(ctx.load_texture(
                        format!("video-{}", p.id),
                        img,
                        TextureOptions::LINEAR,
                    ))
                }
            }
        }
        if p.failed.is_none() {
            if let Some(e) = p.player.status().error {
                p.failed = Some(e);
            }
        }
    }

    /// Downloads that finished with something to do.
    pub fn on_fetched(
        &mut self,
        ctx: &egui::Context,
        done: Vec<(String, Then, std::path::PathBuf)>,
    ) {
        for (id, then, path) in done {
            let att = self.find_attachment(&id);
            let Some(a) = att else { continue };
            match then {
                Then::Play => self.start_playing(ctx, &a, path),
                Then::Open => self.open_now(&path, &a.name),
                Then::Nothing => {}
            }
        }
    }

    pub fn find_attachment(&self, id: &str) -> Option<Attachment> {
        self.session
            .as_ref()?
            .history
            .values()
            .flatten()
            .flat_map(|m| &m.attachments)
            .find(|a| a.id == id)
            .cloned()
    }

    /// "Open this program?" before running something someone sent.
    pub fn confirm_open_dialog(&mut self, ctx: &egui::Context) {
        let Some(a) = self.confirm_open.clone() else {
            return;
        };
        let mut close = false;
        let mut go = false;
        egui::Window::new("Open a program?")
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .frame(Frame::popup(&ctx.style()).fill(pal().raised).inner_margin(Margin::same(18)))
            .show(ctx, |ui| {
                ui.set_max_width(360.0);
                ui.label(
                    RichText::new(format!(
                        "{} is a program or script. It can change anything on your PC. Only open it if you trust whoever sent it and expected it.",
                        a.name
                    ))
                    .color(pal().text),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Open anyway").clicked() {
                        go = true;
                    }
                    if ui.button("Save instead").clicked() {
                        self.att.save_as(&a);
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if go {
            self.open_file(&a);
            close = true;
        }
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.confirm_open = None;
        }
    }
}

/// "Open elsewhere": a box with an arrow coming out of its corner.
fn paint_open_icon(p: &egui::Painter, c: egui::Pos2, color: Color32) {
    let s = Stroke::new(1.6_f32, color);
    let b = Rect::from_center_size(c + egui::vec2(-1.5, 1.5), Vec2::splat(11.0));
    // The box, open at the top right where the arrow leaves.
    p.line_segment([b.left_top(), egui::pos2(b.center().x, b.top())], s);
    p.line_segment([b.left_top(), b.left_bottom()], s);
    p.line_segment([b.left_bottom(), b.right_bottom()], s);
    p.line_segment([b.right_bottom(), egui::pos2(b.right(), b.center().y)], s);
    let tip = c + egui::vec2(6.0, -6.0);
    p.line_segment([c, tip], s);
    p.line_segment([tip, tip + egui::vec2(-5.0, 0.0)], s);
    p.line_segment([tip, tip + egui::vec2(0.0, 5.0)], s);
}

/// A small dark label in the corner of a picture ("GIF", "0:07").
fn badge(ui: &egui::Ui, rect: Rect, text: &str) {
    let painter = ui.painter_at(rect);
    let g = painter.layout_no_wrap(text.to_string(), FontId::proportional(11.5), Color32::WHITE);
    let pad = Vec2::new(6.0, 3.0);
    let r = Rect::from_min_size(
        rect.right_bottom() - g.size() - pad * 2.0 - Vec2::splat(6.0),
        g.size() + pad * 2.0,
    );
    painter.rect_filled(r, CornerRadius::same(5), Color32::from_black_alpha(160));
    painter.galley(r.min + pad, g, Color32::WHITE);
}
