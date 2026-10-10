//! Attachments in the app: files waiting to be sent, uploads in progress,
//! files received (images and video posters fetched only when they scroll into
//! view and decoded at display size, GIFs animated, anything else downloaded on
//! request), and the full-size image viewer.
//!
//! Files travel over HTTP on servers from 0.7 (see `tuffcord::files`). Older
//! servers only take images, over the WebSocket, as before.
//!
//! Slow work (reading, shrinking, probing, uploading, downloading, decoding)
//! happens on short-lived worker threads; results come back over a channel and
//! become textures on the UI thread.

use tuffcord::files::{self as xfer, Endpoint, Source};
use tuffcord::images::{self, Prepared};
use tuffcord::media;
use tuffcord::net::{Net, Wake};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};
use parking_lot::Mutex;
use proto::files::{kind_of, Kind};
use proto::{Attachment, ClientMsg, PostFile};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Largest size an image is shown at in the chat.
pub const THUMB_MAX: Vec2 = Vec2::new(420.0, 320.0);
/// Largest size a video is shown at in the chat.
pub const VIDEO_MAX: Vec2 = Vec2::new(480.0, 320.0);
/// Up to this many files can wait to be sent at once.
pub const MAX_PENDING: usize = 10;
/// Downloaded image bytes kept around for the viewer (older servers only).
const RAW_BUDGET: usize = 32 * 1024 * 1024;
/// Chat thumbnails kept as textures (oldest dropped first, re-decoded if needed).
const MAX_THUMBS: usize = 150;
/// Animated GIFs kept in memory at once (oldest dropped first).
const MAX_ANIMS: usize = 8;
/// Most memory one GIF's frames may take; longer ones stop early.
const ANIM_BUDGET: usize = 24 * 1024 * 1024;

pub struct Pending {
    pub name: String,
    pub kind: Kind,
    pub source: Source,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    /// Videos: a JPEG of one frame.
    pub poster: Option<Vec<u8>>,
    pub preview: Option<TextureHandle>,
}

/// An upload in progress.
pub struct Sending {
    pub id: u64,
    pub label: String,
    pub done: Arc<AtomicU64>,
    pub total: u64,
    pub cancel: Arc<AtomicBool>,
}

struct Ready {
    name: String,
    kind: Kind,
    source: Source,
    size: u64,
    width: u32,
    height: u32,
    duration_ms: u64,
    poster: Option<Vec<u8>>,
    preview: Option<ColorImage>,
}

enum Work {
    Prepared(Result<Ready, String>),
    Cancelled,
    Thumb {
        id: String,
        result: Result<ColorImage, String>,
    },
    Full {
        id: String,
        result: Result<ColorImage, String>,
    },
    Anim {
        id: String,
        result: Result<Vec<(ColorImage, u32)>, String>,
    },
    Fetched {
        id: String,
        result: Result<PathBuf, String>,
    },
    Uploaded {
        job: u64,
        channel: String,
        text: String,
        result: Result<Vec<PostFile>, String>,
    },
    SavePicked {
        att: Attachment,
        to: Option<PathBuf>,
    },
}

pub enum ImgState {
    Requested,
    Decoding,
    Ready(TextureHandle),
    Failed(String),
}

pub struct Anim {
    pub frames: Vec<(TextureHandle, u32)>,
    pub total_ms: u32,
    pub started: Instant,
}

impl Anim {
    /// The frame to show now, and how long until the next one.
    pub fn current(&self) -> (&TextureHandle, Duration) {
        if self.frames.len() == 1 || self.total_ms == 0 {
            return (&self.frames[0].0, Duration::from_secs(3600));
        }
        let mut t = (self.started.elapsed().as_millis() as u64 % self.total_ms as u64) as u32;
        for (tex, d) in &self.frames {
            if t < *d {
                return (tex, Duration::from_millis((d - t) as u64));
            }
            t -= d;
        }
        (&self.frames[0].0, Duration::from_millis(20))
    }
}

