//! Image attachments in the app: images waiting to be sent, images received
//! (fetched only when they scroll into view, decoded at display size), and the
//! full-size viewer.
//!
//! Decoding and file reading happen on short-lived worker threads; results come
//! back over a channel and are turned into textures on the UI thread.

use backroom::images::{self, Prepared};
use backroom::net::{Net, Wake};
use eframe::egui::{self, ColorImage, TextureHandle, TextureOptions, Vec2};
use proto::{Attachment, ClientMsg};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Largest size an image is shown at in the chat.
pub const THUMB_MAX: Vec2 = Vec2::new(420.0, 320.0);
/// Up to this many images can wait to be sent at once.
pub const MAX_PENDING: usize = 4;
/// Downloaded image bytes kept around for the viewer (oldest dropped first).
const RAW_BUDGET: usize = 48 * 1024 * 1024;
/// Chat thumbnails kept as textures (oldest dropped first, re-decoded if needed).
const MAX_THUMBS: usize = 150;

pub struct Pending {
    pub name: String,
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub preview: TextureHandle,
}

enum Work {
    Prepared(Result<Prepared, String>),
    Cancelled,
    Thumb {
        id: String,
        result: Result<ColorImage, String>,
    },
    Full {
        id: String,
        result: Result<ColorImage, String>,
    },
}

pub enum ImgState {
    Requested,
    Decoding,
    Ready(TextureHandle),
    Failed(String),
}

pub struct Viewer {
    pub id: String,
    pub name: String,
    pub size: u64,
    pub full: Option<TextureHandle>,
    pub waiting_for_bytes: bool,
}

pub struct Attachments {
    pub pending: Vec<Pending>,
    pub preparing: usize,
    pub images: HashMap<String, ImgState>,
    pub viewer: Option<Viewer>,
    /// Does the server accept images? (Older servers don't.)
    pub server_supports: bool,
    pub max_bytes: u64,
    tx: Sender<Work>,
    rx: Receiver<Work>,
    wake: Wake,
    thumb_order: VecDeque<String>,
    thumb_target: HashMap<String, [u32; 2]>,
    raw: VecDeque<(String, Arc<Vec<u8>>)>,
    raw_total: usize,
}

/// Size an image is shown at, keeping its shape.
pub fn display_size(a: &Attachment, max: Vec2) -> Vec2 {
    if a.width == 0 || a.height == 0 {
        return Vec2::new(240.0_f32.min(max.x), 180.0_f32.min(max.y));
    }
    let (w, h) = (a.width as f32, a.height as f32);
    let scale = (max.x / w).min(max.y / h).min(1.0);
    Vec2::new((w * scale).max(24.0), (h * scale).max(24.0))
}

impl Attachments {
    pub fn new(wake: Wake) -> Self {
        let (tx, rx) = channel();
        Self {
            pending: Vec::new(),
            preparing: 0,
            images: HashMap::new(),
            viewer: None,
            server_supports: false,
            max_bytes: 8 * 1024 * 1024,
            tx,
            rx,
            wake,
            thumb_order: VecDeque::new(),
            thumb_target: HashMap::new(),
            raw: VecDeque::new(),
            raw_total: 0,
        }
    }

    fn spawn(&self, job: impl FnOnce() -> Work + Send + 'static) {
        let tx = self.tx.clone();
        let wake = self.wake.clone();
        let _ = std::thread::Builder::new()
            .name("images".into())
            .spawn(move || {
                let _ = tx.send(job());
                wake();
            });
    }

    fn room_for_more(&self) -> bool {
        self.pending.len() + self.preparing < MAX_PENDING
    }

    // ------------------------------------------------------------ adding images to send

    /// Opens the file picker (Windows). Returns false when there isn't one.
    pub fn pick(&mut self) -> Result<(), String> {
        if !images::has_file_picker() {
            return Err("Drag an image onto the window, or paste one with Ctrl+V.".into());
        }
        if !self.room_for_more() {
            return Err(format!(
                "You can send up to {MAX_PENDING} images at a time."
            ));
        }
        self.preparing += 1;
        let max = self.max_bytes;
        self.spawn(move || match images::pick_image_file() {
            Some(path) => Work::Prepared(images::prepare_file(&path, max)),
            None => Work::Cancelled,
        });
        Ok(())
    }

