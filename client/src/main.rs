// No console window behind the app in release builds on Windows.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use backroom::audio::{self, DeviceList};
use backroom::keys::{self, GlobalKeys};
use backroom::net::{self, Net, NetEvent, Wake};
use backroom::settings::Settings;
use backroom::voice::{chime, Mixer, VoiceControls};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Frame, Key, Layout, Margin, RichText,
    Sense, Stroke, Vec2,
};
use parking_lot::Mutex;
use proto::update::{self as updates, Release};
use proto::{ChatMessage, ClientMsg, ServerMsg, User, VoiceChannelState};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- palette

const BG_DEEP: Color32 = Color32::from_rgb(23, 17, 30);
const BG: Color32 = Color32::from_rgb(31, 24, 40);
const RAISED: Color32 = Color32::from_rgb(42, 33, 53);
const RAISED_2: Color32 = Color32::from_rgb(52, 41, 66);
const LINE: Color32 = Color32::from_rgb(58, 46, 71);
const ME_BG: Color32 = Color32::from_rgb(20, 15, 26);
const TEXT: Color32 = Color32::from_rgb(239, 232, 244);
const MUTED: Color32 = Color32::from_rgb(163, 150, 177);
const FAINT: Color32 = Color32::from_rgb(122, 109, 137);
const AMBER: Color32 = Color32::from_rgb(244, 184, 96);
const AMBER_INK: Color32 = Color32::from_rgb(43, 27, 6);
const RED: Color32 = Color32::from_rgb(236, 102, 118);
const TEAL: Color32 = Color32::from_rgb(116, 209, 176);

fn main() -> eframe::Result {
    let settings = Settings::load();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Backroom")
            .with_inner_size([1040.0, 700.0])
            .with_min_inner_size([620.0, 420.0])
            .with_icon(app_icon()),
        renderer: eframe::Renderer::Glow,
        vsync: true,
        ..Default::default()
    };
    eframe::run_native(
        "Backroom",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, settings)))),
    )
}

fn app_icon() -> egui::IconData {
    // Three amber level bars on a plum rounded square, drawn at 32x32.
    let n = 32usize;
    let mut rgba = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let r = 8.0;
            let cx = fx.clamp(r, n as f32 - r);
            let cy = fy.clamp(r, n as f32 - r);
            let inside = (fx - cx).powi(2) + (fy - cy).powi(2) <= r * r;
            let bar = |x0: f32, y0: f32, y1: f32| fx >= x0 && fx < x0 + 4.0 && fy >= y0 && fy < y1;
            let px = if !inside {
                [0, 0, 0, 0]
            } else if bar(7.5, 13.0, 20.0) || bar(14.0, 8.0, 24.0) || bar(20.5, 11.0, 22.0) {
                [244, 184, 96, 255]
            } else {
                [42, 33, 53, 255]
            };
            rgba[(y * n + x) * 4..(y * n + x) * 4 + 4].copy_from_slice(&px);
        }
    }
    egui::IconData {
        rgba,
        width: n as u32,
        height: n as u32,
    }
}

// ---------------------------------------------------------------- state

struct Session {
    me_id: u32,
    me_name: String,
    app_name: String,
    text_channels: Vec<String>,
    voice_channels: Vec<String>,
    max_per_voice: usize,
    history: BTreeMap<String, Vec<ChatMessage>>,
    voice_state: Vec<VoiceChannelState>,
    users: Vec<User>,
}

impl Session {
    fn members(&self, channel: &str) -> &[proto::VoiceMember] {
        self.voice_state
            .iter()
            .find(|c| c.name == channel)
            .map(|c| c.members.as_slice())
            .unwrap_or(&[])
    }
}

#[derive(PartialEq)]
enum Conn {
    Offline,
    Connecting,
    Online,
    Reconnecting,
}

struct Banner {
    text: String,
    error: bool,
    until: Option<Instant>,
}

struct VolumePop {
    name: String,
    pos: egui::Pos2,
    opened_frame: u64,
}

#[derive(Default)]
struct UpdateState {
    checking: bool,
    checked: bool,
    latest: Option<Release>,
    error: Option<String>,
}

/// Looks for a newer release in the background. With `repeat`, keeps checking every few hours.
fn spawn_update_check(state: Arc<Mutex<UpdateState>>, wake: Wake, repeat: bool) {
    let _ = std::thread::Builder::new()
        .name("updates".into())
        .spawn(move || {
            if repeat {
                std::thread::sleep(Duration::from_secs(4));
            }
            loop {
                state.lock().checking = true;
                wake();
                let result = updates::check();
                {
                    let mut s = state.lock();
                    s.checking = false;
                    s.checked = true;
                    match result {
                        Ok(latest) => {
                            s.latest = latest;
                            s.error = None;
                        }
                        Err(e) => s.error = Some(e),
                    }
                }
                wake();
                if !repeat {
                    break;
                }
                std::thread::sleep(updates::CHECK_EVERY);
            }
        });
}

struct App {
    s: Settings,
    net: Net,
    mixer: Arc<Mutex<Mixer>>,
    ctl: Arc<VoiceControls>,
    keys: GlobalKeys,
    wake: Wake,
    audio_error: Arc<Mutex<Option<String>>>,
    input: Option<audio::Input>,
    output: Option<audio::Output>,
    input_failed: bool,
    output_failed: bool,
    audio_linger: Option<Instant>,
    devices: Option<DeviceList>,

