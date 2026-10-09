//! The app's log: `backroom.log` next to the settings (%APPDATA%\Backroom on
//! Windows). Sign-ins, connection trouble, every error the app shows, and file
//! transfer details, so problems can be looked into afterwards. When it passes
//! 2 MB it's renamed to `backroom.log.1` (replacing the previous one).

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

const MAX_BYTES: u64 = 2 * 1024 * 1024;

static FILE: Mutex<Option<(File, u64)>> = Mutex::new(None);

pub fn path() -> PathBuf {
    crate::settings::path().with_file_name("backroom.log")
}

fn open() -> Option<(File, u64)> {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let f = OpenOptions::new().create(true).append(true).open(&p).ok()?;
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    Some((f, len))
}

pub fn write(level: &str, message: &str) {
    let line = format!(
        "{} {level:<5} {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        message.replace('\n', " | ")
    );
    let mut slot = FILE.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = open();
    }
    let Some((file, len)) = slot.as_mut() else {
        return;
    };
    if file.write_all(line.as_bytes()).is_err() {
        *slot = None;
        return;
    }
    *len += line.len() as u64;
    if *len > MAX_BYTES {
        *slot = None;
        let p = path();
        let _ = std::fs::rename(&p, p.with_extension("log.1"));
    }
}

pub fn info(message: impl AsRef<str>) {
    write("INFO", message.as_ref());
}

pub fn warn(message: impl AsRef<str>) {
    write("WARN", message.as_ref());
}

pub fn error(message: impl AsRef<str>) {
    write("ERROR", message.as_ref());
}
