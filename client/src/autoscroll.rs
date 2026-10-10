//! Middle-click auto scroll in the chat, as in a web browser: a middle click
//! sets a marker, and moving the pointer above or below it scrolls that way,
//! faster the further away it is. Any click, Esc or another channel stops it;
//! so does letting go after holding the middle button and moving. Repaints
//! only while it's on.

use crate::theme::pal;
use crate::App;
use eframe::egui::{self, CursorIcon, Key, PointerButton, Pos2, Rect, Stroke};
use std::time::{Duration, Instant};

/// Pixels around the marker where nothing scrolls.
const DEAD: f32 = 12.0;
/// The middle button as a push-to-talk key (Windows virtual-key code).
const MIDDLE_VK: u32 = 0x04;

#[derive(Default)]
pub(crate) struct AutoScroll {
    on: Option<Anchor>,
}

struct Anchor {
    at: Pos2,
    since: Instant,
    /// Moved away while the button was still held (hold-and-drag style).
    dragged: bool,
    channel: String,
}

impl AutoScroll {
    pub(crate) fn active(&self) -> bool {
        self.on.is_some()
    }
}

/// Scroll speed (points per second) for the pointer this far from the marker.
fn speed(offset: f32) -> f32 {
    let d = (offset.abs() - DEAD).max(0.0);
    (d * 6.0 + d * d * 0.05).min(8000.0) * offset.signum()
}

impl App {
    /// Handle the middle button over the chat (`area`) and return how far to
    /// scroll the message list this frame (positive: down).
    pub(crate) fn auto_scroll(&mut self, ui: &egui::Ui, area: Rect) -> f32 {
        if self.s.push_to_talk && self.s.ptt_key == MIDDLE_VK {
            self.autoscroll.on = None;
            return 0.0;
        }
        let (pos, middle, released, other, esc, dt) = ui.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.button_pressed(PointerButton::Middle),
                i.pointer.button_released(PointerButton::Middle),
                i.pointer.button_pressed(PointerButton::Primary)
                    || i.pointer.button_pressed(PointerButton::Secondary),
                i.key_pressed(Key::Escape),
                i.stable_dt.min(0.05),
            )
        });
        let Some(a) = &mut self.autoscroll.on else {
            if let Some(p) = pos.filter(|p| middle && area.contains(*p)) {
                self.autoscroll.on = Some(Anchor {
                    at: p,
                    since: Instant::now(),
                    dragged: false,
                    channel: self.current_text.clone(),
                });
                ui.ctx().request_repaint();
            }
            return 0.0;
        };
        let held = ui.input(|i| i.pointer.button_down(PointerButton::Middle));
        let offset = pos.map_or(0.0, |p| p.y - a.at.y);
        if held && offset.abs() > DEAD {
            a.dragged = true;
        }
        let let_go = released && a.dragged && a.since.elapsed() > Duration::from_millis(300);
        if middle || other || esc || let_go || a.channel != self.current_text {
            self.autoscroll.on = None;
            return 0.0;
        }
        let at = a.at;
        paint_marker(ui.ctx(), at);
        let icon = if offset < -DEAD {
            CursorIcon::ResizeNorth
        } else if offset > DEAD {
            CursorIcon::ResizeSouth
        } else {
            CursorIcon::AllScroll
        };
        ui.ctx().set_cursor_icon(icon);
        ui.ctx().request_repaint();
        speed(offset) * dt
    }
}

/// The round marker with an arrow up and down, where the middle click was.
fn paint_marker(ctx: &egui::Context, at: Pos2) {
    let p = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("autoscroll"),
    ));
    p.circle(at, 13.0, pal().raised, Stroke::new(1.0_f32, pal().line));
    let c = pal().muted;
    for dir in [-1.0_f32, 1.0] {
        let tip = at + egui::vec2(0.0, 8.5 * dir);
        let base = at.y + 3.5 * dir;
        p.add(egui::Shape::convex_polygon(
            vec![
                tip,
                egui::pos2(at.x - 4.0, base),
                egui::pos2(at.x + 4.0, base),
            ],
            c,
            Stroke::NONE,
        ));
    }
    p.circle_filled(at, 1.6, c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_is_zero_near_the_marker_and_grows_with_distance() {
        assert_eq!(speed(0.0), 0.0);
        assert_eq!(speed(DEAD), 0.0);
        assert_eq!(speed(-DEAD), 0.0);
        assert!(speed(50.0) > 0.0 && speed(-50.0) < 0.0);
        assert!(speed(200.0) > speed(50.0) * 3.0);
        assert_eq!(speed(10_000.0), 8000.0);
    }
}
