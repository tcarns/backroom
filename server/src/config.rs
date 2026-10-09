//! Server settings: read from `backroom-server.toml` (created on first run),
//! with environment variables overriding the file.

use crate::log::Level;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub struct Config {
    pub port: u16,
    pub host: String,
    pub password: String,
    pub app_name: String,
    pub text_channels: Vec<String>,
    pub voice_channels: Vec<String>,
    pub max_per_voice_channel: usize,
    pub history_limit: usize,
    pub data_dir: PathBuf,
    pub log_level: Level,
    pub log_file: Option<PathBuf>,
    /// Look for newer versions on GitHub and mention them in the log.
    pub check_updates: bool,
    /// Install new versions by itself and restart when nobody's in voice.
    pub auto_update: bool,
    /// New accounts can be created (with the group password).
    pub allow_signup: bool,
    /// Largest image someone can send.
    pub max_attachment_bytes: u64,
    pub config_path: PathBuf,
    /// Notes to log once logging is running (first-run messages, bad values).
    pub notes: Vec<(Level, String)>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FileConfig {
    port: Option<u16>,
    host: Option<String>,
    password: Option<String>,
    app_name: Option<String>,
    text_channels: Option<Vec<String>>,
    voice_channels: Option<Vec<String>>,
    max_per_voice_channel: Option<usize>,
    history_limit: Option<usize>,
    data_dir: Option<String>,
    log_level: Option<String>,
    log_file: Option<String>,
    check_updates: Option<bool>,
    auto_update: Option<bool>,
    allow_signup: Option<bool>,
    max_attachment_mb: Option<u64>,
}

fn base_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn random_password() -> String {
    // Easy to read aloud: no 0/O or 1/l.
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 10];
    getrandom::fill(&mut bytes).expect("system random source unavailable");
    bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

fn default_file(password: &str) -> String {
    format!(
        r#"# Backroom server settings. Restart the server after changing anything here.

# Group password: people need it once, to create their account. After that they
# sign in with their own name and password. Leave it empty ("") to let anyone with
# the address create an account.
password = "{password}"

# Set to false to stop new accounts being created (once everyone has one).
allow_signup = true

port = 3000
app_name = "Backroom"
text_channels = ["general", "links", "memes"]
voice_channels = ["Lounge", "Gaming", "Quiet room"]
max_per_voice_channel = 8

# Chat messages kept per text channel.
history_limit = 300

# Largest image people can send, in megabytes. Images are kept in data/attachments
# and deleted when their message ages out of history.
max_attachment_mb = 8

# How much to log: trace, debug, info, warn, error, critical.
# You can also change it while the server runs by typing: level debug
log_level = "info"

# Set to "" to log only to this window.
log_file = "data/backroom.log"

# Check GitHub for a newer Backroom.
check_updates = true

# Install new versions by itself, then restart once nobody is in voice (people
# reconnect on their own). Set to false to just be told, then type "update" to install.
auto_update = true
"#
    )
}

fn clean_channel(c: &str) -> String {
    c.trim()
        .to_lowercase()
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .filter(|v| !v.is_empty() || key == "PASSWORD" || key == "LOG_FILE")
}
fn list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
}