    conn: Conn,
    login_error: Option<String>,
    session: Option<Session>,
    current_text: String,
    unread: BTreeSet<String>,
    voice_channel: Option<String>,
    joining: Option<String>,
    want_voice: Option<String>,
    ping: Option<u32>,
    composer: String,
    focus_composer: bool,
    banner: Option<Banner>,
    settings_open: bool,
    pop: Option<VolumePop>,
    capturing: bool,
    mem_mb: Option<f64>,
    mem_checked: Option<Instant>,
    update: Arc<Mutex<UpdateState>>,
    update_auto_started: bool,
    update_dismissed: Option<String>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, s: Settings) -> Self {
        setup_style(&cc.egui_ctx);
        let ctx = cc.egui_ctx.clone();
        let wake: Wake = Arc::new(move || ctx.request_repaint());
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        let ctl = Arc::new(VoiceControls::default());
        let net = Net::spawn(mixer.clone(), wake.clone());
        let keys = GlobalKeys::start(ctl.clone(), wake.clone());
        let audio_error = Arc::new(Mutex::new(None));
        let mut app = App {
            s,
            net,
            mixer,
            ctl,
            keys,
            wake,
            audio_error,
            input: None,
            output: None,
            input_failed: false,
            output_failed: false,
            audio_linger: None,
            devices: None,
            conn: Conn::Offline,
            login_error: None,
            session: None,
            current_text: String::new(),
            unread: BTreeSet::new(),
            voice_channel: None,
            joining: None,
            want_voice: None,
            ping: None,
            composer: String::new(),
            focus_composer: false,
            banner: None,
            settings_open: false,
            pop: None,
            capturing: false,
            mem_mb: None,
            mem_checked: None,
            update: Arc::new(Mutex::new(UpdateState::default())),
            update_auto_started: false,
            update_dismissed: None,
        };
        app.apply_voice_flags();
        if app.s.check_updates {
            app.update_auto_started = true;
            spawn_update_check(app.update.clone(), app.wake.clone(), true);
        }
        if app.s.auto_connect
            && app.s.remember
            && !app.s.server.is_empty()
            && !app.s.name.is_empty()
        {
            app.connect();
        }
        app
    }

    // ------------------------------------------------------------ actions

    fn connect(&mut self) {
        let url = match net::normalize_server(&self.s.server) {
            Ok(u) => u,
            Err(e) => {
                self.login_error = Some(e);
                return;
            }
        };
        if self.s.name.trim().is_empty() {
            self.login_error = Some("Pick a name to join.".into());
            return;
        }
        self.login_error = None;
        self.conn = Conn::Connecting;
        self.net
            .connect(url, self.s.name.trim().to_string(), self.s.password.clone());
    }

    fn sign_out(&mut self) {
        self.leave_voice_quietly();
        self.want_voice = None;
        self.net.disconnect();
        self.session = None;
        self.conn = Conn::Offline;
        self.s.auto_connect = false;
        if !self.s.remember {
            self.s.password.clear();
        }
        self.s.save();
        self.settings_open = false;
        self.banner = None;
    }

    fn apply_voice_flags(&mut self) {
        self.ctl.muted.store(self.s.muted, Ordering::Relaxed);
        self.ctl.deafened.store(self.s.deafened, Ordering::Relaxed);
        self.ctl
            .push_to_talk
            .store(self.s.push_to_talk, Ordering::Relaxed);
        self.ctl
            .noise_suppression
            .store(self.s.noise_suppression, Ordering::Relaxed);
        self.ctl.set_threshold_db(self.s.threshold_db);
        self.keys.set_key(self.s.ptt_key);
        self.keys.set_active(self.s.push_to_talk);
        self.mixer.lock().deafened = self.s.deafened;
    }

    fn sound(&self, kind: &str) {
        if self.s.sounds && self.output.is_some() {
            self.mixer.lock().play_system(&chime(kind));
        }
    }

    fn send_status(&self) {
        if self.voice_channel.is_some() {
            self.net.send(ClientMsg::Status {
                muted: self.s.muted,
                deafened: self.s.deafened,
            });
        }
    }

    fn toggle_mute(&mut self) {
        if self.s.deafened {
            // Unmuting while deafened also undeafens.
            self.s.deafened = false;
            self.s.muted = false;
        } else {
            self.s.muted = !self.s.muted;
        }
        self.apply_voice_flags();
        self.send_status();
        self.sound(if self.s.muted { "mute" } else { "unmute" });
        self.s.save();
    }

    fn toggle_deafen(&mut self) {
        self.s.deafened = !self.s.deafened;
        self.apply_voice_flags();
        self.send_status();
        self.sound(if self.s.deafened { "mute" } else { "unmute" });
        self.s.save();
    }

    fn join_voice(&mut self, channel: String) {
        if self.conn != Conn::Online
            || self.voice_channel.as_deref() == Some(channel.as_str())
            || self.joining.is_some()
        {
            return;
        }
        self.input_failed = false;
        self.output_failed = false;
        self.joining = Some(channel.clone());
        self.net.send(ClientMsg::JoinVoice {
            channel,
            muted: self.s.muted,
            deafened: self.s.deafened,
        });
    }

    fn leave_voice(&mut self) {
        if self.voice_channel.is_none() {
            return;
        }
        self.net.send(ClientMsg::LeaveVoice);
        self.sound("leave");
        self.leave_voice_quietly();
        self.audio_linger = Some(Instant::now() + Duration::from_millis(500));
    }

    fn leave_voice_quietly(&mut self) {
        self.voice_channel = None;
        self.joining = None;
        self.ctl.in_voice.store(false, Ordering::Relaxed);
        self.ctl.talking.store(false, Ordering::Relaxed);
        self.mixer.lock().clear();
        self.ping = None;
    }

    fn apply_gains(&self) {
        let Some(sess) = &self.session else { return };
        let mut m = self.mixer.lock();
        for ch in &sess.voice_state {
            for mem in &ch.members {
                m.set_gain(mem.id, self.s.gain(&mem.name));
            }
        }
    }

    fn show_banner(&mut self, text: impl Into<String>, error: bool, secs: Option<u64>) {
        self.banner = Some(Banner {
            text: text.into(),
            error,
            until: secs.map(|s| Instant::now() + Duration::from_secs(s)),
        });
    }

    // ------------------------------------------------------------ audio streams

    /// Mic and speakers run only while you're in voice (or testing in settings).
    fn sync_audio(&mut self) {
        let device_error = self.audio_error.lock().take();
        if let Some(err) = device_error {
            // A device went away: drop the streams so they reopen (on the default device if needed).
            self.input = None;
            self.output = None;
            self.show_banner(err.clone(), true, Some(8));
            self.net.send(ClientMsg::Report {
                kind: if err.starts_with("Microphone") {
                    "mic".into()
                } else {
                    "speaker".into()
                },
                message: err,
            });
        }
        let lingering = self.audio_linger.is_some_and(|t| Instant::now() < t);
        let want = self.voice_channel.is_some()
            || self.joining.is_some()
            || self.settings_open
            || lingering;
        if !want {
            self.input = None;
            self.output = None;
            return;
        }
        if self.input.is_none() && !self.input_failed {
            let hook = self.error_hook();
            match audio::start_input(
                self.s.input_device.as_deref(),
                self.ctl.clone(),
                self.net.voice_sender(),
                self.wake.clone(),
                hook,
            ) {
                Ok(i) => self.input = Some(i),
                Err(e) => {
                    self.input_failed = true;
                    self.show_banner(e.clone(), true, Some(10));
                    self.net.send(ClientMsg::Report {
                        kind: "mic".into(),
                        message: e,
                    });
                }
            }
        }
        if self.output.is_none() && !self.output_failed {
            let hook = self.error_hook();
            match audio::start_output(self.s.output_device.as_deref(), self.mixer.clone(), hook) {
                Ok(o) => self.output = Some(o),
                Err(e) => {
                    self.output_failed = true;
                    self.show_banner(e.clone(), true, Some(10));
                    self.net.send(ClientMsg::Report {
                        kind: "speaker".into(),
                        message: e,
                    });
                }
            }
        }
    }

    fn error_hook(&self) -> audio::ErrorHook {
        let slot = self.audio_error.clone();
        let wake = self.wake.clone();
        Arc::new(move |e| {
            *slot.lock() = Some(e);
            wake();
        })
    }

    // ------------------------------------------------------------ network events

    fn handle_events(&mut self, ctx: &egui::Context) {
        while let Some(ev) = self.net.try_recv() {
            match ev {
                NetEvent::Connecting => {
                    if self.conn == Conn::Offline {
                        self.conn = Conn::Connecting;
                    }
                }
                NetEvent::Failed { message } => {
                    self.leave_voice_quietly();
                    self.session = None;
                    self.conn = Conn::Offline;
                    self.login_error = Some(message);
                    self.banner = None;
                }
                NetEvent::Reconnecting { reason } => {
                    if let Some(ch) = self.voice_channel.clone() {
                        self.want_voice = Some(ch);
                    }
                    self.leave_voice_quietly();
                    self.conn = Conn::Reconnecting;
                    self.show_banner(
                        format!("Connection lost ({reason}). Reconnecting…"),
                        false,
                        None,
                    );
                }
                NetEvent::Ping(ms) => self.ping = Some(ms),
                NetEvent::Server(msg) => self.handle_server(ctx, msg),
            }
        }
    }

    fn handle_server(&mut self, ctx: &egui::Context, msg: ServerMsg) {
        match msg {
            ServerMsg::Welcome {
                id,
                name,
                app_name,
                text_channels,
                voice_channels,
                max_per_voice_channel,
                history,
                voice_state,
                users,
            } => {
                let first = self.session.is_none();
                if !text_channels.contains(&self.current_text) {
                    self.current_text = self
                        .s
                        .last_text_channel
                        .clone()
                        .filter(|c| text_channels.contains(c))
                        .unwrap_or_else(|| text_channels[0].clone());
                }
                self.session = Some(Session {
                    me_id: id,
                    me_name: name,
                    app_name,
                    text_channels,
                    voice_channels,
                    max_per_voice: max_per_voice_channel,
                    history,
                    voice_state,
                    users,
                });
                self.conn = Conn::Online;
                self.login_error = None;
                self.banner = None;
                if first {
                    self.focus_composer = true;
                }
                self.s.auto_connect = self.s.remember;
                self.s.save();
                if let Some(ch) = self.want_voice.take() {
                    self.join_voice(ch);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                    self.session
                        .as_ref()
                        .map(|s| s.app_name.clone())
                        .unwrap_or_default(),
                ));
            }
            ServerMsg::Chat { message } => {
                let Some(sess) = self.session.as_mut() else {
                    return;
                };
                let mine = message.author_id == sess.me_id.to_string();
                let channel = message.channel.clone();
                sess.history
                    .entry(channel.clone())
                    .or_default()
                    .push(message);
                if channel != self.current_text && !mine {
                    self.unread.insert(channel);
                }
                let focused = ctx.input(|i| i.focused);
                if !mine && !focused {
                    self.sound("message");
                    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(
                        egui::UserAttentionType::Informational,
                    ));
                }
            }
            ServerMsg::VoiceState { state } => {
                let Some(sess) = self.session.as_mut() else {
                    return;
                };
                if let Some(ch) = &self.voice_channel {
                    let before: HashSet<u32> = sess.members(ch).iter().map(|m| m.id).collect();
                    sess.voice_state = state;
                    let after: HashSet<u32> = sess.members(ch).iter().map(|m| m.id).collect();
                    if !after.contains(&sess.me_id) {
                        // The server no longer has us in voice.
                        self.leave_voice_quietly();
                    } else {
                        let me = sess.me_id;
                        if after.iter().any(|id| *id != me && !before.contains(id)) {
                            self.sound("join");
                        }
                        if before.iter().any(|id| *id != me && !after.contains(id)) {
                            self.sound("leave");
                        }
                    }
                } else {
                    sess.voice_state = state;
                }
                self.apply_gains();
            }
            ServerMsg::Presence { users } => {
                if let Some(sess) = self.session.as_mut() {
                    sess.users = users;
                }
            }
            ServerMsg::VoiceJoined { channel } => {
                self.joining = None;
                self.voice_channel = Some(channel);
                self.ctl.in_voice.store(true, Ordering::Relaxed);
                self.audio_linger = None;
                self.sync_audio();
                self.sound("join");
                self.apply_gains();
            }
            ServerMsg::Error { code, message } => {
                if code == "voice_full" {
                    self.joining = None;
                }
                self.show_banner(message, true, Some(5));
            }
            ServerMsg::Pong { .. } => {}
        }
    }

    // ------------------------------------------------------------ input outside widgets

    fn handle_keys(&mut self, ctx: &egui::Context) {
        if self.capturing {
            if !GlobalKeys::is_global() {
                let pressed = ctx.input(|i| {
                    i.events.iter().find_map(|e| match e {
                        egui::Event::Key {
                            key, pressed: true, ..
                        } => Some(*key),
                        _ => None,
                    })
                });
                if let Some(k) = pressed {
                    if k == Key::Escape {
                        self.keys.cancel_capture();
                        self.capturing = false;
                    } else if let Some(vk) = keys::vk_from_egui(k) {
                        self.keys.finish_capture(vk);
                    }
                }
            }
            if let Some(vk) = self.keys.take_captured() {
                if vk != 0x1B {
                    self.s.ptt_key = vk;
                    self.apply_voice_flags();
                    self.s.save();
                }
                self.capturing = false;
            } else if !self.keys.is_capturing() && GlobalKeys::is_global() {
                self.capturing = false;
            }
        }
        // Without a global key hook, push-to-talk works while this window has focus.
        if self.s.push_to_talk && !GlobalKeys::is_global() {
            let typing = ctx.memory(|m| m.focused().is_some());
            let held = !typing
                && ctx.input(|i| {
                    egui::Key::ALL
                        .iter()
                        .any(|k| keys::vk_from_egui(*k) == Some(self.s.ptt_key) && i.key_down(*k))
                });
            if held != self.ctl.ptt_held.load(Ordering::Relaxed) {
                self.ctl.ptt_held.store(held, Ordering::Relaxed);
            }
        }
    }
}

