//! Leveled logging to the console (colored) and an optional rotating log file.
//! The level can be changed at any time; no restart needed.

use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Level {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Critical = 5,
}

pub const LEVEL_NAMES: [&str; 6] = ["trace", "debug", "info", "warn", "error", "critical"];
const LABELS: [&str; 6] = ["TRACE", "DEBUG", "INFO ", "WARN ", "ERROR", "CRIT "];
const COLORS: [&str; 6] = [
    "\x1b[90m",
    "\x1b[36m",
    "\x1b[32m",
    "\x1b[33m",
    "\x1b[31m",
    "\x1b[1;97;41m",
];
const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[90m";
const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.trim().to_ascii_lowercase().as_str() {
            "trace" => Some(Level::Trace),
            "debug" => Some(Level::Debug),
            "info" => Some(Level::Info),
            "warn" | "warning" => Some(Level::Warn),
            "error" => Some(Level::Error),
            "critical" | "crit" => Some(Level::Critical),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        LEVEL_NAMES[self as usize]
    }
    fn from_u8(v: u8) -> Level {
        [
            Level::Trace,
            Level::Debug,
            Level::Info,
            Level::Warn,
            Level::Error,
            Level::Critical,
        ][v.min(5) as usize]
    }
}

struct FileSink {
    path: PathBuf,
    file: File,
    bytes: u64,
}

static LEVEL: AtomicU8 = AtomicU8::new(Level::Info as u8);
static COLOR: AtomicBool = AtomicBool::new(false);
static FILE: Mutex<Option<FileSink>> = Mutex::new(None);
// Serializes console + file writes so lines never interleave.
static WRITE_LOCK: Mutex<()> = Mutex::new(());

pub fn init(level: Level, file: Option<PathBuf>) {
    LEVEL.store(level as u8, Ordering::Relaxed);
    COLOR.store(
        std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() && enable_ansi(),
        Ordering::Relaxed,
    );
    if let Some(path) = file {
        match open_file(&path) {
            Ok(sink) => *FILE.lock().unwrap() = Some(sink),
            Err(e) => println!(
                "Could not open log file {}: {e}. Logging to the console only.",
                path.display()
            ),
        }
    }
}

fn open_file(path: &PathBuf) -> std::io::Result<FileSink> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir)?;
        }
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    Ok(FileSink {
        path: path.clone(),
        file,
        bytes,
    })
}

pub fn level() -> Level {
    Level::from_u8(LEVEL.load(Ordering::Relaxed))
}
pub fn set_level(l: Level) {
    LEVEL.store(l as u8, Ordering::Relaxed);
}
#[inline]
pub fn enabled(l: Level) -> bool {
    l as u8 >= LEVEL.load(Ordering::Relaxed)
}
pub fn file_path() -> Option<PathBuf> {
    FILE.lock().unwrap().as_ref().map(|s| s.path.clone())
}

/// "2026-10-09 14:05" in the server's time zone, from milliseconds since 1970.
pub fn local_time(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "?".into())
}

pub fn write(l: Level, category: &str, message: &str, force: bool) {
    if !force && !enabled(l) {
        return;
    }
    let ts = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S%.3f")
        .to_string();
    let i = l as usize;
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if COLOR.load(Ordering::Relaxed) {
        println!(
            "{DIM}{ts}{RESET} {}{}{RESET} {DIM}[{category}]{RESET} {message}",
            COLORS[i], LABELS[i]
        );
    } else {
        println!("{ts} {} [{category}] {message}", LABELS[i]);
    }
    let mut file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(sink) = file.as_mut() {
        let line = format!("{ts} {} [{category}] {message}\n", LABELS[i]);
        if sink.file.write_all(line.as_bytes()).is_err() {
            println!("Writing to the log file failed. Logging to the console only.");
            *file = None;
            return;
        }
        sink.bytes += line.len() as u64;
        if sink.bytes > MAX_FILE_BYTES {
            // Keep one old file: backroom.log -> backroom.log.1
            let path = sink.path.clone();
            *file = None;
            let mut old = path.clone().into_os_string();
            old.push(".1");
            let _ = std::fs::rename(&path, &old);
            *file = open_file(&path).ok();
        }
    }
}

#[cfg(windows)]
fn enable_ansi() -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        STD_OUTPUT_HANDLE,
    };
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        let mut mode = 0;
        if GetConsoleMode(h, &mut mode) == 0 {
            return false;
        }
        SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}
#[cfg(not(windows))]
fn enable_ansi() -> bool {
    true
}

macro_rules! log_at {
    ($lvl:expr, $cat:expr, $($arg:tt)*) => {
        if $crate::log::enabled($lvl) {
            $crate::log::write($lvl, $cat, &format!($($arg)*), false)
        }
    };
}
macro_rules! trace { ($cat:expr, $($arg:tt)*) => { log_at!($crate::log::Level::Trace, $cat, $($arg)*) }; }
macro_rules! debug { ($cat:expr, $($arg:tt)*) => { log_at!($crate::log::Level::Debug, $cat, $($arg)*) }; }
macro_rules! info { ($cat:expr, $($arg:tt)*) => { log_at!($crate::log::Level::Info, $cat, $($arg)*) }; }
macro_rules! warn { ($cat:expr, $($arg:tt)*) => { log_at!($crate::log::Level::Warn, $cat, $($arg)*) }; }
macro_rules! error { ($cat:expr, $($arg:tt)*) => { log_at!($crate::log::Level::Error, $cat, $($arg)*) }; }
macro_rules! critical { ($cat:expr, $($arg:tt)*) => { log_at!($crate::log::Level::Critical, $cat, $($arg)*) }; }