pub fn load() -> Config {
    let mut notes = Vec::new();
    let base = base_dir();
    let config_path = std::env::var_os("BACKROOM_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| base.join("backroom-server.toml"));

    let mut file = FileConfig::default();
    if config_path.exists() {
        match std::fs::read_to_string(&config_path)
            .map_err(|e| e.to_string())
            .and_then(|s| toml::from_str::<FileConfig>(&s).map_err(|e| e.to_string()))
        {
            Ok(f) => file = f,
            Err(e) => notes.push((
                Level::Error,
                format!(
                    "Could not read {} ({e}). Using default settings.",
                    config_path.display()
                ),
            )),
        }
    } else if std::env::var_os("PASSWORD").is_none() {
        let pw = random_password();
        match std::fs::write(&config_path, default_file(&pw)) {
            Ok(()) => {
                notes.push((Level::Warn, format!("First run: created {} with the group password \"{pw}\". Change it there if you like.", config_path.display())));
                file.password = Some(pw);
            }
            Err(e) => notes.push((
                Level::Warn,
                format!(
                    "Could not create {} ({e}). Using default settings.",
                    config_path.display()
                ),
            )),
        }
    }

    let parse_num =
        |key: &str, v: Option<String>, notes: &mut Vec<(Level, String)>| -> Option<usize> {
            v.and_then(|s| match s.trim().parse() {
                Ok(n) => Some(n),
                Err(_) => {
                    notes.push((
                        Level::Warn,
                        format!("{key}=\"{s}\" isn't a number; ignoring it."),
                    ));
                    None
                }
            })
        };

    let port = parse_num("PORT", env("PORT"), &mut notes)
        .map(|n| n as u16)
        .or(file.port)
        .unwrap_or(3000);
    let host = env("HOST")
        .or(file.host)
        .unwrap_or_else(|| "0.0.0.0".into());
    let password = env("PASSWORD").or(file.password).unwrap_or_default();
    let app_name = env("APP_NAME")
        .or(file.app_name)
        .unwrap_or_else(|| "Backroom".into());
    let mut text_channels: Vec<String> = env("TEXT_CHANNELS")
        .map(|s| list(&s))
        .or(file.text_channels)
        .unwrap_or_else(|| vec!["general".into(), "links".into(), "memes".into()]);
    text_channels = text_channels
        .iter()
        .map(|c| clean_channel(c))
        .filter(|c| !c.is_empty())
        .collect();
    text_channels.dedup();
    if text_channels.is_empty() {
        text_channels.push("general".into());
    }
    let mut voice_channels: Vec<String> = env("VOICE_CHANNELS")
        .map(|s| list(&s))
        .or(file.voice_channels)
        .unwrap_or_else(|| vec!["Lounge".into(), "Gaming".into(), "Quiet room".into()]);
    voice_channels = voice_channels
        .into_iter()
        .map(|c| c.trim().chars().take(40).collect::<String>())
        .filter(|c| !c.is_empty())
        .collect();
    voice_channels.dedup();
    if voice_channels.is_empty() {
        voice_channels.push("Lounge".into());
    }
    let max_per_voice_channel = parse_num("MAX_PER_VOICE", env("MAX_PER_VOICE"), &mut notes)
        .or(file.max_per_voice_channel)
        .unwrap_or(8)
        .clamp(2, 32);
    let history_limit = parse_num("HISTORY_LIMIT", env("HISTORY_LIMIT"), &mut notes)
        .or(file.history_limit)
        .unwrap_or(300)
        .clamp(10, 10_000);

    let resolve = |p: String| -> PathBuf {
        let p = PathBuf::from(p);
        if p.is_absolute() {
            p
        } else {
            base.join(p)
        }
    };
    let data_dir = resolve(
        env("DATA_DIR")
            .or(file.data_dir)
            .unwrap_or_else(|| "data".into()),
    );

    let level_str = env("LOG_LEVEL")
        .or(file.log_level)
        .unwrap_or_else(|| "info".into());
    let log_level = Level::parse(&level_str).unwrap_or_else(|| {
        notes.push((Level::Warn, format!("Unknown log level \"{level_str}\", using info. Options: trace, debug, info, warn, error, critical")));
        Level::Info
    });
    let log_file = match env("LOG_FILE").or(file.log_file) {
        Some(s)
            if s.is_empty()
                || ["off", "none", "false", "0"].contains(&s.to_lowercase().as_str()) =>
        {
            None
        }
        Some(s) => Some(resolve(s)),
        None => Some(data_dir.join("backroom.log")),
    };

    let check_updates = match env("CHECK_UPDATES") {
        Some(v) => !["0", "false", "no", "off"].contains(&v.to_lowercase().as_str()),
        None => file.check_updates.unwrap_or(true),
    };

    let auto_update = match env("AUTO_UPDATE") {
        Some(v) => !["0", "false", "no", "off"].contains(&v.to_lowercase().as_str()),
        None => file.auto_update.unwrap_or(true),
    };

    let allow_signup = match env("ALLOW_SIGNUP") {
        Some(v) => !["0", "false", "no", "off"].contains(&v.to_lowercase().as_str()),
        None => file.allow_signup.unwrap_or(true),
    };

    let max_attachment_mb = parse_num("MAX_ATTACHMENT_MB", env("MAX_ATTACHMENT_MB"), &mut notes)
        .map(|n| n as u64)
        .or(file.max_attachment_mb)
        .unwrap_or(8)
        .clamp(1, 50);

    Config {
        max_attachment_bytes: max_attachment_mb * 1024 * 1024,
        check_updates,
        auto_update,
        allow_signup,
        port,
        host,
        password,
        app_name,
        text_channels,
        voice_channels,
        max_per_voice_channel,
        history_limit,
        data_dir,
        log_level,
        log_file,
        config_path,
        notes,
    }
}
