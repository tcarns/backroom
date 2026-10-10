//! Video preview frames: the poster the sender attached, and when there's
//! none (older uploads) or it came out all black, one taken from the clip on
//! this PC once it's downloaded, kept in the cache as a small JPEG.

use crate::attach::{Attachments, ImgState, Work};
use crate::attach_ui::Act;
use crate::App;
use eframe::egui::{self, ColorImage, TextureHandle, Vec2};
use proto::Attachment;
use tuffcord::files as xfer;
use tuffcord::{images, media};

/// What `images` knows a clip's locally taken frame by.
fn local_key(id: &str) -> String {
    format!("frame:{id}")
}

/// Where a clip's locally taken frame is kept.
fn local_jpeg(id: &str) -> std::path::PathBuf {
    xfer::cached_path(id, "poster.jpg")
}

const NOT_DOWNLOADED: &str = "The clip isn't downloaded yet.";

impl App {
    /// The picture to show on a video that isn't playing, asking for it when
    /// it's on screen and not loaded yet.
    pub(crate) fn video_poster<'a>(
        &'a self,
        a: &Attachment,
        size: Vec2,
        visible: bool,
        acts: &mut Vec<Act>,
    ) -> Option<&'a TextureHandle> {
        let att = &self.att;
        if let Some(p) = a
            .poster
            .as_ref()
            .filter(|p| !att.dark_posters.contains(&p.id))
        {
            match att.images.get(&p.id) {
                Some(ImgState::Ready(t)) => return Some(t),
                None => {
                    if visible {
                        acts.push(Act::Poster(p.id.clone(), size));
                    }
                    return None;
                }
                // Couldn't be fetched: try the clip's own frame below.
                Some(ImgState::Failed(_)) => {}
                _ => return None,
            }
        }
        match att.images.get(&local_key(&a.id)) {
            Some(ImgState::Ready(t)) => Some(t),
            None if visible && media::supported() && !a.expired => {
                acts.push(Act::LocalPoster(a.clone(), size));
                None
            }
            _ => None,
        }
    }
}

impl Attachments {
    /// Remember that `id` is a video's poster, so it's checked for being all
    /// black once decoded.
    pub(crate) fn request_poster(&mut self, id: &str) {
        self.posters.insert(id.to_string());
    }

    /// Take a clip's preview frame on this PC: from the cache if it was taken
    /// before, else from the downloaded clip (once). Clips not downloaded yet
    /// are tried again when their download finishes.
    pub(crate) fn request_local_poster(&mut self, a: &Attachment, size: Vec2, ppp: f32) {
        let key = local_key(&a.id);
        // One at a time, so a screenful of old clips doesn't decode all at once.
        if self.local_poster_busy || self.images.contains_key(&key) {
            return;
        }
        self.local_poster_busy = true;
        self.images.insert(key, ImgState::Decoding);
        let px = size * ppp.clamp(1.0, 2.0);
        let (w, h) = (px.x.ceil() as u32, px.y.ceil() as u32);
        let (id, name) = (a.id.clone(), a.name.clone());
        self.spawn(move || {
            let jpeg = local_jpeg(&id);
            let clip = xfer::cached_path(&id, &name);
            let result = if let Ok(bytes) = std::fs::read(&jpeg) {
                images::decode_fit(&bytes, w, h)
            } else if clip.exists() {
                media::probe(&clip).and_then(|p| {
                    let img = p.poster.ok_or("No frame in this clip.")?;
                    if let Some(bytes) = media::poster_jpeg(&img) {
                        let _ = std::fs::write(&jpeg, bytes);
                    }
                    let small = image::DynamicImage::ImageRgba8(img).thumbnail(w, h);
                    Ok(images::to_color_image(&small))
                })
            } else {
                Err(NOT_DOWNLOADED.into())
            };
            Work::LocalPoster { id, result }
        });
    }

    pub(crate) fn local_poster_done(
        &mut self,
        ctx: &egui::Context,
        id: String,
        result: Result<ColorImage, String>,
    ) {
        self.local_poster_busy = false;
        match result {
            Ok(img) => self.keep_texture(ctx, local_key(&id), img),
            Err(e) => {
                if e != NOT_DOWNLOADED {
                    tuffcord::applog::warn(format!("No preview frame for clip {id}: {e}"));
                }
                self.images.insert(local_key(&id), ImgState::Failed(e));
            }
        }
    }

    /// A clip finished downloading: if it had no frame to show for want of
    /// the file, try again.
    pub(crate) fn retry_local_poster(&mut self, id: &str) {
        let key = local_key(id);
        if matches!(self.images.get(&key), Some(ImgState::Failed(e)) if e == NOT_DOWNLOADED) {
            self.images.remove(&key);
        }
    }
}