// ---------------------------------------------------------------- drawing helpers

fn setup_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = BG_DEEP;
    v.faint_bg_color = RAISED;
    v.code_bg_color = RAISED;
    v.hyperlink_color = AMBER;
    v.selection.bg_fill = Color32::from_rgb(110, 80, 40);
    v.selection.stroke = Stroke::new(1.0, AMBER);
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(14);
    v.window_shadow = egui::Shadow {
        offset: [0, 10],
        blur: 30,
        spread: 0,
        color: Color32::from_black_alpha(110),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(100),
    };
    v.override_text_color = Some(TEXT);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    for (st, fill) in [
        (&mut w.inactive, RAISED_2),
        (&mut w.hovered, LINE),
        (&mut w.active, LINE),
        (&mut w.open, RAISED_2),
    ] {
        st.weak_bg_fill = fill;
        st.bg_fill = fill;
        st.corner_radius = CornerRadius::same(5);
        st.bg_stroke = Stroke::NONE;
    }
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(90, 72, 108));
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        use egui::TextStyle::*;
        s.text_styles.insert(Body, FontId::proportional(15.0));
        s.text_styles.insert(Button, FontId::proportional(15.0));
        s.text_styles.insert(Small, FontId::proportional(12.0));
        s.text_styles.insert(Heading, FontId::proportional(21.0));
        s.text_styles.insert(Monospace, FontId::monospace(14.0));
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 220.0;
        s.visuals.slider_trailing_fill = true;
    });
}

