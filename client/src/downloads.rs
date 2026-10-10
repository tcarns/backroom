//! Downloads to the cache on disk: on request (Play, Open) and ahead of
//! time for small clips that come into view, so Play starts at once.

use crate::attach::{Attachments, Fetch, Then, Work};
use crate::attach_ui::Act;
use crate::App;
use proto::Attachment;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};
use tuffcord::files as xfer;
use tuffcord::images;

impl Attachments {
    /// Download a file to the cache (if it isn't there yet), then do `then`.
    pub fn fetch(&mut self, a: &Attachment, then: Then) {
        if then != Then::Nothing {
            // Pressed Play on a clip that's preloading: show its progress.
            self.preloads.remove(&a.id);
        }
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

    /// Should this clip start downloading now, so Play starts at once? Clips
    /// up to `limit` bytes, one at a time, each only once.
    pub fn should_preload(&self, a: &Attachment, limit: u64) -> bool {
        a.size > 0
            && a.size <= limit
            && self.preloads.is_empty()
            && self.endpoint.is_some()
            && !self.fetches.contains_key(&a.id)
            && !self.no_preload.contains(&a.id)
    }

    /// Download a clip ahead of time (see `should_preload`).
    pub fn preload(&mut self, a: &Attachment) {
        self.preloads.insert(a.id.clone());
        self.fetch(a, Then::Nothing);
        if !matches!(self.fetches.get(&a.id), Some(Fetch::Downloading)) {
            // Already in the cache.
            self.preloads.remove(&a.id);
        }
    }

    /// Is this a preload nobody has pressed Play on yet?
    pub fn preloading(&self, id: &str) -> bool {
        self.preloads.contains(id)
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
}

impl App {
    /// Clips shown in the open channel download ahead of time when they're
    /// small enough (Settings, "Download videos and audio ahead of time").
    pub(crate) fn maybe_preload(&self, a: &Attachment, acts: &mut Vec<Act>) {
        let limit = u64::from(self.s.preload_mb) * 1024 * 1024;
        if self.att.should_preload(a, limit) {
            acts.push(Act::Preload(a.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tuffcord::files::Endpoint;

    #[test]
    fn preloads_small_clips_one_at_a_time() {
        let mut att = Attachments::new(Arc::new(|| {}));
        let clip = |id: &str, size: u64| Attachment {
            id: id.into(),
            name: "clip.mp4".into(),
            mime: "video/mp4".into(),
            size,
            width: 640,
            height: 360,
            duration_ms: 1000,
            poster: None,
            expired: false,
        };
        let mb = 1024 * 1024;
        // Old servers have no file endpoint.
        assert!(!att.should_preload(&clip("a1", mb), 50 * mb));
        att.endpoint = Some(Endpoint {
            base: "http://127.0.0.1:1".into(),
            key: "k".into(),
        });
        assert!(att.should_preload(&clip("a1", mb), 50 * mb));
        assert!(!att.should_preload(&clip("a1", 51 * mb), 50 * mb));
        assert!(!att.should_preload(&clip("a1", mb), 0), "0 turns it off");
        att.preloads.insert("b2".into());
        assert!(
            !att.should_preload(&clip("a1", mb), 50 * mb),
            "one at a time"
        );
        att.preloads.clear();
        att.fetches.insert("a1".into(), Fetch::Downloading);
        assert!(
            !att.should_preload(&clip("a1", mb), 50 * mb),
            "already fetching"
        );
        att.no_preload.insert("c3".into());
        assert!(
            !att.should_preload(&clip("c3", mb), 50 * mb),
            "failed before"
        );
    }
}