    pub fn add_file(&mut self, path: PathBuf) -> Result<(), String> {
        if !self.room_for_more() {
            return Err(format!(
                "You can send up to {MAX_PENDING} images at a time."
            ));
        }
        self.preparing += 1;
        let max = self.max_bytes;
        self.spawn(move || Work::Prepared(images::prepare_file(&path, max)));
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
            Some(Ok(png)) => Work::Prepared(images::prepare(png, "pasted image.png", max)),
            Some(Err(e)) => Work::Prepared(Err(e)),
            None => Work::Cancelled,
        });
    }

    pub fn remove_pending(&mut self, i: usize) {
        if i < self.pending.len() {
            self.pending.remove(i);
        }
    }

    pub fn take_pending(&mut self) -> Vec<Pending> {
        std::mem::take(&mut self.pending)
    }

    // ------------------------------------------------------------ received images

    /// Ask the server for an image that just scrolled into view.
    pub fn request(&mut self, a: &Attachment, ppp: f32, net: &Net) {
        if self.images.contains_key(&a.id) {
            return;
        }
        let size = display_size(a, THUMB_MAX) * ppp.clamp(1.0, 2.0);
        self.thumb_target
            .insert(a.id.clone(), [size.x.ceil() as u32, size.y.ceil() as u32]);
        if let Some(bytes) = self.raw_bytes(&a.id) {
            // Still have the bytes (the texture was dropped to save memory): just decode again.
            self.images.insert(a.id.clone(), ImgState::Decoding);
            self.decode_thumb(a.id.clone(), bytes);
        } else {
            self.images.insert(a.id.clone(), ImgState::Requested);
            net.send(ClientMsg::GetAttachment { id: a.id.clone() });
        }
    }

    fn decode_thumb(&self, id: String, bytes: Arc<Vec<u8>>) {
        let [w, h] = self.thumb_target.get(&id).copied().unwrap_or([840, 640]);
        self.spawn(move || Work::Thumb {
            result: images::decode_fit(&bytes, w, h),
            id,
        });
    }

    fn decode_full(&self, id: String, bytes: Arc<Vec<u8>>) {
        self.spawn(move || Work::Full {
            result: images::decode_fit(&bytes, 2560, 2560),
            id,
        });
    }

    /// Bytes arrived from the server.
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
                self.decode_full(id, bytes);
            }
        }
    }

    pub fn gone(&mut self, id: &str) {
        self.images.insert(
            id.to_string(),
            ImgState::Failed("This image is no longer available.".into()),
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

    // ------------------------------------------------------------ viewer

    pub fn open_viewer(&mut self, a: &Attachment, net: &Net) {
        self.viewer = Some(Viewer {
            id: a.id.clone(),
            name: a.name.clone(),
            size: a.size,
            full: None,
            waiting_for_bytes: false,
        });
        match self.raw_bytes(&a.id) {
            Some(bytes) => self.decode_full(a.id.clone(), bytes),
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

    /// Collect finished work. Returns error messages to show.
    pub fn poll(&mut self, ctx: &egui::Context) -> Vec<String> {
        let mut errors = Vec::new();
        while let Ok(work) = self.rx.try_recv() {
            match work {
                Work::Prepared(result) => {
                    self.preparing = self.preparing.saturating_sub(1);
                    match result {
                        Ok(p) => {
                            let preview = ctx.load_texture(
                                format!("pending-{}", self.pending.len()),
                                p.preview,
                                TextureOptions::LINEAR,
                            );
                            self.pending.push(Pending {
                                name: p.name,
                                mime: p.mime,
                                bytes: p.bytes,
                                width: p.width,
                                height: p.height,
                                preview,
                            });
                        }
                        Err(e) => errors.push(e),
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
                                // Forget it entirely; it's re-requested if it scrolls back into view.
                                self.images.remove(&old);
                            }
                        }
                    }
                    Err(_) => {
                        self.images
                            .insert(id, ImgState::Failed("Couldn't show this image.".into()));
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
                                Err(e) => errors.push(format!("Couldn't open that image ({e}).")),
                            }
                        }
                    }
                }
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(w: u32, h: u32) -> Attachment {
        Attachment {
            id: "a".into(),
            name: "x.png".into(),
            mime: "image/png".into(),
            size: 1,
            width: w,
            height: h,
        }
    }

    #[test]
    fn display_sizes_keep_shape_and_never_upscale() {
        assert_eq!(
            display_size(&att(2000, 1000), THUMB_MAX),
            Vec2::new(420.0, 210.0)
        );
        assert_eq!(
            display_size(&att(1000, 2000), THUMB_MAX),
            Vec2::new(160.0, 320.0)
        );
        assert_eq!(
            display_size(&att(100, 50), THUMB_MAX),
            Vec2::new(100.0, 50.0)
        );
        assert_eq!(display_size(&att(0, 0), THUMB_MAX), Vec2::new(240.0, 180.0));
    }
}
