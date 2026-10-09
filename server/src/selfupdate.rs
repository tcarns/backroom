//! Keeping the server up to date without anyone touching it.
//!
//! Double-clicking `backroom-server.exe` starts a tiny *watcher*, which starts the
//! real server as a second copy of itself and waits. Both share the window.
//!
//! - When a new release is out, the server downloads `backroom-server.exe`, checks it
//!   against the release's checksums, and swaps it in (see `proto::update`). Then, once
//!   nobody is in voice and chat has been quiet for a minute, it tells everyone it's
//!   restarting, saves chat history, and exits with [`RESTART_CODE`]. The watcher starts
//!   the new server straight away; the app reconnects on its own and rejoins voice.
//! - If the server crashes after running a while, the watcher starts it again.
//! - Ctrl+C or closing the window stops both.
//!
//! The watcher keeps running the version it started as, so what passes between it and
//! the server (the [`CHILD_VAR`] variable and the exit codes) must never change.

use crate::log;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Set on the server copy the watcher starts.
pub const CHILD_VAR: &str = "BACKROOM_SERVER_CHILD";
/// Why the watcher (re)started this copy: "update" or "crash:<code>".
const REASON_VAR: &str = "BACKROOM_SERVER_RESTART_REASON";
/// The server exits with this to be started again (from the freshly installed file).
pub const RESTART_CODE: i32 = 75;
/// A server that crashes sooner than this after starting isn't restarted (it would
/// just crash again, e.g. because of a bad setting).
const MIN_UPTIME_FOR_RESTART: Duration = Duration::from_secs(30);
/// Windows' exit code for a program stopped with Ctrl+C / Ctrl+Break.
const STATUS_CONTROL_C_EXIT: i32 = 0xC000013Au32 as i32;

/// Run as the watcher unless this is the server copy (or the watcher is turned off
/// with BACKROOM_NO_WATCHER=1, e.g. when a service manager already restarts it).
/// Returns only in the server copy.
pub fn watch_unless_child() {
    if std::env::var_os(CHILD_VAR).is_some() || std::env::var_os("BACKROOM_NO_WATCHER").is_some() {
        return;
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return, // can't find ourselves: just be the server
    };
    std::process::exit(watch(exe));
}

/// True when a watcher will start us again after [`RESTART_CODE`].
pub fn supervised() -> bool {
    std::env::var_os(CHILD_VAR).is_some()
}

static RESTARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// True if the watcher started this copy again (after an update or a crash).
pub fn was_restarted() -> bool {
    RESTARTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// What the watcher said about why this copy started, for the log.
pub fn restart_reason() -> Option<String> {
    let r = std::env::var(REASON_VAR).ok()?;
    RESTARTED.store(true, std::sync::atomic::Ordering::Relaxed);
    // SAFETY: called once at startup, before other threads read the environment.
    unsafe { std::env::remove_var(REASON_VAR) };
    Some(match r.strip_prefix("crash:") {
        Some(code) if code.starts_with("signal") => {
            format!("Started again after the server stopped unexpectedly ({code})")
        }
        Some(code) => {
            format!("Started again after the server stopped unexpectedly (exit code {code})")
        }
        None => "Restarted to finish updating".to_string(),
    })
}

fn watch(exe: PathBuf) -> i32 {
    ignore_ctrl_c();
    let mut reason: Option<String> = None;
    loop {
        let started = Instant::now();
        let mut cmd = std::process::Command::new(&exe);
        cmd.args(std::env::args_os().skip(1)).env(CHILD_VAR, "1");
        if let Some(r) = &reason {
            cmd.env(REASON_VAR, r);
        }
        let status = match cmd.spawn() {
            Ok(mut child) => {
                stop_together(&child);
                match child.wait() {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("Backroom: lost track of the server ({e}).");
                        return 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("Backroom: couldn't start the server ({e}).");
                return 1;
            }
        };
        match exit_code(&status) {
            Some(RESTART_CODE) => reason = Some("update".into()),
            // Stopped on purpose (Ctrl+C), or a startup problem it already explained.
            Some(0) | Some(1) | Some(STATUS_CONTROL_C_EXIT) | None => {
                return status.code().unwrap_or(0)
            }
            Some(code) if code >= SIGNALED => {
                // Unix: a crash (abort, segfault) counts; being told to stop doesn't.
                let sig = code - SIGNALED;
                if matches!(sig, 1 | 2 | 9 | 15) {
                    return 0;
                }
                if started.elapsed() < MIN_UPTIME_FOR_RESTART {
                    eprintln!("Backroom: the server stopped right after starting (signal {sig}). Not restarting it.");
                    return 1;
                }
                eprintln!("Backroom: the server stopped unexpectedly (signal {sig}). Starting it again in 3 seconds…");
                std::thread::sleep(Duration::from_secs(3));
                reason = Some(format!("crash:signal {sig}"));
            }
            Some(code) => {
                if started.elapsed() < MIN_UPTIME_FOR_RESTART {
                    eprintln!(
                        "Backroom: the server stopped right after starting (exit code {code}). Not restarting it; see the messages above."
                    );
                    return code;
                }
                eprintln!(
                    "Backroom: the server stopped unexpectedly (exit code {code}). Starting it again in 3 seconds…"
                );
                std::thread::sleep(Duration::from_secs(3));
                reason = Some(format!("crash:{code}"));
            }
        }
    }
}

/// If the watcher is ended (e.g. from Task Manager), end the server with it,
/// so a second copy can start cleanly. Windows does this through a job object.
#[cfg(windows)]
fn stop_together(child: &std::process::Child) {
    use std::os::windows::io::AsRawHandle;
    use std::sync::OnceLock;
    use windows_sys::Win32::System::JobObjects::*;
    static JOB: OnceLock<usize> = OnceLock::new();
    let job = *JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        // Deliberately never closed: Windows closes it when the watcher ends,
        // which is what ends the server.
        job as usize
    });
    if job != 0 {
        unsafe {
            AssignProcessToJobObject(job as _, child.as_raw_handle() as _);
        }
    }
}

