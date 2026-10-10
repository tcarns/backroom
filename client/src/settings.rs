//! Settings saved between runs, in %APPDATA%\TUFFcord\settings.json on Windows.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// Starting volume for videos and audio in the chat (a fifth of 0.7's 0.8).
pub const DEFAULT_MEDIA_LEVEL: f32 = 0.16;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub server: String,
    pub name: String,
    /// Your saved sign-in from the server (not your password), kept only when
    /// `remember` is on.
    pub token: String,
    /// The group password, saved by versions before accounts (and still, for
    /// servers that haven't been updated). Used to fill in "Create account".
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
    /// Color theme id (see theme.rs).
    pub theme: String,
    /// The member list on the right is open.
    pub show_members: bool,
    /// Volume for videos and audio played in the chat (0.0 to 1.0). Called
    /// `media_volume` before 0.8, which started louder; that old value is left
    /// behind on purpose so everyone starts at the new, quieter default.
    pub media_level: f32,
    /// Emoji picked recently, newest first.
    pub recent_emoji: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server: String::new(),
            name: String::new(),
            token: String::new(),
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
            theme: "plum".into(),
            show_members: true,
            media_level: DEFAULT_MEDIA_LEVEL,
            recent_emoji: Vec::new(),
        }
    }
}

pub fn path() -> PathBuf {
    if let Some(p) = std::env::var_os("BACKROOM_SETTINGS") {
        return PathBuf::from(p);
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("TUFFcord")
        .join("settings.json")
}

/// The app was called Backroom before 0.8. Move its folders (settings and log in
/// %APPDATA%, downloaded files in %LOCALAPPDATA%) to the new name, once, so
/// people stay signed in and keep their settings. Call before anything else
/// touches those folders.
pub fn move_old_folders() {
    if std::env::var_os("BACKROOM_SETTINGS").is_some() {
        return;
    }
    let config = dirs::config_dir();
    if let Some(base) = &config {
        move_old_folder(base);
    }
    if let Some(base) = dirs::cache_dir().filter(|c| Some(c) != config.as_ref()) {
        move_old_folder(&base);
    }
}

/// `<base>/Backroom` becomes `<base>/TUFFcord` (and the log file inside is
/// renamed), unless a TUFFcord folder is there already. Says what happened.
pub fn move_old_folder(base: &std::path::Path) -> &'static str {
    let old = base.join("Backroom");
    let new = base.join("TUFFcord");
    if !old.is_dir() {
        return "nothing to move";
    }
    if new.exists() {
        return "already moved";
    }
    // Right after an update the old copy may still be closing (with its log
    // open, which stops Windows renaming the folder), so give it a moment.
    let mut moved = false;
    for _ in 0..12 {
        if std::fs::rename(&old, &new).is_ok() {
            moved = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if !moved {
        // Something still has a file open in there: bring the settings over at least.
        if !old.join("settings.json").exists() {
            return "couldn't move";
        }
        let _ = std::fs::create_dir_all(&new);
        let copied = std::fs::copy(old.join("settings.json"), new.join("settings.json")).is_ok();
        return if copied { "copied settings" } else { "couldn't move" };
    }
    for (from, to) in [
        ("backroom.log", "TUFFcord.log"),
        ("backroom.log.1", "TUFFcord.log.1"),
    ] {
        if new.join(from).exists() {
            let _ = std::fs::rename(new.join(from), new.join(to));
        }
    }
    "moved"
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
            copy.token.clear();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_folder_moves_once() {
        let base = std::env::temp_dir().join(format!("tuff-move-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        assert_eq!(move_old_folder(&base), "nothing to move");

        let old = base.join("Backroom");
        std::fs::create_dir_all(old.join("cache")).unwrap();
        std::fs::write(old.join("settings.json"), r#"{"name":"Tyler","token":"t"}"#).unwrap();
        std::fs::write(old.join("backroom.log"), "old log").unwrap();
        std::fs::write(old.join("cache").join("abc.mp4"), "v").unwrap();
        assert_eq!(move_old_folder(&base), "moved");
        let new = base.join("TUFFcord");
        assert!(!old.exists());
        assert!(new.join("settings.json").exists());
        assert_eq!(std::fs::read_to_string(new.join("TUFFcord.log")).unwrap(), "old log");
        assert!(new.join("cache").join("abc.mp4").exists());

        // An old copy run again recreates Backroom; the new folder is left alone.
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("settings.json"), "{}").unwrap();
        assert_eq!(move_old_folder(&base), "already moved");
        assert!(std::fs::read_to_string(new.join("settings.json")).unwrap().contains("Tyler"));
        let _ = std::fs::remove_dir_all(&base);
    }
}