fn hsl(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = (h / 60.0) % 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color32::from_rgb(
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

fn avatar_color(name: &str) -> Color32 {
    const HUES: [f32; 8] = [280.0, 330.0, 15.0, 42.0, 150.0, 190.0, 225.0, 255.0];
    let mut h: u32 = 0;
    for c in name.chars() {
        h = h.wrapping_mul(31).wrapping_add(c as u32);
    }
    hsl(HUES[(h % 8) as usize], 0.45, 0.72)
}

fn initials(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let first = |w: &str| {
        w.chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default()
    };
    match words.as_slice() {
        [a, b, ..] => first(a) + &first(b),
        [a] => first(a),
        [] => "?".into(),
    }
}

/// Round avatar with initials. Speaking adds the amber glow ring.
fn paint_avatar(
    painter: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    name: &str,
    speaking: bool,
    ring_bg: Color32,
) {
    if speaking {
        painter.circle_filled(
            center,
            radius + 6.0,
            Color32::from_rgba_unmultiplied(244, 184, 96, 40),
        );
        painter.circle_filled(center, radius + 4.0, AMBER);
        painter.circle_filled(center, radius + 2.0, ring_bg);
    }
    painter.circle_filled(center, radius, avatar_color(name));
    painter.text(
        center,
        Align2::CENTER_CENTER,
        initials(name),
        FontId::proportional(radius * 0.85),
        Color32::from_rgb(26, 19, 32),
    );
}

/// A square icon button; `crossed` draws a red slash over it (muted, deafened).
fn icon_button(
    ui: &mut egui::Ui,
    glyph: &str,
    crossed: bool,
    color: Color32,
    tip: &str,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(9), RAISED_2);
    }
    let c = if crossed { RED } else { color };
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        glyph,
        FontId::proportional(17.0),
        c,
    );
    if crossed {
        p.line_segment(
            [
                rect.center() + egui::vec2(-9.0, -9.0),
                rect.center() + egui::vec2(9.0, 9.0),
            ],
            Stroke::new(2.2, RED),
        );
    }
    resp.on_hover_text(tip)
}

/// Paper-plane style send arrow, drawn so it doesn't depend on font glyphs.
fn send_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(9), RAISED_2);
    }
    let c = rect.center();
    let pts = vec![
        c + egui::vec2(-8.0, -7.0),
        c + egui::vec2(9.0, 0.0),
        c + egui::vec2(-8.0, 7.0),
        c + egui::vec2(-5.0, 0.0),
    ];
    p.add(egui::Shape::convex_polygon(pts, AMBER, Stroke::NONE));
    resp.on_hover_text("Send (Enter)")
}

fn close_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::click());
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), LINE);
    }
    let c = rect.center();
    let s = Stroke::new(1.6, MUTED);
    p.line_segment([c + egui::vec2(-4.5, -4.5), c + egui::vec2(4.5, 4.5)], s);
    p.line_segment([c + egui::vec2(4.5, -4.5), c + egui::vec2(-4.5, 4.5)], s);
    resp.on_hover_text("Dismiss")
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(8.0);
        ui.label(RichText::new(text).size(13.0).color(FAINT).strong());
    });
    ui.add_space(2.0);
}

fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let btn = egui::Button::new(RichText::new(text).color(AMBER_INK).strong().size(15.0))
        .fill(if enabled {
            AMBER
        } else {
            Color32::from_rgb(150, 118, 70)
        })
        .corner_radius(CornerRadius::same(10))
        .min_size(egui::vec2(ui.available_width(), 40.0));
    ui.add_enabled(enabled, btn)
}

fn day_label(ts: u64) -> String {
    use chrono::{Local, TimeZone};
    let Some(d) = Local.timestamp_millis_opt(ts as i64).single() else {
        return String::new();
    };
    let today = Local::now().date_naive();
    let day = d.date_naive();
    if day == today {
        "Today".into()
    } else if Some(day) == today.pred_opt() {
        "Yesterday".into()
    } else if d.format("%Y").to_string() == Local::now().format("%Y").to_string() {
        d.format("%A, %B %-d").to_string()
    } else {
        d.format("%B %-d, %Y").to_string()
    }
}

fn time_label(ts: u64) -> String {
    use chrono::{Local, TimeZone};
    Local
        .timestamp_millis_opt(ts as i64)
        .single()
        .map(|d| d.format("%-I:%M %p").to_string())
        .unwrap_or_default()
}

/// Message text with clickable links.
fn message_text(ui: &mut egui::Ui, text: &str) {
    for line in text.split('\n') {
        if !line.contains("http://") && !line.contains("https://") {
            ui.add(egui::Label::new(RichText::new(line).color(TEXT)).wrap());
            continue;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let mut rest = line;
            while !rest.is_empty() {
                let next = [rest.find("https://"), rest.find("http://")]
                    .into_iter()
                    .flatten()
                    .min();
                match next {
                    Some(0) => {
                        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                        let mut url = &rest[..end];
                        url = url.trim_end_matches(|c: char| ").,!?;:'\"]".contains(c));
                        if url.len() <= 8 {
                            ui.label(RichText::new(&rest[..end]).color(TEXT));
                            rest = &rest[end..];
                            continue;
                        }
                        ui.hyperlink_to(RichText::new(url).color(AMBER), url);
                        rest = &rest[url.len()..];
                    }
                    Some(i) => {
                        ui.label(RichText::new(&rest[..i]).color(TEXT));
                        rest = &rest[i..];
                    }
                    None => {
                        ui.label(RichText::new(rest).color(TEXT));
                        rest = "";
                    }
                }
            }
        });
    }
}

// ---------------------------------------------------------------- screens

