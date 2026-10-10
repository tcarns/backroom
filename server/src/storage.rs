//! Room for sent files: the `max_storage_mb` limit (oldest files go first)
//! and the warning admins see when files or the disk are nearly full.

use crate::{attachments_dir, files, State};
use proto::ServerMsg;

/// Warn admins from this share of the limit (or of the disk in use)...
const WARN: f64 = 0.80;
/// ...and say it's urgent from this one.
const URGENT: f64 = 0.95;

/// Delete the oldest files until everything fits in `max_storage_mb`.
/// Their messages stay, with the files marked expired.
pub fn enforce(st: &mut State) {
    let max = st.cfg.max_storage_bytes;
    let dir = attachments_dir(&st.cfg);
    while st.files.stored > max {
        // The oldest message that still has a file, recent or archived.
        let recent = st
            .history
            .values()
            .flatten()
            .filter(|m| m.attachments.iter().any(|a| !a.expired))
            .min_by_key(|m| m.ts)
            .map(|m| (m.ts, m.channel.clone(), m.id.clone()));
        let archived = st.archive.oldest_with_files();
        let mut gone = Vec::new();
        match (recent, archived) {
            (None, None) => break,
            (Some((ts, ..)), Some((ats, msg))) if ats <= ts => {
                gone = st.archive.expire(&msg);
            }
            (None, Some((_, msg))) => gone = st.archive.expire(&msg),
            (Some((_, channel, msg_id)), _) => {
                if let Some(m) = st
                    .history
                    .get_mut(&channel)
                    .and_then(|l| l.iter_mut().find(|m| m.id == msg_id))
                {
                    for a in m.attachments.iter_mut().filter(|a| !a.expired) {
                        a.expired = true;
                        gone.push(a.id.clone());
                        if let Some(p) = a.poster.take() {
                            gone.push(p.id);
                        }
                    }
                }
                st.history_dirty = true;
            }
        }
        for id in &gone {
            st.files.stored = st.files.stored.saturating_sub(files::remove_file(&dir, id));
        }
        info!(
            "files",
            "Deleted {} old file(s) to stay under the storage limit",
            gone.len()
        );
        for id in gone {
            st.broadcast(&ServerMsg::AttachmentGone { id });
        }
    }
    check(st);
}

/// What admins should be told now: (text, urgent), or `None` when all's well.
fn warning(st: &State) -> Option<(String, bool)> {
    let limit = st.cfg.max_storage_bytes.max(1);
    let used = st.files.stored;
    let share = used as f64 / limit as f64;
    let disk = disk_space(&st.cfg.data_dir);
    let disk_share = disk.map_or(0.0, |(free, total)| 1.0 - free as f64 / total.max(1) as f64);
    if share < WARN && disk_share < WARN {
        return None;
    }
    let urgent = share >= URGENT || disk_share >= URGENT;
    let mut text = String::new();
    if share >= WARN {
        text += &format!(
            "Sent files use {} of the {} limit ({:.0}%). Past it, the oldest files are deleted. ",
            gb(used),
            gb(limit),
            share * 100.0
        );
    }
    if let Some((free, total)) = disk.filter(|_| disk_share >= WARN) {
        text += &format!(
            "The server's disk is {:.0}% full ({} free of {}). ",
            disk_share * 100.0,
            gb(free),
            gb(total)
        );
    }
    text += "Add disk space, then raise max_storage_mb in TUFFcord-server.toml.";
    Some((text, urgent))
}

/// Tell admins when the warning changes (called after uploads and once a minute).
pub fn check(st: &mut State) {
    let now = warning(st);
    if now == st.storage_warning {
        return;
    }
    match &now {
        Some((text, urgent)) => {
            if *urgent {
                error!("files", "{text}");
            } else {
                warn!("files", "{text}");
            }
        }
        None => info!("files", "Storage is fine again"),
    }
    st.storage_warning = now;
    send_to_admins(st, None);
}

/// Send the current warning to every admin (or just `conn`). Nothing is sent
/// while all's well, unless it's to clear an earlier warning.
pub fn send_to_admins(st: &State, conn: Option<u32>) {
    let (text, critical) = st.storage_warning.clone().unwrap_or_default();
    if conn.is_some() && text.is_empty() {
        return;
    }
    let msg = ServerMsg::StorageWarning { text, critical };
    for c in st.clients.values().filter(|c| c.authed && c.admin) {
        if conn.is_none_or(|id| id == c.id) {
            st.send(c.id, &msg);
        }
    }
}

fn gb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{} MB", bytes / 1_000_000)
    }
}

/// Free and total bytes on the disk holding `dir` (what this program may use).
#[cfg(unix)]
fn disk_space(dir: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut s) } != 0 {
        return None;
    }
    // Like `df`: space reserved for root counts as neither used nor free.
    let unit = s.f_frsize as u64;
    let used = (s.f_blocks as u64).saturating_sub(s.f_bfree as u64) * unit;
    let free = s.f_bavail as u64 * unit;
    Some((free, used + free))
}

#[cfg(windows)]
fn disk_space(dir: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
    let (mut free, mut total) = (0u64, 0u64);
    let ok =
        unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, std::ptr::null_mut()) };
    (ok != 0).then_some((free, total))
}

#[cfg(not(any(unix, windows)))]
fn disk_space(_dir: &std::path::Path) -> Option<(u64, u64)> {
    None
}
