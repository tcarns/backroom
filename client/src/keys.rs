//! Global push-to-talk. On Windows a small thread checks the chosen key (or mouse
//! button) every 10 ms with GetAsyncKeyState, so it works while you're in another
//! app or a game. Elsewhere the UI feeds key state while the window has focus.
//!
//! Keys are stored as Windows virtual-key codes on every platform.

use crate::net::Wake;
use crate::voice::VoiceControls;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

pub struct GlobalKeys {
    shared: Arc<Shared>,
}

struct Shared {
    key: AtomicU32,
    active: AtomicBool,
    capturing: AtomicBool,
    captured: AtomicU32,
}

impl GlobalKeys {
    pub fn start(ctl: Arc<VoiceControls>, wake: Wake) -> Self {
        let shared = Arc::new(Shared {
            key: AtomicU32::new(0xC0),
            active: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            captured: AtomicU32::new(0),
        });
        #[cfg(windows)]
        {
            let s = shared.clone();
            std::thread::Builder::new()
                .name("ptt".into())
                .spawn(move || win::poll(s, ctl, wake))
                .expect("spawn ptt thread");
        }
        #[cfg(not(windows))]
        {
            let _ = (ctl, wake);
        }
        GlobalKeys { shared }
    }

    pub fn set_key(&self, vk: u32) {
        self.shared.key.store(vk, Ordering::Relaxed);
    }
    /// Only watch the key while push-to-talk is on.
    pub fn set_active(&self, on: bool) {
        self.shared.active.store(on, Ordering::Relaxed);
    }
    pub fn begin_capture(&self) {
        self.shared.captured.store(0, Ordering::Relaxed);
        self.shared.capturing.store(true, Ordering::Relaxed);
    }
    pub fn cancel_capture(&self) {
        self.shared.capturing.store(false, Ordering::Relaxed);
    }
    pub fn is_capturing(&self) -> bool {
        self.shared.capturing.load(Ordering::Relaxed)
    }
    /// Called by the UI on non-Windows builds, or to finish a capture from a window key event.
    pub fn finish_capture(&self, vk: u32) {
        self.shared.captured.store(vk, Ordering::Relaxed);
        self.shared.capturing.store(false, Ordering::Relaxed);
    }
    pub fn take_captured(&self) -> Option<u32> {
        match self.shared.captured.swap(0, Ordering::Relaxed) {
            0 => None,
            vk => Some(vk),
        }
    }
    /// True when the OS-level poller handles push-to-talk (works in the background).
    pub fn is_global() -> bool {
        cfg!(windows)
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::time::Duration;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

    fn down(vk: u32) -> bool {
        unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
    }

    pub fn poll(s: Arc<Shared>, ctl: Arc<VoiceControls>, wake: Wake) {
        let mut was_held = false;
        loop {
            if s.capturing.load(Ordering::Relaxed) {
                // Wait for everything to be released (the click on "Change" is still down), then grab the next press.
                while (1..255u32).any(down) {
                    if !s.capturing.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                while s.capturing.load(Ordering::Relaxed) {
                    // Skip left/right click, which would make the mouse unusable.
                    if let Some(vk) =
                        (3..255u32).find(|vk| down(*vk) && !matches!(vk, 0x10 | 0x11 | 0x12))
                    {
                        s.captured.store(vk, Ordering::Relaxed);
                        s.capturing.store(false, Ordering::Relaxed);
                        wake();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                continue;
            }
            if s.active.load(Ordering::Relaxed) {
                let held = down(s.key.load(Ordering::Relaxed));
                if held != was_held {
                    was_held = held;
                    ctl.ptt_held.store(held, Ordering::Relaxed);
                    wake();
                }
                std::thread::sleep(Duration::from_millis(10));
            } else {
                if was_held {
                    was_held = false;
                    ctl.ptt_held.store(false, Ordering::Relaxed);
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Human-readable name for a Windows virtual-key code.
pub fn key_name(vk: u32) -> String {
    let fixed = match vk {
        0x04 => "Middle mouse button",
        0x05 => "Mouse button 4",
        0x06 => "Mouse button 5",
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x13 => "Pause",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left arrow",
        0x26 => "Up arrow",
        0x27 => "Right arrow",
        0x28 => "Down arrow",
        0x2D => "Insert",
        0x2E => "Delete",
        0xA0 => "Left Shift",
        0xA1 => "Right Shift",
        0xA2 => "Left Ctrl",
        0xA3 => "Right Ctrl",
        0xA4 => "Left Alt",
        0xA5 => "Right Alt",
        0xC0 => "` (backtick)",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.into();
    }
    match vk {
        0x30..=0x39 | 0x41..=0x5A => char::from_u32(vk)
            .map(|c| c.to_string())
            .unwrap_or_default(),
        0x60..=0x69 => format!("Numpad {}", vk - 0x60),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        _ => format!("Key {vk:#04x}"),
    }
}

/// Map an egui key to a Windows virtual-key code (for non-Windows builds and window capture).
pub fn vk_from_egui(key: eframe::egui::Key) -> Option<u32> {
    use eframe::egui::Key;
    let name = key.name();
    if name.len() == 1 {
        let c = name.chars().next()?.to_ascii_uppercase();
        if c.is_ascii_alphanumeric() {
            return Some(c as u32);
        }
    }
    Some(match key {
        Key::Space => 0x20,
        Key::Backtick => 0xC0,
        Key::Tab => 0x09,
        Key::Insert => 0x2D,
        Key::Delete => 0x2E,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        Key::Minus => 0xBD,
        Key::Equals => 0xBB,
        Key::OpenBracket => 0xDB,
        Key::CloseBracket => 0xDD,
        Key::Backslash => 0xDC,
        Key::Semicolon => 0xBA,
        Key::Quote => 0xDE,
        Key::Comma => 0xBC,
        Key::Period => 0xBE,
        Key::Slash => 0xBF,
        k => {
            let n = k.name();
            if let Some(num) = n.strip_prefix('F').and_then(|x| x.parse::<u32>().ok()) {
                0x6F + num
            } else {
                return None;
            }
        }
    })
}
