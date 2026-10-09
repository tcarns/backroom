//! Settings saved between runs, in %APPDATA%\Backroom\settings.json on Windows.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub server: String,
    pub name: String,
    /// Kept only when `remember` is on. Stored as plain text in your user folder.
    pub password: String,
    pub remember: bool,
    /// Connect straight away next time (set after a successful sign-in with "remember").
    pub auto_connect: bool,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub noise_suppression: bool,
    pub push_to_talk: bool,
    /// Windows virtual-key code.
    pub ptt_key: u32,
    /// Voice activation threshold in dBFS.
    pub threshold_db: f32,
    pub sounds: bool,
    pub muted: bool,
    pub deafened: bool,
    /// Per-person volume (0.0 to 2.0), by name.
    pub volumes: BTreeMap<String, f32>,
    /// People you've muted for yourself, by name.
    pub local_mutes: BTreeSet<String>,
    pub last_text_channel: Option<String>,
    /// Look for new versions on GitHub.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: String::new(),
            name: String::new(),
            password: String::new(),
            remember: true,
            auto_connect: false,
            input_device: None,
            output_device: None,
            noise_suppression: true,
            push_to_talk: false,
            ptt_key: 0xC0,
            threshold_db: -45.0,
            sounds: true,
            muted: false,
            deafened: false,
            volumes: BTreeMap::new(),
            local_mutes: BTreeSet::new(),
            last_text_channel: None,
            check_updates: true,
        }
    }
}

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("BACKROOM_SETTINGS") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Backroom")
        .join("settings.json")
}

impl Settings {
    pub fn load() -> Settings {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let mut copy = self.clone();
        if !copy.remember {
            copy.password.clear();
            copy.auto_connect = false;
        }
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(json) = serde_json::to_string_pretty(&copy) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }

    pub fn volume(&self, name: &str) -> f32 {
        *self.volumes.get(name).unwrap_or(&1.0)
    }

    /// The gain the mixer should use for this person.
    pub fn gain(&self, name: &str) -> f32 {
        if self.local_mutes.contains(name) {
            0.0
        } else {
            self.volume(name)
        }
    }
}
