//! When the app crashes, write down why before it closes: in the app log, and
//! in `last-crash.txt` next to it so the next start can tell the server's log
//! too. Covers Rust panics (the release build stops on the first one) and, on
//! Windows, crashes inside system code such as video decoders and graphics
//! drivers, naming the file (DLL) that crashed.
//!
//! Playing a video is also marked in `playing.txt` while it lasts, so if the
//! app goes away in the middle of one, the next start knows which video it was.

use std::path::PathBuf;
use std::sync::Mutex;

/// What's playing, for crash messages (read without waiting in a crash).
static PLAYING: Mutex<Option<Playback>> = Mutex::new(None);

/// A video or audio file being played in the chat.
#[derive(Clone, Debug, PartialEq)]
pub struct Playback {
    pub id: String,
    pub name: String,
    /// Allowed to decode on the graphics card.
    pub gpu: bool,
}

impl Playback {
    fn line(&self) -> String {
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        format!(
            "{}\t{}\t{}",
            clean(&self.id),
            clean(&self.name),
            if self.gpu { "gpu" } else { "software" }
        )
    }

    fn parse(line: &str) -> Option<Playback> {
        let mut parts = line.trim_end().splitn(3, '\t');
        let id = parts.next()?.to_string();
        let name = parts.next()?.to_string();
        let gpu = parts.next()? == "gpu";
        (!id.is_empty()).then_some(Playback { id, name, gpu })
    }

    pub fn describe(&self) -> String {
        format!(
            "{} ({})",
            self.name,
            if self.gpu {
                "using the graphics card"
            } else {
                "without the graphics card"
            }
        )
    }
}

fn dir() -> PathBuf {
    crate::settings::path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_default()
}

fn crash_file() -> PathBuf {
    dir().join("last-crash.txt")
}

fn playing_file() -> PathBuf {
    dir().join("playing.txt")
}

/// Start recording crashes. Call once, early.
pub fn install() {
    std::panic::set_hook(Box::new(|info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("unnamed");
        record(&format!("{info} (thread {name})"));
    }));
    #[cfg(windows)]
    windows::install();
}

fn record(what: &str) {
    let playing = PLAYING
        .try_lock()
        .ok()
        .and_then(|p| p.as_ref().map(|p| p.describe()));
    let text = match playing {
        Some(p) => format!("{what}, while playing {p}"),
        None => what.to_string(),
    };
    let text = text.replace(['\n', '\r'], " ");
    crate::applog::write_no_wait("CRIT", &format!("The app crashed: {text}"));
    let _ = std::fs::write(crash_file(), &text);
}

/// A file started playing in the chat.
pub fn playing_started(p: Playback) {
    let _ = std::fs::create_dir_all(dir());
    let _ = std::fs::write(playing_file(), p.line());
    if let Ok(mut slot) = PLAYING.lock() {
        *slot = Some(p);
    }
}

/// Playing stopped normally (another file, closed, or the app closing).
pub fn playing_stopped() {
    if let Ok(mut slot) = PLAYING.lock() {
        if slot.take().is_some() {
            let _ = std::fs::remove_file(playing_file());
        }
    }
}

/// How the previous run ended, if badly.
#[derive(Debug, Default, PartialEq)]
pub struct LastRun {
    /// Why it crashed, as recorded then.
    pub crash: Option<String>,
    /// It ended (crashed, or was ended from Task Manager) while this was playing.
    pub interrupted: Option<Playback>,
}

/// Read (and clear) what the previous run left behind. Call once at start.
pub fn take_last_run() -> LastRun {
    let read = |p: PathBuf| {
        let text = std::fs::read_to_string(&p).ok();
        let _ = std::fs::remove_file(&p);
        text
    };
    LastRun {
        crash: read(crash_file())
            .map(|t| t.trim().chars().take(600).collect::<String>())
            .filter(|t| !t.is_empty()),
        interrupted: read(playing_file()).and_then(|t| Playback::parse(&t)),
    }
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::Foundation::HMODULE;
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SetUnhandledExceptionFilter, EXCEPTION_POINTERS,
    };
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    pub fn install() {
        unsafe {
            SetUnhandledExceptionFilter(Some(filter));
        }
    }

    /// Which file (the app, or a DLL) contains `addr`, and how far into it.
    unsafe fn module_of(addr: usize) -> String {
        unsafe {
            let mut module: HMODULE = std::ptr::null_mut();
            let ok = GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
                    | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
                addr as *const u16,
                &mut module,
            );
            if ok == 0 || module.is_null() {
                return format!("unknown code at {addr:#x}");
            }
            let mut buf = [0u16; 260];
            let n = GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) as usize;
            let path = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
            let file = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
            format!("{file}+{:#x}", addr - module as usize)
        }
    }

    unsafe extern "system" fn filter(info: *const EXCEPTION_POINTERS) -> i32 {
        unsafe {
            if let Some(rec) = info.as_ref().and_then(|i| i.ExceptionRecord.as_ref()) {
                let code = rec.ExceptionCode as u32;
                let addr = rec.ExceptionAddress as usize;
                let what = match code {
                    0xC0000005 => {
                        let op = match rec.ExceptionInformation[0] {
                            0 => "reading",
                            1 => "writing",
                            _ => "running",
                        };
                        format!("memory error {op} {:#x}", rec.ExceptionInformation[1])
                    }
                    0xC00000FD => "stack overflow".to_string(),
                    0xC0000409 => "security check failure".to_string(),
                    0xC000001D => "illegal instruction".to_string(),
                    _ => "Windows error".to_string(),
                };
                let thread = std::thread::current();
                let name = thread.name().unwrap_or("system");
                super::record(&format!(
                    "{what} (code {code:#010x}) in {} (thread {name})",
                    module_of(addr)
                ));
            }
        }
        0 // EXCEPTION_CONTINUE_SEARCH: let Windows end the app as usual.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playback_marker_round_trip() {
        let p = Playback {
            id: "abc123".into(),
            name: "recess\t- pain.mp4".into(),
            gpu: true,
        };
        let back = Playback::parse(&p.line()).unwrap();
        assert_eq!(back.id, "abc123");
        assert_eq!(back.name, "recess - pain.mp4");
        assert!(back.gpu);
        assert!(Playback::parse("").is_none());
        assert!(!Playback::parse("x\ty\tsoftware").unwrap().gpu);
    }
}
