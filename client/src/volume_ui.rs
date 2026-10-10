//! The speaker button on video and audio players, with the vertical volume bar
//! that pops up above it while the pointer is over it.

use crate::attach_ui::Act;
use crate::theme::pal;
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Id, Rect, Sense, Stroke, Vec2};

/// Size of the popup holding the vertical bar.
const POPUP: Vec2 = Vec2::new(30.0, 112.0);
/// Height of the bar's track inside the popup.
const TRACK: f32 = 78.0;

/// Speaker button centred at `center` (click to mute or unmute). Hovering it
/// opens a vertical volume bar above it; scrolling over either changes the
/// volume too. Returns the button's width.
pub(crate) fn volume_control(
    ui: &mut egui::Ui,
    level: f32,
    center: egui::Pos2,
    on_dark: bool,
    acts: &mut Vec<Act>,
) -> f32 {
    let muted = level <= 0.001;
    let id = ui.id().with(("vol", center.x as i32, center.y as i32));
    let icon = Rect::from_center_size(center, Vec2::splat(26.0));
    let resp = ui.interact(icon, id, Sense::click());
    let ink = if on_dark { Color32::WHITE } else { pal().muted };
    ui.painter().text(
        icon.center(),
        Align2::CENTER_CENTER,
        if muted { "🔇" } else { "🔊" },
        FontId::proportional(14.0),
        if resp.hovered() { pal().accent } else { ink },
    );
    if resp.clicked() {
        acts.push(Act::ToggleMute);
    }

    // The popup overlaps the button a little so the pointer can move up into it.
    let popup = Rect::from_min_size(
        egui::pos2(center.x - POPUP.x / 2.0, icon.top() - POPUP.y + 2.0),
        POPUP,
    );
    let ctx = ui.ctx().clone();
    let open_id = id.with("open");
    let track_id = id.with("track");
    let was_open = ctx.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
    let pointer_in = ctx
        .pointer_hover_pos()
        .is_some_and(|p| icon.union(popup).contains(p));
    let open = resp.hovered()
        || (was_open && pointer_in)
        || (was_open && ctx.dragged_id() == Some(track_id));
    ctx.data_mut(|d| d.insert_temp(open_id, open));

    if resp.hovered() || (open && pointer_in) {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.5 {
            acts.push(Act::Volume((level + scroll / 400.0).clamp(0.0, 1.0)));
        }
    }
    if open {
        egui::Area::new(id.with("popup"))
            .order(egui::Order::Foreground)
            .fixed_pos(popup.min)
            .show(&ctx, |ui| {
                if let Some(f) = popup_bar(ui, popup, track_id, level, on_dark) {
                    acts.push(Act::Volume(f));
                }
            });
    } else {
        resp.on_hover_text(if muted { "Unmute" } else { "Mute" });
    }
    icon.width()
}

/// Draws the popup with the percentage on top and the bar below it. Returns the
/// new level while the bar is clicked or dragged.
fn popup_bar(
    ui: &mut egui::Ui,
    popup: Rect,
    track_id: Id,
    level: f32,
    on_dark: bool,
) -> Option<f32> {
    let (fill, line, text, bg, fg) = if on_dark {
        (
            Color32::from_black_alpha(200),
            Color32::from_white_alpha(30),
            Color32::WHITE,
            Color32::from_white_alpha(70),
            Color32::WHITE,
        )
    } else {
        (
            pal().raised,
            pal().line,
            pal().muted,
            pal().line,
            pal().accent,
        )
    };
    let p = ui.painter();
    p.rect(
        popup,
        CornerRadius::same(8),
        fill,
        Stroke::new(1.0_f32, line),
        egui::StrokeKind::Inside,
    );
    p.text(
        egui::pos2(popup.center().x, popup.top() + 12.0),
        Align2::CENTER_CENTER,
        format!("{}", (level * 100.0).round()),
        FontId::proportional(11.0),
        text,
    );
    let track = Rect::from_center_size(
        egui::pos2(popup.center().x, popup.top() + 24.0 + TRACK / 2.0),
        Vec2::new(4.0, TRACK),
    );
    // A wider target than the track so it's easy to grab.
    let hit = track.expand2(Vec2::new(9.0, 6.0));
    let resp = ui.interact(hit, track_id, Sense::click_and_drag());
    let p = ui.painter();
    p.rect_filled(track, CornerRadius::same(2), bg);
    let lv = level.clamp(0.0, 1.0);
    let done = Rect::from_min_max(
        egui::pos2(track.left(), track.bottom() - track.height() * lv),
        track.max,
    );
    p.rect_filled(done, CornerRadius::same(2), fg);
    let r = if resp.hovered() || resp.dragged() {
        6.0
    } else {
        5.0
    };
    p.circle_filled(egui::pos2(track.center().x, done.top()), r, fg);
    if resp.clicked() || resp.dragged() {
        let y = resp.interact_pointer_pos()?.y;
        return Some(((track.bottom() - y) / track.height()).clamp(0.0, 1.0));
    }
    None
}