/// A file being (or already) downloaded for playing, opening or saving.
pub enum Fetch {
    Downloading,
    Ready(PathBuf),
    Failed(String),
}

/// What to do once a download finishes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Then {
    Nothing,
    Play,
    Open,
}

pub struct Viewer {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub full: Option<TextureHandle>,
    pub waiting_for_bytes: bool,
}

/// Messages for the person (banner text, is it an error).
pub type Notes = Vec<(String, bool)>;

pub struct Attachments {
    pub pending: Vec<Pending>,
    pub preparing: usize,
    pub sending: Vec<Sending>,
    pub images: HashMap<String, ImgState>,
    pub anims: HashMap<String, Anim>,
    pub fetches: HashMap<String, Fetch>,
    /// Download progress by attachment id: (bytes so far, total).
    pub progress: Arc<Mutex<HashMap<String, (u64, u64)>>>,
    pub then: HashMap<String, Then>,
    pub viewer: Option<Viewer>,
    /// Does the server take images? (Very old servers don't.)
    pub server_supports: bool,
    /// Where to send and fetch files (servers from 0.7). None: images only, over the WebSocket.
    pub endpoint: Option<Endpoint>,
    pub max_bytes: u64,
    /// Upload finished: post this message (channel, text, files).
    pub ready_to_post: Vec<ClientMsg>,
    /// Images already asked for over the WebSocket after HTTP failed (so we
    /// don't ask twice).
    ws_fallback: std::collections::HashSet<String>,
    tx: Sender<Work>,
    rx: Receiver<Work>,
    wake: Wake,
    next_job: u64,
    thumb_order: VecDeque<String>,
    thumb_target: HashMap<String, [u32; 2]>,
    anim_order: VecDeque<String>,
    raw: VecDeque<(String, Arc<Vec<u8>>)>,
    raw_total: usize,
}

/// Size an image or video is shown at, keeping its shape.
pub fn display_size(width: u32, height: u32, max: Vec2) -> Vec2 {
    if width == 0 || height == 0 {
        let fallback = Vec2::new(360.0, 202.0);
        return Vec2::new(fallback.x.min(max.x), fallback.y.min(max.y));
    }
    let (w, h) = (width as f32, height as f32);
    let scale = (max.x / w).min(max.y / h).min(1.0);
    Vec2::new((w * scale).max(24.0), (h * scale).max(24.0))
}

impl Attachments {
    pub fn new(wake: Wake) -> Self {
        let (tx, rx) = channel();
        Self {
            pending: Vec::new(),
            preparing: 0,
            sending: Vec::new(),
            images: HashMap::new(),
            anims: HashMap::new(),
            fetches: HashMap::new(),
            progress: Arc::new(Mutex::new(HashMap::new())),
            then: HashMap::new(),
            viewer: None,
            server_supports: false,
            endpoint: None,
            max_bytes: 8 * 1024 * 1024,
            ready_to_post: Vec::new(),
            ws_fallback: std::collections::HashSet::new(),
            tx,
            rx,
            wake,
            next_job: 0,
            thumb_order: VecDeque::new(),
            thumb_target: HashMap::new(),
            anim_order: VecDeque::new(),
            raw: VecDeque::new(),
            raw_total: 0,
        }
    }

    fn spawn(&self, job: impl FnOnce() -> Work + Send + 'static) {
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        let _ = std::thread::Builder::new()
            .name("files".into())
            .spawn(move || {
                let _ = tx.send(job());
                wake();
            });
    }

    /// Any file type (servers from 0.7), or just images.
    pub fn any_files(&self) -> bool {
        self.endpoint.is_some()
    }

    /// Signed in somewhere else (or again): forget what belonged to the last server.
    pub fn reset_for_server(&mut self) {
        self.images.clear();
        self.anims.clear();
        self.anim_order.clear();
        self.thumb_order.clear();
        self.fetches.retain(|_, f| matches!(f, Fetch::Ready(_)));
        self.then.clear();
    }

    fn room_for_more(&self) -> bool {
        self.pending.len() + self.preparing < MAX_PENDING
    }