#[cfg(not(windows))]
fn stop_together(_child: &std::process::Child) {}

/// Exit codes stand in for Unix signals from here up (never seen on Windows,
/// where the server's own codes are small or NTSTATUS values, i.e. negative).
const SIGNALED: i32 = 1 << 20;

fn exit_code(status: &std::process::ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return Some(SIGNALED + sig);
        }
    }
    status.code()
}

/// Ctrl+C reaches every program in the window. The server shuts itself down
/// cleanly; the watcher ignores it and exits once the server has. (A handler is
/// used rather than ignoring Ctrl+C outright, because "ignore" would be inherited
/// by the server.)
#[cfg(windows)]
fn ignore_ctrl_c() {
    use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_C_EVENT};
    unsafe extern "system" fn handler(kind: u32) -> windows_sys::core::BOOL {
        (kind == CTRL_C_EVENT) as windows_sys::core::BOOL
    }
    unsafe {
        SetConsoleCtrlHandler(Some(handler), 1);
    }
}

#[cfg(not(windows))]
fn ignore_ctrl_c() {
    // In a terminal, Ctrl+C stops the watcher and the server together, and the
    // server still saves before exiting. Nothing to do.
}

// ---------------------------------------------------------------- checking and installing

/// `backroom-server --version` prints this and exits (handled before anything else).
pub fn print_version_if_asked() {
    if std::env::args()
        .skip(1)
        .any(|a| a == "--version" || a == "-V")
    {
        println!("backroom-server {}", proto::update::CURRENT);
        std::process::exit(0);
    }
}

/// Run `exe --version` (giving up after 15 seconds) and return the version it prints.
fn reported_version(exe: &std::path::Path) -> Result<String, String> {
    use std::process::{Command, Stdio};
    let mut child = Command::new(exe)
        .arg("--version")
        .env_remove(CHILD_VAR)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(_) => break,
            None if Instant::now() > deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("it didn't answer".into());
            }
            None => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let mut out = String::new();
    use std::io::Read;
    let _ = child.stdout.take().map(|mut o| o.read_to_string(&mut out));
    out.trim()
        .strip_prefix("backroom-server ")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| format!("unexpected answer {:?}", out.trim()))
}

/// The release file the server installs from.
pub fn asset_name() -> String {
    std::env::var("BACKROOM_SERVER_UPDATE_ASSET").unwrap_or_else(|_| "backroom-server.exe".into())
}

