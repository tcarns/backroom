//! Updating the app from inside the app.
//!
//! 1. Download the new `backroom.exe` from the GitHub release, next to the running one.
//! 2. Check its SHA-256 against the release's `SHA256SUMS.txt`. A mismatch stops here.
//! 3. Swap: rename the running exe to `backroom.exe.old` (Windows allows renaming a
//!    running program, just not overwriting or deleting it), then move the new file
//!    into place. If that second step fails, the old file is put back.
//! 4. The UI starts the new exe and closes. The new copy deletes `backroom.exe.old`.

use crate::net::Wake;
use parking_lot::Mutex;
use proto::update::Release;
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

pub const SUMS_FILE: &str = "SHA256SUMS.txt";
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;

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
    supported() && release.asset(&asset_name()).is_some() && release.asset(SUMS_FILE).is_some()
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
    let name = asset_name();
    let asset = release
        .asset(&name)
        .ok_or("This release doesn't include the app for in-app updating.")?;
    let sums = release
        .asset(SUMS_FILE)
        .ok_or("This release has no checksum file, so it can't be installed safely.")?;
    let exe =
        std::env::current_exe().map_err(|e| format!("Couldn't find the running app ({e})."))?;
    let dir = exe
        .parent()
        .ok_or("Couldn't find the app's folder.")?
        .to_path_buf();
    let file_name = exe
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("backroom.exe")
        .to_string();
    let agent = proto::update::agent(Duration::from_secs(600));

    // Expected checksum.
    let mut resp = agent
        .get(&sums.url)
        .header(
            "User-Agent",
            &format!("Backroom/{}", proto::update::CURRENT),
        )
        .call()
        .map_err(|e| format!("Couldn't reach GitHub ({e})."))?;
    if resp.status().as_u16() != 200 {
        return Err(format!(
            "GitHub answered with an error ({}).",
            resp.status().as_u16()
        ));
    }
    let sums_text = resp
        .body_mut()
        .with_config()
        .limit(64 * 1024)
        .read_to_string()
        .map_err(|e| format!("Couldn't read the checksum file ({e})."))?;
    let expected =
        expected_hash(&sums_text, &name).ok_or("The checksum file doesn't list the app.")?;

    // Download next to the running exe, so the final move stays on the same drive.
    let tmp = dir.join(format!("{file_name}.download"));
    let mut file = File::create(&tmp).map_err(|e| {
        format!("Backroom can't save files in its folder ({}): {e}. Move backroom.exe to a folder you own, like Documents\\Backroom, or download the update manually.", dir.display())
    })?;
    let mut resp = agent
        .get(&asset.url)
        .header(
            "User-Agent",
            &format!("Backroom/{}", proto::update::CURRENT),
        )
        .call()
        .map_err(|e| format!("The download failed ({e})."))?;
    if resp.status().as_u16() != 200 {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "The download failed (GitHub answered {}).",
            resp.status().as_u16()
        ));
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
        .unwrap_or(asset.size);
    let mut reader = resp.body_mut().with_config().limit(MAX_DOWNLOAD).reader();
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    let mut last_report = Instant::now();
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                drop(file);
                let _ = std::fs::remove_file(&tmp);
                return Err(format!("The download was interrupted ({e}). Try again."));
            }
        };
        hasher.update(&buf[..n]);
        if let Err(e) = file.write_all(&buf[..n]) {
            drop(file);
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Couldn't save the download ({e})."));
        }
        done += n as u64;
        if last_report.elapsed() > Duration::from_millis(100) {
            *phase.lock() = Phase::Downloading { done, total };
            wake();
            last_report = Instant::now();
        }
    }
    *phase.lock() = Phase::Downloading { done, total };
    let _ = file.sync_all();
    drop(file);

    // Verify before touching the installed app.
    *phase.lock() = Phase::Verifying;
    wake();
    let got = hex(&hasher.finalize());
    if !got.eq_ignore_ascii_case(&expected) {
        let _ = std::fs::remove_file(&tmp);
        return Err("The download didn't match its checksum (it may be incomplete). Nothing was changed. Try again.".into());
    }

    *phase.lock() = Phase::Installing;
    wake();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755));
    }
    install(&tmp, &exe).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Couldn't install the update ({e}). Nothing was changed.")
    })?;
    Ok(exe)
}

/// Put `new_file` where `exe` is, keeping the old one as `<exe>.old` until next start.
pub fn install(new_file: &Path, exe: &Path) -> std::io::Result<()> {
    let old = old_path(exe);
    // A leftover from an earlier update (that program isn't running any more).
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old)?;
    if let Err(e) = std::fs::rename(new_file, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(e);
    }
    Ok(())
}

fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".old");
    exe.with_file_name(name)
}

/// Remove what an update left behind. The previous copy may still be closing,
/// so keep trying for a few seconds.
pub fn cleanup_leftovers() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let old = old_path(&exe);
    let mut download = exe
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    download.push(".download");
    let download = exe.with_file_name(download);
    if !old.exists() && !download.exists() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("cleanup".into())
        .spawn(move || {
            for _ in 0..20 {
                let _ = std::fs::remove_file(&download);
                if !old.exists() || std::fs::remove_file(&old).is_ok() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
}

/// Find `name`'s hash in a `sha256sum`-style file ("<hex>  name" or "<hex> *name").
pub fn expected_hash(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?.trim_start_matches('*');
        (file.eq_ignore_ascii_case(name)
            && hash.len() == 64
            && hash.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| hash.to_ascii_lowercase())
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_hashes() {
        let sums = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef  backroom.exe\nFEDCBA9876543210FEDCBA9876543210FEDCBA9876543210FEDCBA9876543210 *backroom-server.exe\n";
        assert_eq!(
            expected_hash(sums, "backroom.exe").unwrap(),
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            expected_hash(sums, "backroom-server.exe").unwrap(),
            "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
        );
        assert!(expected_hash(sums, "other.exe").is_none());
        assert!(expected_hash("nothex  backroom.exe", "backroom.exe").is_none());
    }

    #[test]
    fn install_swaps_and_keeps_old() {
        let dir = std::env::temp_dir().join(format!("br-install-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("backroom.exe");
        let new = dir.join("backroom.exe.download");
        std::fs::write(&exe, b"old").unwrap();
        std::fs::write(&new, b"new").unwrap();
        install(&new, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(dir.join("backroom.exe.old")).unwrap(), b"old");
        assert!(!new.exists());
        // A failed swap puts the old file back.
        let missing = dir.join("nope.download");
        assert!(install(&missing, &exe).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