    // ------------------------------------------------------------ adding files to send

    /// Opens the file picker (Windows).
    pub fn pick(&mut self) -> Result<(), String> {
        if !images::has_file_picker() {
            return Err("Drag a file onto the window, or paste an image with Ctrl+V.".into());
        }
        if !self.room_for_more() {
            return Err(format!("You can send up to {MAX_PENDING} files at a time."));
        }
        self.preparing += 1;
        let (max, any) = (self.max_bytes, self.any_files());
        self.spawn(move || match images::pick_file(any) {
            Some(path) => Work::Prepared(prepare_path(path, max, any)),
            None => Work::Cancelled,
        });
        Ok(())
    }

    pub fn add_file(&mut self, path: PathBuf) -> Result<(), String> {
        if !self.room_for_more() {
            return Err(format!("You can send up to {MAX_PENDING} files at a time."));
        }
        self.preparing += 1;
        let (max, any) = (self.max_bytes, self.any_files());
        self.spawn(move || Work::Prepared(prepare_path(path, max, any)));
        Ok(())
    }

    /// Ctrl+V: if the clipboard holds an image, attach it. Text pastes are left alone.
    pub fn paste(&mut self) {
        if !self.room_for_more() {
            return;
        }
        self.preparing += 1;
        let max = self.max_bytes;
        self.spawn(move || match images::clipboard_image() {
            Some(Ok(png)) => {
                Work::Prepared(images::prepare(png, "pasted image.png", max).map(from_prepared))
            }
            Some(Err(e)) => Work::Prepared(Err(e)),
            None => Work::Cancelled,
        });
    }

    pub fn remove_pending(&mut self, i: usize) {
        if i < self.pending.len() {
            self.pending.remove(i);
        }
    }

    // ------------------------------------------------------------ sending

