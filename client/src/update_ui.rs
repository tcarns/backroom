//! The "update available" bar, its controls and restarting into a new version.

use crate::theme::pal;
use crate::App;
use tuffcord::images;
use tuffcord::updater::{self, Phase};
use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Sense};
use proto::update::{self as updates, Release};
use std::time::Duration;

impl App {
    /// "TUFFcord X is available" bar above the chat.
    pub(crate) fn update_bar(&mut self, ui: &mut egui::Ui) {
        let latest = self.update.lock().latest.clone();
        let Some(r) = latest else { return };
        if self.update_dismissed.as_deref() == Some(r.version.as_str()) {
            return;
        }
        egui::TopBottomPanel::top("update_bar")
            .frame(Frame::new().fill(pal().raised_2).inner_margin(Margin {
                left: 20,
                right: 14,
                top: 8,
                bottom: 8,
            }))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.5, pal().accent);
                    ui.label(
                        RichText::new(format!("TUFFcord {} is available.", r.version)).strong(),
                    );
                    ui.label(
                        RichText::new(format!("You have {}.", updates::CURRENT)).color(pal().muted),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !self.updater.busy() && ui.button("Later").clicked() {
                            self.update_dismissed = Some(r.version.clone());
                        }
                        self.update_controls(ui, &r);
                    });
                });
            });
    }

    /// "Update now" and its progress, or "Download" when this build can't update itself.
    /// Laid out right to left (callers use a right-to-left layout).
    pub(crate) fn update_controls(&mut self, ui: &mut egui::Ui, r: &Release) {
        let primary = |t: &str| {
            egui::Button::new(RichText::new(t).color(pal().accent_ink).strong()).fill(pal().accent)
        };
        match self.updater.phase() {
            Phase::Idle => {
                if updater::can_install(r) {
                    let mut tip = format!(
                        "Downloads TUFFcord {}, installs it and restarts the app.",
                        r.version
                    );
                    if let Some(ch) = &self.voice_channel {
                        tip.push_str(&format!(" You'll rejoin {ch} automatically."));
                    }
                    if !r.notes.is_empty() {
                        tip.push_str(&format!("\n\nWhat's new:\n{}", r.notes));
                    }
                    if ui.add(primary("Update now")).on_hover_text(tip).clicked() {
                        self.updater.start(r.clone(), self.wake.clone());
                    }
                } else {
                    let resp = ui.add(primary("Download"));
                    let resp = if r.notes.is_empty() {
                        resp
                    } else {
                        resp.on_hover_text(format!("What's new:\n{}", r.notes))
                    };
                    if resp.clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&r.url));
                    }
                }
            }
            Phase::Downloading { done, total } => {
                let frac = if total > 0 {
                    done as f32 / total as f32
                } else {
                    0.0
                };
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_width(160.0)
                        .fill(pal().accent)
                        .text(RichText::new(format!("{:.0}%", frac * 100.0)).color(pal().text)),
                );
                ui.label(
                    RichText::new(format!(
                        "Downloading {}",
                        images::size_label(total.max(done))
                    ))
                    .color(pal().muted),
                );
            }
            Phase::Verifying => {
                ui.label(RichText::new("Checking the download…").color(pal().muted));
                ui.add(egui::Spinner::new().color(pal().muted));
            }
            Phase::Installing => {
                ui.label(RichText::new("Installing…").color(pal().muted));
                ui.add(egui::Spinner::new().color(pal().muted));
            }
            Phase::Ready { .. } => {
                ui.label(RichText::new("Restarting…").color(pal().muted));
                ui.add(egui::Spinner::new().color(pal().muted));
            }
            Phase::Failed(msg) => {
                if ui.button("Download manually").clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&r.url));
                }
                if ui.add(primary("Try again")).clicked() {
                    self.updater.reset();
                    self.updater.start(r.clone(), self.wake.clone());
                }
                ui.label(
                    RichText::new("Update didn't finish.")
                        .color(pal().red)
                        .size(13.0),
                )
                .on_hover_text(format!(
                    "{msg}\n\nNothing was changed; your current version still works."
                ));
            }
        }
    }

    /// The update is installed: start the new copy (it signs back in and rejoins voice) and close.
    pub(crate) fn restart_into_update(&mut self, exe: std::path::PathBuf) {
        let resume = updater::Resume {
            server: self.s.server.clone(),
            name: self.s.name.clone(),
            password: self.acct.group_password.clone().unwrap_or_default(),
            // Signed in with the saved key, the server doesn't send a new one.
            token: self
                .acct
                .token
                .clone()
                .unwrap_or_else(|| self.s.token.clone()),
            voice: self
                .voice_channel
                .clone()
                .or_else(|| self.want_voice.clone()),
            text_channel: Some(self.current_text.clone()),
        };
        self.s.save();
        // Not a crash: don't let the new copy think playing something was the problem.
        self.player = None;
        match updater::relaunch(&exe, &resume) {
            Ok(()) => {
                self.net.disconnect();
                std::thread::sleep(Duration::from_millis(200));
                std::process::exit(0);
            }
            Err(e) => {
                self.updater.reset();
                self.show_banner(format!("TUFFcord was updated but couldn't restart itself ({e}). Close it and open it again."), true, None);
            }
        }
    }
}