impl App {
    fn login_screen(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG_DEEP))
            .show(ctx, |ui| {
                let card_h = 470.0;
                ui.add_space(((ui.available_height() - card_h) / 2.0).max(16.0));
                ui.vertical_centered(|ui| {
                    Frame::new()
                        .fill(BG)
                        .stroke(Stroke::new(1.0, LINE))
                        .corner_radius(CornerRadius::same(16))
                        .inner_margin(Margin::same(28))
                        .show(ui, |ui| {
                            ui.set_width(330.0);
                            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                // Logo: three level bars.
                                let (rect, _) =
                                    ui.allocate_exact_size(egui::vec2(40.0, 36.0), Sense::hover());
                                for (i, h) in [16.0f32, 34.0, 24.0].iter().enumerate() {
                                    let x = rect.left() + i as f32 * 13.0;
                                    let r = egui::Rect::from_min_max(
                                        egui::pos2(x, rect.center().y - h / 2.0),
                                        egui::pos2(x + 8.0, rect.center().y + h / 2.0),
                                    );
                                    ui.painter().rect_filled(r, CornerRadius::same(4), AMBER);
                                }
                                ui.add_space(6.0);
                                ui.label(RichText::new("Backroom").size(26.0).strong());
                                ui.label(
                                    RichText::new("Voice and chat for the group.").color(MUTED),
                                );
                                ui.add_space(12.0);

                                let busy = self.conn != Conn::Offline;
                                let mut submit = false;
                                for (label, hint, field, secret) in [
                                    ("Server address", "abc.trycloudflare.com", 0, false),
                                    ("Your name", "", 1, false),
                                    ("Group password", "", 2, true),
                                ] {
                                    ui.label(RichText::new(label).size(13.0).color(MUTED).strong());
                                    let value = match field {
                                        0 => &mut self.s.server,
                                        1 => &mut self.s.name,
                                        _ => &mut self.s.password,
                                    };
                                    let edit = egui::TextEdit::singleline(value)
                                        .hint_text(RichText::new(hint).color(FAINT))
                                        .password(secret)
                                        .desired_width(f32::INFINITY)
                                        .margin(Margin::symmetric(10, 8))
                                        .char_limit(if field == 1 { 32 } else { 300 });
                                    let r = ui.add_enabled(!busy, edit);
                                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                        submit = true;
                                    }
                                    ui.add_space(6.0);
                                }
                                ui.checkbox(&mut self.s.remember, "Remember me on this computer");
                                ui.add_space(4.0);
                                if let Some(err) = &self.login_error {
                                    ui.add(
                                        egui::Label::new(RichText::new(err).color(RED).size(13.0))
                                            .wrap(),
                                    );
                                    ui.add_space(4.0);
                                }
                                ui.add_space(6.0);
                                let label = if busy { "Connecting…" } else { "Join" };
                                if primary_button(ui, label, !busy).clicked() || (submit && !busy) {
                                    self.connect();
                                }
                                if busy {
                                    ui.add_space(6.0);
                                    if ui.small_button("Cancel").clicked() {
                                        self.net.disconnect();
                                        self.conn = Conn::Offline;
                                    }
                                }
                            });
                        });
                    let latest = self.update.lock().latest.clone();
                    if let Some(r) = latest {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 330.0).max(0.0) / 2.0);
                            ui.label(
                                RichText::new(format!("Backroom {} is available.", r.version))
                                    .color(MUTED),
                            );
                            ui.hyperlink_to(RichText::new("Download it").color(AMBER), &r.url);
                        });
                    }
                });
            });
    }

    fn main_screen(&mut self, ctx: &egui::Context) {
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
        self.chat(ctx);
        self.volume_popup(ctx);
        if self.settings_open {
            self.settings_window(ctx);
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context, speaking: &HashSet<u32>) {
        egui::SidePanel::left("sidebar")
            .exact_width(270.0)
            .resizable(false)
            .frame(Frame::new().fill(BG_DEEP))
            .show(ctx, |ui| {
                // Bottom: voice connection + me.
                egui::TopBottomPanel::bottom("me")
                    .frame(Frame::new().fill(ME_BG).inner_margin(Margin {
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

    fn row(ui: &mut egui::Ui, height: f32, selected: bool) -> (egui::Rect, egui::Response) {
        let (rect, resp) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::click());
        let fill = if selected {
            RAISED_2
        } else if resp.hovered() {
            RAISED
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
        (rect, resp)
    }

    fn channel_lists(&mut self, ui: &mut egui::Ui, speaking: &HashSet<u32>) {
        let Some(sess) = &self.session else { return };
        let mut select_text: Option<String> = None;
        let mut join: Option<String> = None;
        let mut open_pop: Option<VolumePop> = None;

        section_title(ui, "Text channels");
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
                FAINT,
            );
            let color = if selected || unread { TEXT } else { MUTED };
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
                p.circle_filled(rect.right_center() - egui::vec2(12.0, 0.0), 3.5, TEXT);
            }
            if resp.clicked() {
                select_text = Some(ch.clone());
            }
        }

        section_title(ui, "Voice channels");
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
                if here { AMBER } else { FAINT },
            );
            p.text(
                rect.left_center() + egui::vec2(32.0, 0.0),
                Align2::LEFT_CENTER,
                ch,
                FontId::proportional(15.0),
                if here { TEXT } else { MUTED },
            );
            if members.len() >= sess.max_per_voice {
                p.text(
                    rect.right_center() - egui::vec2(10.0, 0.0),
                    Align2::RIGHT_CENTER,
                    "Full",
                    FontId::proportional(12.0),
                    FAINT,
                );
            }
            if self.joining.as_deref() == Some(ch.as_str()) {
                p.text(
                    rect.right_center() - egui::vec2(10.0, 0.0),
                    Align2::RIGHT_CENTER,
                    "Joining…",
                    FontId::proportional(12.0),
                    AMBER,
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
                        BG_DEEP,
                    );
                    let label = if is_me {
                        format!("{} (you)", m.name)
                    } else {
                        m.name.clone()
                    };
                    let locally_muted = !is_me && self.s.local_mutes.contains(&m.name);
                    let color = if talking {
                        AMBER
                    } else if locally_muted {
                        FAINT
                    } else {
                        MUTED
                    };
                    p.text(
                        rect.left_center() + egui::vec2(38.0, 0.0),
                        Align2::LEFT_CENTER,
                        label,
                        FontId::proportional(14.5),
                        color,
                    );
                    // Status flags on the right.
                    let mut x = rect.right() - 12.0;
                    let mut flag_at = |glyph: &str| {
                        let c = egui::pos2(x, rect.center().y);
                        p.text(
                            c,
                            Align2::CENTER_CENTER,
                            glyph,
                            FontId::proportional(12.0),
                            RED,
                        );
                        p.line_segment(
                            [c + egui::vec2(-6.0, -6.0), c + egui::vec2(6.0, 6.0)],
                            Stroke::new(1.5, RED),
                        );
                        x -= 18.0;
                    };
                    if m.deafened {
                        flag_at("🎧");
                    } else if m.muted {
                        flag_at("🎤");
                    }
                    if locally_muted {
                        flag_at("🔊");
                    }
                    if !is_me {
                        let resp = resp.on_hover_text(format!("Volume for {}", m.name));
                        if resp.clicked() {
                            open_pop = Some(VolumePop {
                                name: m.name.clone(),
                                pos: rect.right_top() + egui::vec2(8.0, 0.0),
                                opened_frame: ui.ctx().cumulative_frame_nr(),
                            });
                        }
                    }
                });
            }
        }

        section_title(
            ui,
            &format!("Online ({})", {
                let names: BTreeSet<&str> = sess.users.iter().map(|u| u.name.as_str()).collect();
                names.len()
            }),
        );
        let names: BTreeSet<&str> = sess.users.iter().map(|u| u.name.as_str()).collect();
        for name in names {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 28.0), Sense::hover());
            let p = ui.painter();
            let c = rect.left_center() + egui::vec2(20.0, 0.0);
            paint_avatar(p, c, 11.0, name, false, BG_DEEP);
            p.circle_filled(c + egui::vec2(8.0, 8.0), 5.0, BG_DEEP);
            p.circle_filled(c + egui::vec2(8.0, 8.0), 3.5, TEAL);
            p.text(
                rect.left_center() + egui::vec2(40.0, 0.0),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(14.5),
                MUTED,
            );
        }
        ui.add_space(12.0);

        if let Some(ch) = select_text {
            self.unread.remove(&ch);
            self.current_text = ch.clone();
            self.s.last_text_channel = Some(ch);
            self.focus_composer = true;
        }
        if let Some(ch) = join {
            self.join_voice(ch);
        }
        if let Some(p) = open_pop {
            self.pop = Some(p);
        }
    }

    fn voice_panel(&mut self, ui: &mut egui::Ui) {
        let peers_connecting = self.joining.is_some();
        ui.horizontal(|ui| {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(
                dot.center(),
                4.5,
                if peers_connecting { AMBER } else { TEAL },
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
                        .color(if peers_connecting { AMBER } else { TEAL }),
                );
                let ch = self
                    .voice_channel
                    .clone()
                    .or(self.joining.clone())
                    .unwrap_or_default();
                ui.label(RichText::new(ch).size(13.0).color(MUTED));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, "📞", false, RED, "Leave voice").clicked() {
                    self.leave_voice();
                }
            });
        });
    }

    fn me_row(&mut self, ui: &mut egui::Ui, speaking: &HashSet<u32>) {
        let Some(sess) = &self.session else { return };
        let me_name = sess.me_name.clone();
        let talking = speaking.contains(&sess.me_id);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(36.0, 36.0), Sense::hover());
            paint_avatar(ui.painter(), rect.center(), 15.0, &me_name, talking, ME_BG);
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
                ui.add(egui::Label::new(RichText::new(hint).size(12.0).color(FAINT)).truncate());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if icon_button(ui, "⚙", false, MUTED, "Settings").clicked() {
                    self.settings_open = !self.settings_open;
                }
                let deaf_tip = if self.s.deafened {
                    "Undeafen"
                } else {
                    "Deafen"
                };
                if icon_button(ui, "🎧", self.s.deafened, MUTED, deaf_tip).clicked() {
                    self.toggle_deafen();
                }
                let muted = self.s.muted || self.s.deafened;
                if icon_button(
                    ui,
                    "🎤",
                    muted,
                    MUTED,
                    if muted { "Unmute" } else { "Mute" },
                )
                .clicked()
                {
                    self.toggle_mute();
                }
            });
        });
    }

    fn chat(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG))
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
                            ui.label(RichText::new("#").size(19.0).color(FAINT));
                            ui.label(RichText::new(&self.current_text).size(18.0).strong());
                        });
                    });
                self.update_bar(ui);
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

    /// "Backroom X is available" bar above the chat.
    fn update_bar(&mut self, ui: &mut egui::Ui) {
        let latest = self.update.lock().latest.clone();
        let Some(r) = latest else { return };
        if self.update_dismissed.as_deref() == Some(r.version.as_str()) {
            return;
        }
        egui::TopBottomPanel::top("update_bar")
            .frame(Frame::new().fill(RAISED_2).inner_margin(Margin {
                left: 20,
                right: 14,
                top: 8,
                bottom: 8,
            }))
            .show_separator_line(false)
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.5, AMBER);
                    ui.label(
                        RichText::new(format!("Backroom {} is available.", r.version)).strong(),
                    );
                    ui.label(RichText::new(format!("You have {}.", updates::CURRENT)).color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Later").clicked() {
                            self.update_dismissed = Some(r.version.clone());
                        }
                        let btn =
                            egui::Button::new(RichText::new("Download").color(AMBER_INK).strong())
                                .fill(AMBER);
                        let resp = ui.add(btn);
                        let resp = if r.notes.is_empty() {
                            resp
                        } else {
                            resp.on_hover_text(format!("What's new:\n{}", r.notes))
                        };
                        if resp.clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(&r.url));
                        }
                    });
                });
            });
    }

    fn messages(&mut self, ui: &mut egui::Ui) {
        let Some(sess) = &self.session else { return };
        let msgs = sess
            .history
            .get(&self.current_text)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        egui::ScrollArea::vertical()
            .stick_to_bottom(true)
            .auto_shrink(false)
            .id_salt(&self.current_text)
            .show(ui, |ui| {
                ui.add_space(12.0);
                if msgs.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(format!(
                                "No messages in #{} yet. Write the first one.",
                                self.current_text
                            ))
                            .color(FAINT),
                        );
                    });
                    return;
                }
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
                            let galley =
                                p.layout_no_wrap(day.clone(), FontId::proportional(12.0), FAINT);
                            let tw = galley.size().x + 24.0;
                            let y = rect.center().y;
                            p.line_segment(
                                [
                                    egui::pos2(rect.left(), y),
                                    egui::pos2(rect.center().x - tw / 2.0, y),
                                ],
                                Stroke::new(1.0, LINE),
                            );
                            p.line_segment(
                                [
                                    egui::pos2(rect.center().x + tw / 2.0, y),
                                    egui::pos2(rect.right(), y),
                                ],
                                Stroke::new(1.0, LINE),
                            );
                            p.galley(rect.center() - galley.size() / 2.0, galley, FAINT);
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
                    ui.horizontal_top(|ui| {
                        ui.add_space(20.0);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(40.0, 40.0), Sense::hover());
                        paint_avatar(ui.painter(), rect.center(), 19.0, &first.author, false, BG);
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.set_max_width((ui.available_width() - 20.0).min(760.0));
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&first.author).strong().color(TEXT));
                                ui.label(
                                    RichText::new(time_label(first.ts)).size(12.0).color(FAINT),
                                );
                            });
                            for m in &msgs[i..j] {
                                message_text(ui, &m.text);
                            }
                        });
                    });
                    ui.add_space(10.0);
                    i = j;
                }
            });
    }

    fn composer(&mut self, ui: &mut egui::Ui) {
        let online = self.conn == Conn::Online;
        Frame::new()
            .fill(RAISED)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(12))
            .inner_margin(Margin {
                left: 14,
                right: 6,
                top: 6,
                bottom: 6,
            })
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let send_w = 40.0;
                    let edit = egui::TextEdit::multiline(&mut self.composer)
                        .hint_text(
                            RichText::new(if online {
                                format!("Message #{}", self.current_text)
                            } else {
                                "Waiting for the connection…".into()
                            })
                            .color(FAINT),
                        )
                        .desired_rows(1)
                        .desired_width(ui.available_width() - send_w - 8.0)
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
                    let send_clicked = send_button(ui).clicked();
                    if (enter || send_clicked) && online {
                        let text = self.composer.trim().to_string();
                        if !text.is_empty() {
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

    fn volume_popup(&mut self, ctx: &egui::Context) {
        let Some(pop) = &self.pop else { return };
        let name = pop.name.clone();
        let mut vol = (self.s.volume(&name) * 100.0).round();
        let mut muted = self.s.local_mutes.contains(&name);
        let area = egui::Area::new(egui::Id::new("volume_pop"))
            .fixed_pos(pop.pos)
            .order(egui::Order::Foreground)
            .constrain(true)
            .show(ctx, |ui| {
                Frame::popup(ui.style())
                    .fill(RAISED)
                    .inner_margin(Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(240.0);
                        ui.label(RichText::new(&name).strong());
                        ui.add_space(4.0);
                        ui.label(RichText::new("Volume").size(13.0).color(MUTED));
                        let changed_vol = ui
                            .add(
                                egui::Slider::new(&mut vol, 0.0..=200.0)
                                    .suffix("%")
                                    .step_by(1.0),
                            )
                            .changed();
                        let changed_mute = ui.checkbox(&mut muted, "Mute for me").changed();
                        (changed_vol, changed_mute)
                    })
            });
        let (changed_vol, changed_mute) = area.inner.inner;
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

    fn settings_window(&mut self, ctx: &egui::Context) {
        if self.devices.is_none() {
            self.devices = Some(audio::list_devices());
        }
        if self
            .mem_checked
            .is_none_or(|t| t.elapsed() > Duration::from_secs(2))
        {
            self.mem_mb = memory_stats::memory_stats().map(|m| m.physical_mem as f64 / 1_048_576.0);
            self.mem_checked = Some(Instant::now());
        }
        let mut open = true;
        let mut reopen_input = false;
        let mut reopen_output = false;
        let mut flags_changed = false;
        let mut sign_out = false;
        egui::Window::new(RichText::new("Voice settings").strong().size(17.0))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .open(&mut open)
            .frame(
                Frame::window(&ctx.style())
                    .fill(BG)
                    .inner_margin(Margin::same(22)),
            )
            .show(ctx, |ui| {
                let max_h = (ctx.screen_rect().height() - 140.0).max(200.0);
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.set_width(400.0);
                        let devices = self.devices.clone().unwrap_or_default();

                        ui.label(RichText::new("Microphone").size(13.0).strong().color(MUTED));
                        let current = self
                            .s
                            .input_device
                            .clone()
                            .unwrap_or_else(|| "System default".into());
                        egui::ComboBox::from_id_salt("input")
                            .width(400.0)
                            .selected_text(current)
                            .show_ui(ui, |ui| {
                                if ui
                                    .selectable_value(
                                        &mut self.s.input_device,
                                        None,
                                        "System default",
                                    )
                                    .changed()
                                {
                                    reopen_input = true;
                                }
                                for d in &devices.inputs {
                                    if ui
                                        .selectable_value(
                                            &mut self.s.input_device,
                                            Some(d.clone()),
                                            d,
                                        )
                                        .changed()
                                    {
                                        reopen_input = true;
                                    }
                                }
                            });
                        // Level meter, with the voice activation threshold marked.
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(400.0, 10.0), Sense::hover());
                        let p = ui.painter();
                        p.rect_filled(rect, CornerRadius::same(5), BG_DEEP);
                        let to_x = |db: f32| {
                            rect.left() + rect.width() * ((db + 70.0) / 70.0).clamp(0.0, 1.0)
                        };
                        let level = self.ctl.level_db();
                        let above = level > self.s.threshold_db;
                        let fill = if !self.s.push_to_talk && above {
                            TEAL
                        } else {
                            Color32::from_rgb(85, 140, 122)
                        };
                        p.rect_filled(
                            egui::Rect::from_min_max(rect.min, egui::pos2(to_x(level), rect.max.y)),
                            CornerRadius::same(5),
                            fill,
                        );
                        if !self.s.push_to_talk {
                            let x = to_x(self.s.threshold_db);
                            p.line_segment(
                                [
                                    egui::pos2(x, rect.top() - 3.0),
                                    egui::pos2(x, rect.bottom() + 3.0),
                                ],
                                Stroke::new(2.0, AMBER),
                            );
                        }
                        let hint = if self.input.is_some() {
                            if self.s.muted || self.s.deafened {
                                "You're muted, but the meter still shows your mic for testing."
                            } else {
                                "Talk to test your mic."
                            }
                        } else {
                            "Microphone isn't available."
                        };
                        ui.label(RichText::new(hint).size(12.5).color(FAINT));
                        ui.add_space(8.0);

                        ui.label(
                            RichText::new("Speakers or headphones")
                                .size(13.0)
                                .strong()
                                .color(MUTED),
                        );
                        let current = self
                            .s
                            .output_device
                            .clone()
                            .unwrap_or_else(|| "System default".into());
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("output")
                                .width(290.0)
                                .selected_text(current)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_value(
                                            &mut self.s.output_device,
                                            None,
                                            "System default",
                                        )
                                        .changed()
                                    {
                                        reopen_output = true;
                                    }
                                    for d in &devices.outputs {
                                        if ui
                                            .selectable_value(
                                                &mut self.s.output_device,
                                                Some(d.clone()),
                                                d,
                                            )
                                            .changed()
                                        {
                                            reopen_output = true;
                                        }
                                    }
                                });
                            if ui.button("Test sound").clicked() {
                                self.mixer.lock().play_system(&chime("join"));
                            }
                        });

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new("When to send your voice")
                                .size(13.0)
                                .strong()
                                .color(MUTED),
                        );
                        ui.horizontal(|ui| {
                            flags_changed |= ui
                                .radio_value(&mut self.s.push_to_talk, false, "When I talk")
                                .changed();
                            flags_changed |= ui
                                .radio_value(&mut self.s.push_to_talk, true, "Push to talk")
                                .changed();
                        });
                        if self.s.push_to_talk {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Key").color(MUTED));
                                let label = if self.capturing {
                                    "Press a key or mouse button…".to_string()
                                } else {
                                    keys::key_name(self.s.ptt_key)
                                };
                                let btn = egui::Button::new(
                                    RichText::new(label).color(if self.capturing {
                                        AMBER_INK
                                    } else {
                                        TEXT
                                    }),
                                )
                                .fill(if self.capturing { AMBER } else { RAISED_2 })
                                .min_size(egui::vec2(220.0, 30.0));
                                if ui.add(btn).clicked() && !self.capturing {
                                    self.capturing = true;
                                    self.keys.begin_capture();
                                }
                            });
                            let note = if GlobalKeys::is_global() {
                                "Works even while another app or a game is in front. Esc cancels."
                            } else {
                                "Works while this window is in front."
                            };
                            ui.label(RichText::new(note).size(12.5).color(FAINT));
                        } else {
                            ui.label(
                                RichText::new(
                                    "Sensitivity: your mic opens when it passes the amber line.",
                                )
                                .size(12.5)
                                .color(FAINT),
                            );
                            if ui
                                .add(
                                    egui::Slider::new(&mut self.s.threshold_db, -70.0..=-10.0)
                                        .show_value(false),
                                )
                                .changed()
                            {
                                flags_changed = true;
                            }
                        }
                        ui.add_space(6.0);
                        flags_changed |= ui
                            .checkbox(
                                &mut self.s.noise_suppression,
                                "Noise suppression (removes fans, typing, background hum)",
                            )
                            .changed();
                        ui.checkbox(&mut self.s.sounds, "Join, leave and message sounds");

                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(4.0);
                        ui.label(RichText::new("Updates").size(13.0).strong().color(MUTED));
                        let (checking, latest, error, checked) = {
                            let u = self.update.lock();
                            (u.checking, u.latest.clone(), u.error.clone(), u.checked)
                        };
                        ui.horizontal(|ui| {
                            let status = if checking {
                                "Checking…".to_string()
                            } else if let Some(r) = &latest {
                                format!("Backroom {} is available.", r.version)
                            } else if let Some(e) = &error {
                                format!("Couldn't check: {e}.")
                            } else if checked {
                                "You're up to date.".to_string()
                            } else {
                                String::new()
                            };
                            ui.label(
                                RichText::new(format!("Version {}. {status}", updates::CURRENT))
                                    .size(13.0),
                            );
                        });
                        ui.horizontal(|ui| {
                            if let Some(r) = &latest {
                                let btn = egui::Button::new(
                                    RichText::new("Download").color(AMBER_INK).strong(),
                                )
                                .fill(AMBER);
                                if ui.add(btn).clicked() {
                                    ui.ctx().open_url(egui::OpenUrl::new_tab(&r.url));
                                }
                            }
                            if ui
                                .add_enabled(!checking, egui::Button::new("Check now"))
                                .clicked()
                            {
                                spawn_update_check(self.update.clone(), self.wake.clone(), false);
                            }
                        });
                        if ui
                            .checkbox(&mut self.s.check_updates, "Check for updates automatically")
                            .changed()
                        {
                            if self.s.check_updates && !self.update_auto_started {
                                self.update_auto_started = true;
                                spawn_update_check(self.update.clone(), self.wake.clone(), true);
                            }
                            self.s.save();
                        }
                        ui.add_space(10.0);
                        ui.separator();
                        ui.add_space(4.0);
                        if let Some(mb) = self.mem_mb {
                            ui.label(
                                RichText::new(format!("Backroom is using {mb:.0} MB of memory."))
                                    .size(12.5)
                                    .color(FAINT),
                            );
                        }
                        ui.add_space(6.0);
                        let signout =
                            egui::Button::new(RichText::new("Sign out").color(RED)).fill(RAISED_2);
                        if ui.add(signout).clicked() {
                            sign_out = true;
                        }
                    });
            });
        if reopen_input {
            self.input = None;
            self.input_failed = false;
        }
        if reopen_output {
            self.output = None;
            self.output_failed = false;
        }
        if flags_changed {
            self.apply_voice_flags();
        }
        if reopen_input || reopen_output || flags_changed {
            self.s.save();
        }
        if sign_out {
            self.sign_out();
            return;
        }
        if !open {
            self.settings_open = false;
            self.devices = None;
            self.capturing = false;
            self.keys.cancel_capture();
            self.s.save();
        }
    }

    fn banner(&mut self, ctx: &egui::Context) {
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
                    .fill(RAISED_2)
                    .stroke(Stroke::new(1.0, if error { RED } else { LINE }))
                    .corner_radius(CornerRadius::same(18))
                    .inner_margin(Margin::symmetric(16, 8))
                    .shadow(ctx.style().visuals.popup_shadow)
                    .show(ui, |ui| {
                        ui.set_max_width(560.0);
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::Label::new(RichText::new(&text).size(13.5).color(if error {
                                    Color32::from_rgb(255, 212, 218)
                                } else {
                                    TEXT
                                }))
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

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_events(ctx);
        self.handle_keys(ctx);
        self.sync_audio();

        if self.session.is_some() && self.conn != Conn::Offline {
            self.main_screen(ctx);
        } else {
            self.login_screen(ctx);
        }
        self.banner(ctx);

        // Redraw only when something can change on its own.
        let in_voice = self.voice_channel.is_some();
        let someone_talking = in_voice
            && (self.ctl.talking.load(Ordering::Relaxed)
                || !self.mixer.lock().speaking().is_empty());
        if self.settings_open {
            ctx.request_repaint_after(Duration::from_millis(60));
        } else if someone_talking {
            ctx.request_repaint_after(Duration::from_millis(120));
        } else if let Some(t) = self.banner.as_ref().and_then(|b| b.until) {
            ctx.request_repaint_after(
                t.saturating_duration_since(Instant::now()) + Duration::from_millis(20),
            );
        } else if self.audio_linger.is_some_and(|t| Instant::now() < t) {
            ctx.request_repaint_after(Duration::from_millis(550));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.s.save();
        self.net.disconnect();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [
            BG_DEEP.r() as f32 / 255.0,
            BG_DEEP.g() as f32 / 255.0,
            BG_DEEP.b() as f32 / 255.0,
            1.0,
        ]
    }
}