    /// Send what's waiting, with `text`. Returns false if nothing was sent.
    pub fn send(&mut self, net: &Net, channel: &str, text: &str) -> bool {
        let pending = std::mem::take(&mut self.pending);
        if pending.is_empty() {
            return false;
        }
        let Some(to) = self.endpoint.clone() else {
            // An older server: images over the WebSocket, one message each.
            for (i, p) in pending.into_iter().enumerate() {
                let Source::Bytes(bytes) = p.source else {
                    continue;
                };
                let header = proto::UploadHeader {
                    channel: channel.to_string(),
                    text: if i == 0 {
                        text.to_string()
                    } else {
                        String::new()
                    },
                    name: p.name,
                    mime: images::mime_for_name(&bytes).to_string(),
                    width: p.width,
                    height: p.height,
                };
                net.upload(proto::encode_upload(&header, &bytes));
            }
            return true;
        };
        self.next_job += 1;
        let job = self.next_job;
        let total: u64 = pending
            .iter()
            .map(|p| p.size + p.poster.as_ref().map_or(0, |b| b.len() as u64))
            .sum();
        let label = if pending.len() == 1 {
            pending[0].name.clone()
        } else {
            format!("{} files", pending.len())
        };
        let done = Arc::new(AtomicU64::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        self.sending.push(Sending {
            id: job,
            label,
            done: done.clone(),
            total,
            cancel: cancel.clone(),
        });
        let (channel, text) = (channel.to_string(), text.to_string());
        let wake = self.wake.clone();
        self.spawn(move || {
            let mut sent_before = 0u64;
            let last_wake = std::cell::Cell::new(Instant::now());
            let mut posted = Vec::new();
            let result = (|| {
                for p in pending {
                    let before = sent_before;
                    let report = |n: u64| {
                        done.store(before + n, Ordering::Relaxed);
                        if last_wake.get().elapsed() > Duration::from_millis(150) {
                            last_wake.set(Instant::now());
                            wake();
                        }
                    };
                    let poster = match &p.poster {
                        Some(jpg) => {
                            let src = Source::Bytes(jpg.clone());
                            let id = xfer::upload(&to, "poster.jpg", &src, &|_| {}, &cancel)?;
                            sent_before += jpg.len() as u64;
                            Some(id)
                        }
                        None => None,
                    };
                    let upload = xfer::upload(&to, &p.name, &p.source, &report, &cancel)?;
                    sent_before += p.size;
                    posted.push(PostFile {
                        upload,
                        width: p.width,
                        height: p.height,
                        duration_ms: p.duration_ms,
                        poster,
                    });
                }
                Ok(std::mem::take(&mut posted))
            })();
            Work::Uploaded {
                job,
                channel,
                text,
                result,
            }
        });
        true
    }

    pub fn cancel_sending(&mut self, job: u64) {
        if let Some(s) = self.sending.iter().find(|s| s.id == job) {
            s.cancel.store(true, Ordering::Relaxed);
        }
    }

    // ------------------------------------------------------------ received images and posters

    /// Ask for an image (or a video's poster) that just scrolled into view.
    /// `id` is the image's own id; `size` the size it's shown at.
    pub fn request_image(&mut self, id: &str, name: &str, size: Vec2, ppp: f32, net: &Net) {
        if self.images.contains_key(id) {
            return;
        }
        let px = size * ppp.clamp(1.0, 2.0);
        let target = [px.x.ceil() as u32, px.y.ceil() as u32];
        self.thumb_target.insert(id.to_string(), target);
        if let Some(to) = self.endpoint.clone() {
            self.images.insert(id.to_string(), ImgState::Decoding);
            let (id, name) = (id.to_string(), name.to_string());
            self.spawn(move || {
                let path = xfer::cached_path(&id, &name);
                let never = AtomicBool::new(false);
                let result = xfer::download(&to, &id, &path, &|_, _| {}, &never)
                    .and_then(|_| std::fs::read(&path).map_err(|e| e.to_string()))
                    .and_then(|b| images::decode_fit(&b, target[0], target[1]));
                Work::Thumb { id, result }
            });
        } else if let Some(bytes) = self.raw_bytes(id) {
            // Still have the bytes (the texture was dropped to save memory): just decode again.
            self.images.insert(id.to_string(), ImgState::Decoding);
            self.decode_thumb(id.to_string(), bytes);
        } else {
            self.images.insert(id.to_string(), ImgState::Requested);
            net.send(ClientMsg::GetAttachment { id: id.to_string() });
        }
    }

    /// Start animating a GIF that just scrolled into view.
    pub fn request_gif(&mut self, a: &Attachment, size: Vec2, ppp: f32, net: &Net) {
        let Some(to) = self.endpoint.clone() else {
            // Older servers: shown like any image (first frame).
            return self.request_image(&a.id, &a.name, size, ppp, net);
        };
        if self.images.contains_key(&a.id) {
            return;
        }
        self.images.insert(a.id.clone(), ImgState::Decoding);
        let px = size * ppp.clamp(1.0, 2.0);
        let (w, h) = (px.x.ceil() as u32, px.y.ceil() as u32);
        let (id, name) = (a.id.clone(), a.name.clone());
        self.spawn(move || {
            let path = xfer::cached_path(&id, &name);
            let never = AtomicBool::new(false);
            let result = xfer::download(&to, &id, &path, &|_, _| {}, &never)
                .and_then(|_| images::decode_gif(&path, w, h, ANIM_BUDGET));
            Work::Anim { id, result }
        });
    }

    fn decode_thumb(&self, id: String, bytes: Arc<Vec<u8>>) {
        let [w, h] = self.thumb_target.get(&id).copied().unwrap_or([840, 640]);
        self.spawn(move || Work::Thumb {
            result: images::decode_fit(&bytes, w, h),
            id,
        });
    }

    /// Image bytes arrived over the WebSocket (older servers).
    pub fn on_data(&mut self, id: String, bytes: Vec<u8>) {
        let bytes = Arc::new(bytes);
        self.remember_raw(id.clone(), bytes.clone());
        if matches!(self.images.get(&id), Some(ImgState::Requested) | None) {
            self.images.insert(id.clone(), ImgState::Decoding);
            self.decode_thumb(id.clone(), bytes.clone());
        }
        if let Some(v) = &mut self.viewer {
            if v.id == id && v.waiting_for_bytes {
                v.waiting_for_bytes = false;
                self.spawn(move || Work::Full {
                    result: images::decode_fit(&bytes, 2560, 2560),
                    id,
                });
            }
        }
    }

    pub fn gone(&mut self, id: &str) {
        self.images.insert(
            id.to_string(),
            ImgState::Failed("This file is no longer available.".into()),
        );
        self.anims.remove(id);
        self.fetches.insert(
            id.to_string(),
            Fetch::Failed("This file is no longer available.".into()),
        );
        if self.viewer.as_ref().is_some_and(|v| v.id == id) {
            self.viewer = None;
        }
    }

    fn remember_raw(&mut self, id: String, bytes: Arc<Vec<u8>>) {
        self.raw.retain(|(i, _)| *i != id);
        self.raw_total = self.raw.iter().map(|(_, b)| b.len()).sum();
        self.raw_total += bytes.len();
        self.raw.push_back((id, bytes));
        while self.raw_total > RAW_BUDGET && self.raw.len() > 1 {
            if let Some((_, b)) = self.raw.pop_front() {
                self.raw_total -= b.len();
            }
        }
    }

    pub fn raw_bytes(&self, id: &str) -> Option<Arc<Vec<u8>>> {
        self.raw
            .iter()
            .find(|(i, _)| i == id)
            .map(|(_, b)| b.clone())
    }

    // ------------------------------------------------------------ downloads

    /// Download a file to the cache (if it isn't there yet), then do `then`.
    pub fn fetch(&mut self, a: &Attachment, then: Then) {
        let cached = xfer::cached_path(&a.id, &a.name);
        if cached.exists() {
            self.fetches.insert(a.id.clone(), Fetch::Ready(cached));
            self.then.insert(a.id.clone(), then);
            return;
        }
        self.then.insert(a.id.clone(), then);
        if matches!(self.fetches.get(&a.id), Some(Fetch::Downloading)) {
            return;
        }
        let Some(to) = self.endpoint.clone() else {
            self.fetches.insert(
                a.id.clone(),
                Fetch::Failed("The server needs updating first.".into()),
            );
            return;
        };
        self.fetches.insert(a.id.clone(), Fetch::Downloading);
        self.progress.lock().insert(a.id.clone(), (0, a.size));
        let progress = self.progress.clone();
        let wake = self.wake.clone();
        let id = a.id.clone();
        self.spawn(move || {
            let last = std::cell::Cell::new(Instant::now());
            let report = |done: u64, total: u64| {
                progress.lock().insert(id.clone(), (done, total));
                if last.get().elapsed() > Duration::from_millis(120) {
                    last.set(Instant::now());
                    wake();
                }
            };
            let never = AtomicBool::new(false);
            let result = xfer::download(&to, &id, &cached, &report, &never).map(|_| cached);
            progress.lock().remove(&id);
            Work::Fetched { id, result }
        });
    }

    /// The finished download, if there is one.
    pub fn fetched(&self, id: &str) -> Option<&PathBuf> {
        match self.fetches.get(id) {
            Some(Fetch::Ready(p)) => Some(p),
            _ => None,
        }
    }

    /// Ask where to save a file (Windows dialog), then download and copy it there.
    pub fn save_as(&mut self, a: &Attachment) {
        let att = a.clone();
        self.spawn(move || {
            let to = images::save_file_dialog(&att.name);
            Work::SavePicked { att, to }
        });
    }

    // ------------------------------------------------------------ viewer

    pub fn open_viewer(&mut self, a: &Attachment, net: &Net) {
        self.viewer = Some(Viewer {
            id: a.id.clone(),
            name: a.name.clone(),
            size: a.size,
            full: None,
            waiting_for_bytes: false,
        });
        if let Some(to) = self.endpoint.clone() {
            let (id, name) = (a.id.clone(), a.name.clone());
            self.spawn(move || {
                let path = xfer::cached_path(&id, &name);
                let never = AtomicBool::new(false);
                let result = xfer::download(&to, &id, &path, &|_, _| {}, &never)
                    .and_then(|_| std::fs::read(&path).map_err(|e| e.to_string()))
                    .and_then(|b| images::decode_fit(&b, 2560, 2560));
                Work::Full { id, result }
            });
            return;
        }
        match self.raw_bytes(&a.id) {
            Some(bytes) => {
                let id = a.id.clone();
                self.spawn(move || Work::Full {
                    result: images::decode_fit(&bytes, 2560, 2560),
                    id,
                });
            }
            None => {
                if let Some(v) = &mut self.viewer {
                    v.waiting_for_bytes = true;
                }
                net.send(ClientMsg::GetAttachment { id: a.id.clone() });
            }
        }
    }

    pub fn close_viewer(&mut self) {
        // Dropping the texture frees its memory.
        self.viewer = None;
    }

    // ------------------------------------------------------------ results from workers

    /// Collect finished work. Returns (message, is error) notes to show, and
    /// downloads that finished with something to do next.
    pub fn poll(&mut self, ctx: &egui::Context) -> (Notes, Vec<(String, Then, PathBuf)>) {
        let mut notes = Vec::new();
        let mut finished = Vec::new();
        while let Ok(work) = self.rx.try_recv() {
            match work {
                Work::Prepared(result) => {
                    self.preparing = self.preparing.saturating_sub(1);
                    match result {
                        Ok(r) => {
                            let preview = r.preview.map(|img| {
                                ctx.load_texture(
                                    format!("pending-{}-{}", self.pending.len(), r.name),
                                    img,
                                    TextureOptions::LINEAR,
                                )
                            });
                            self.pending.push(Pending {
                                name: r.name,
                                kind: r.kind,
                                source: r.source,
                                size: r.size,
                                width: r.width,
                                height: r.height,
                                duration_ms: r.duration_ms,
                                poster: r.poster,
                                preview,
                            });
                        }
                        Err(e) => notes.push((e, true)),
                    }
                }
                Work::Cancelled => self.preparing = self.preparing.saturating_sub(1),
                Work::Thumb { id, result } => match result {
                    Ok(img) => {
                        let tex =
                            ctx.load_texture(format!("img-{id}"), img, TextureOptions::LINEAR);
                        self.images.insert(id.clone(), ImgState::Ready(tex));
                        self.thumb_order.push_back(id);
                        while self.thumb_order.len() > MAX_THUMBS {
                            if let Some(old) = self.thumb_order.pop_front() {
                                // Forget it entirely; it's asked for again if it scrolls back into view.
                                self.images.remove(&old);
                            }
                        }
                    }
                    Err(e) => {
                        if !self.fall_back_to_websocket(&id, &e) {
                            tuffcord::applog::warn(format!("Couldn't show image {id}: {e}"));
                            self.images
                                .insert(id, ImgState::Failed("Couldn't show this image.".into()));
                        }
                    }
                },
                Work::Anim { id, result } => match result {
                    Ok(frames) if !frames.is_empty() => {
                        let total_ms = frames.iter().map(|f| f.1).sum();
                        let frames = frames
                            .into_iter()
                            .enumerate()
                            .map(|(i, (img, d))| {
                                (
                                    ctx.load_texture(
                                        format!("gif-{id}-{i}"),
                                        img,
                                        TextureOptions::LINEAR,
                                    ),
                                    d,
                                )
                            })
                            .collect();
                        self.anims.insert(
                            id.clone(),
                            Anim {
                                frames,
                                total_ms,
                                started: Instant::now(),
                            },
                        );
                        // Mark as shown so it isn't asked for again.
                        self.images.insert(id.clone(), ImgState::Requested);
                        self.anim_order.push_back(id);
                        while self.anim_order.len() > MAX_ANIMS {
                            if let Some(old) = self.anim_order.pop_front() {
                                self.anims.remove(&old);
                                self.images.remove(&old);
                            }
                        }
                    }
                    Ok(_) => {
                        self.images
                            .insert(id, ImgState::Failed("Couldn't show this GIF.".into()));
                    }
                    Err(e) => {
                        if !self.fall_back_to_websocket(&id, &e) {
                            tuffcord::applog::warn(format!("Couldn't show GIF {id}: {e}"));
                            self.images
                                .insert(id, ImgState::Failed("Couldn't show this GIF.".into()));
                        }
                    }
                },
                Work::Full { id, result } => {
                    if let Some(v) = &mut self.viewer {
                        if v.id == id {
                            match result {
                                Ok(img) => {
                                    v.full = Some(ctx.load_texture(
                                        format!("full-{id}"),
                                        img,
                                        TextureOptions::LINEAR,
                                    ))
                                }
                                Err(e) => {
                                    notes.push((format!("Couldn't open that image ({e})."), true))
                                }
                            }
                        }
                    }
                }
                Work::Fetched { id, result } => match result {
                    Ok(path) => {
                        let then = self.then.remove(&id).unwrap_or(Then::Nothing);
                        self.fetches.insert(id.clone(), Fetch::Ready(path.clone()));
                        finished.push((id, then, path));
                    }
                    Err(e) => {
                        self.then.remove(&id);
                        notes.push((format!("Couldn't download that file: {e}"), true));
                        self.fetches.insert(id, Fetch::Failed(e));
                    }
                },
                Work::Uploaded {
                    job,
                    channel,
                    text,
                    result,
                } => {
                    self.sending.retain(|s| s.id != job);
                    match result {
                        Ok(files) => self.ready_to_post.push(ClientMsg::Post {
                            channel,
                            text,
                            files,
                        }),
                        Err(e) if e == "Cancelled." => {
                            notes.push(("Sending cancelled.".into(), false))
                        }
                        Err(e) => notes.push((format!("Couldn't send: {e}"), true)),
                    }
                }
                Work::SavePicked { att, to } => {
                    let Some(to) = to else { continue };
                    let Some(endpoint) = self.endpoint.clone() else {
                        continue;
                    };
                    let wake = self.wake.clone();
                    let tx = self.tx.clone();
                    let _ = std::thread::Builder::new()
                        .name("save".into())
                        .spawn(move || {
                            let cached = xfer::cached_path(&att.id, &att.name);
                            let never = AtomicBool::new(false);
                            let r = xfer::download(&endpoint, &att.id, &cached, &|_, _| {}, &never)
                                .and_then(|_| xfer::save_copy(&cached, &to));
                            let _ = tx.send(Work::Fetched {
                                id: format!("saved:{}", to.display()),
                                result: r.map(|_| to.clone()),
                            });
                            wake();
                        });
                }
            }
        }
        // "saved:" results are just for a note.
        finished.retain(|(id, _, path)| {
            if id.starts_with("saved:") {
                notes.push((format!("Saved to {}", path.display()), false));
                self.fetches.remove(id);
                false
            } else {
                true
            }
        });
        (notes, finished)
    }
}

impl Attachments {
    /// Fetching an image over HTTP failed: ask for it the older way, over the
    /// chat connection (servers send images up to 16 MB that way). Returns
    /// false if that was already tried.
    fn fall_back_to_websocket(&mut self, id: &str, error: &str) -> bool {
        if self.endpoint.is_none() || !self.ws_fallback.insert(id.to_string()) {
            return false;
        }
        tuffcord::applog::warn(format!(
            "Fetching image {id} over HTTP failed ({error}); asking over the chat connection instead"
        ));
        self.images.insert(id.to_string(), ImgState::Requested);
        self.ready_to_post
            .push(ClientMsg::GetAttachment { id: id.to_string() });
        true
    }
}

fn from_prepared(p: Prepared) -> Ready {
    Ready {
        name: p.name,
        kind: kind_of(p.mime),
        size: p.bytes.len() as u64,
        source: Source::Bytes(p.bytes),
        width: p.width,
        height: p.height,
        duration_ms: 0,
        poster: None,
        preview: Some(p.preview),
    }
}

/// Work out what a file is and get it ready to send (on a worker thread).
fn prepare_path(path: PathBuf, max: u64, any_files: bool) -> Result<Ready, String> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".into());
    let meta = std::fs::metadata(&path).map_err(|e| format!("Couldn't open {name} ({e})."))?;
    if meta.is_dir() {
        return Err(format!("{name} is a folder. Zip it first to send it."));
    }
    let size = meta.len();
    if size == 0 {
        return Err(format!("{name} is empty."));
    }
    let mut head = [0u8; 64];
    let n = std::fs::File::open(&path)
        .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
        .map_err(|e| format!("Couldn't open {name} ({e})."))?;
    let mime = proto::files::sniff(&head[..n]);
    let image_like = proto::IMAGE_MIMES.contains(&mime)
        || image::guess_format(&head[..n]).is_ok_and(|f| f == image::ImageFormat::Bmp);
    if image_like {
        // Photos are shrunk before sending, so read them even if large.
        if size > max.max(60 * 1024 * 1024) {
            return Err(too_big(&name, max));
        }
        return images::prepare_file(&path, max).map(from_prepared);
    }
    if !any_files {
        return Err(
            "This server only takes images. It needs updating before other files can be sent."
                .into(),
        );
    }
    if size > max {
        return Err(too_big(&name, max));
    }
    let kind = kind_of(mime);
    let mut ready = Ready {
        name: proto::files::clean_name(&name),
        kind,
        source: Source::Path(path.clone()),
        size,
        width: 0,
        height: 0,
        duration_ms: 0,
        poster: None,
        preview: None,
    };
    if matches!(kind, Kind::Video | Kind::Audio) {
        if let Ok(p) = media::probe(&path) {
            ready.width = p.width;
            ready.height = p.height;
            ready.duration_ms = p.duration_ms;
            if let Some(img) = p.poster {
                ready.poster = media::poster_jpeg(&img);
                let small = image::DynamicImage::ImageRgba8(img).thumbnail(160, 160);
                ready.preview = Some(images::to_color_image(&small));
            }
        }
    }
    Ok(ready)
}

