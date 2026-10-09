//! Updating the app from inside the app: download and install with
//! [`proto::update`], then the UI starts the new copy and closes.

use crate::net::Wake;
use parking_lot::Mutex;
use proto::update::{self, Release};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Downloading {
        done: u64,
        total: u64,
    },
    Verifying,
    Installing,
    /// Installed; the UI should restart into `exe`.
    Ready {
        exe: PathBuf,
        version: String,
    },
    Failed(String),
}

/// The release file this build updates itself from.
pub fn asset_name() -> String {
    std::env::var("BACKROOM_UPDATE_ASSET").unwrap_or_else(|_| "backroom.exe".into())
}

/// In-app updating is for the Windows app (other builds can opt in for testing).
pub fn supported() -> bool {
    cfg!(windows) || std::env::var_os("BACKROOM_UPDATE_ASSET").is_some()
}

/// True if this release can be installed from inside the app.
pub fn can_install(release: &Release) -> bool {
    supported() && update::installable(release, &asset_name())
}

#[derive(Clone)]
pub struct Updater {
    phase: Arc<Mutex<Phase>>,
}

impl Default for Updater {
    fn default() -> Self {
        Self {
            phase: Arc::new(Mutex::new(Phase::Idle)),
        }
    }
}

impl Updater {
    pub fn phase(&self) -> Phase {
        self.phase.lock().clone()
    }

    pub fn busy(&self) -> bool {
        matches!(
            self.phase(),
            Phase::Downloading { .. } | Phase::Verifying | Phase::Installing | Phase::Ready { .. }
        )
    }

    pub fn reset(&self) {
        *self.phase.lock() = Phase::Idle;
    }

    pub fn start(&self, release: Release, wake: Wake) {
        if self.busy() {
            return;
        }
        *self.phase.lock() = Phase::Downloading { done: 0, total: 0 };
        let phase = self.phase.clone();
        let _ = std::thread::Builder::new()
            .name("updater".into())
            .spawn(move || {
                let result = run(&release, &phase, &wake);
                *phase.lock() = match result {
                    Ok(exe) => Phase::Ready {
                        exe,
                        version: release.version.clone(),
                    },
                    Err(e) => Phase::Failed(e),
                };
                wake();
            });
    }
}

fn run(release: &Release, phase: &Mutex<Phase>, wake: &Wake) -> Result<PathBuf, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("Couldn't find the running app ({e})."))?;
    let tmp = update::download_verified(release, &asset_name(), &exe, &mut |done, total| {
        *phase.lock() = Phase::Downloading { done, total };
        wake();
    })?;
    *phase.lock() = Phase::Installing;
    wake();
    update::install(&tmp, &exe).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Couldn't install the update ({e}). Nothing was changed.")
    })?;
    Ok(exe)
}

pub use update::cleanup_leftovers;

/// Start the freshly installed app. It signs back in and rejoins voice using
/// `resume` (passed through the environment, never written to disk).
pub fn relaunch(exe: &Path, resume: &Resume) -> std::io::Result<()> {
    std::process::Command::new(exe)
        .arg("--updated")
        .env(
            RESUME_VAR,
            serde_json::to_string(resume).unwrap_or_default(),
        )
        .spawn()
        .map(|_| ())
}

pub const RESUME_VAR: &str = "BACKROOM_RESUME";

/// What the restarted app needs to carry on where you were.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Resume {
    pub server: String,
    pub name: String,
    pub password: String,
    pub voice: Option<String>,
    pub text_channel: Option<String>,
}

/// Read (and remove) resume info handed over by the previous copy.
pub fn take_resume() -> Option<Resume> {
    let v = std::env::var(RESUME_VAR).ok()?;
    // SAFETY: called once at startup, before any other threads read the environment.
    unsafe { std::env::remove_var(RESUME_VAR) };
    serde_json::from_str(&v).ok()
}