/// Installing by itself is for the Windows server (other builds can opt in for testing).
fn can_self_install() -> bool {
    cfg!(windows) || std::env::var_os("BACKROOM_SERVER_UPDATE_ASSET").is_some()
}

/// Asks from the console.
pub enum Request {
    /// `update`: check now, install if there's something new, and restart right away.
    Now,
}

pub struct Checker {
    pub periodic: bool,
    pub auto_install: bool,
    pub requests: mpsc::Receiver<Request>,
    /// Called with the version once it's installed and the server should restart:
    /// `true` = restart now, `false` = when nobody's using it.
    pub installed: Box<dyn Fn(String, bool) + Send>,
}

impl Checker {
    pub fn run(self) {
        if self.periodic {
            std::thread::sleep(Duration::from_secs(3));
        }
        let mut announced: Option<String> = None;
        let mut installed: Option<String> = None;
        let mut asked = false;
        loop {
            if self.periodic || asked {
                self.check_once(asked, &mut announced, &mut installed);
            }
            let next = if self.periodic {
                self.requests.recv_timeout(proto::update::CHECK_EVERY)
            } else {
                self.requests
                    .recv()
                    .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
            };
            asked = match next {
                Ok(Request::Now) => true,
                Err(mpsc::RecvTimeoutError::Timeout) => false,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            };
        }
    }

    fn check_once(
        &self,
        asked: bool,
        announced: &mut Option<String>,
        installed: &mut Option<String>,
    ) {
        if asked {
            if let Some(v) = installed.clone() {
                (self.installed)(v, true);
                return;
            }
            info!("update", "Checking for a new version…");
        }
        let release = match proto::update::check() {
            Ok(Some(r)) => r,
            Ok(None) => {
                let level = if asked {
                    log::Level::Info
                } else {
                    log::Level::Debug
                };
                log::write(
                    level,
                    "update",
                    &format!("Backroom server {} is up to date", proto::update::CURRENT),
                    asked,
                );
                return;
            }
            Err(e) => {
                let level = if asked {
                    log::Level::Warn
                } else {
                    log::Level::Debug
                };
                log::write(
                    level,
                    "update",
                    &format!("Couldn't check for updates: {e}"),
                    asked,
                );
                return;
            }
        };
        if installed.as_deref() == Some(release.version.as_str()) {
            return;
        }
        let asset = asset_name();
        let can_install = can_self_install() && proto::update::installable(&release, &asset);
        if !(asked || self.auto_install) || !can_install {
            if announced.as_deref() != Some(release.version.as_str()) {
                let how = if !can_install {
                    format!("Download it here: {}", release.url)
                } else {
                    "Type \"update\" to install it now.".to_string()
                };
                warn!(
                    "update",
                    "Backroom {} is available (this server is {}). {how}",
                    release.version,
                    proto::update::CURRENT
                );
                *announced = Some(release.version.clone());
            }
            return;
        }

        info!(
            "update",
            "Downloading Backroom server {} (this server is {})…",
            release.version,
            proto::update::CURRENT
        );
        let result = std::env::current_exe()
            .map_err(|e| format!("Couldn't find the running server ({e})."))
            .and_then(|exe| {
                let tmp = proto::update::download_verified(&release, &asset, &exe, &mut |_, _| {})?;
                // Make sure it starts on this PC and is the version it claims to be,
                // so a mislabeled release can't make the server update over and over.
                match reported_version(&tmp) {
                    Ok(v) if v == release.version => {}
                    Ok(v) => {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(format!("The downloaded server says it's version {v}, not {}. Nothing was changed.", release.version));
                    }
                    Err(e) => {
                        let _ = std::fs::remove_file(&tmp);
                        return Err(format!("The downloaded server didn't start ({e}). Nothing was changed."));
                    }
                }
                proto::update::install(&tmp, &exe).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    format!("Couldn't install it ({e}). Nothing was changed.")
                })
            });
        match result {
            Ok(()) => {
                *installed = Some(release.version.clone());
                (self.installed)(release.version.clone(), asked);
            }
            Err(e) => {
                error!(
                    "update",
                    "Updating to {} failed: {} It will try again in a few hours, or type \"update\" to try now.",
                    release.version,
                    e.trim_end_matches(" Try again.")
                );
            }
        }
    }
}