fn too_big(name: &str, max: u64) -> String {
    format!(
        "{name} is too big to send. The limit is {}.",
        proto::files::size_label(max)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_sizes_keep_shape_and_never_upscale() {
        assert_eq!(display_size(2000, 1000, THUMB_MAX), Vec2::new(420.0, 210.0));
        assert_eq!(display_size(1000, 2000, THUMB_MAX), Vec2::new(160.0, 320.0));
        assert_eq!(display_size(100, 50, THUMB_MAX), Vec2::new(100.0, 50.0));
        assert_eq!(display_size(0, 0, THUMB_MAX), Vec2::new(360.0, 202.0));
    }

    #[test]
    fn prepares_files_by_what_they_are() {
        let dir = std::env::temp_dir().join(format!("br-prep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("notes.pdf");
        std::fs::write(&pdf, b"%PDF-1.7 hello").unwrap();
        let r = prepare_path(pdf.clone(), 1 << 20, true).unwrap();
        assert_eq!(
            (r.kind, r.size, r.name.as_str()),
            (Kind::File, 14, "notes.pdf")
        );
        assert!(
            prepare_path(pdf.clone(), 1 << 20, false).is_err(),
            "older servers take images only"
        );
        assert!(prepare_path(pdf, 5, true)
            .err()
            .unwrap()
            .contains("too big"));
        let mp4 = dir.join("clip.mp4");
        std::fs::write(&mp4, b"\0\0\0\x20ftypisom\0\0\x02\0").unwrap();
        assert_eq!(prepare_path(mp4, 1 << 20, true).unwrap().kind, Kind::Video);
        let empty = dir.join("empty.txt");
        std::fs::write(&empty, b"").unwrap();
        assert!(prepare_path(empty, 1 << 20, true).is_err());
        assert!(prepare_path(dir.clone(), 1 << 20, true)
            .err()
            .unwrap()
            .contains("folder"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
