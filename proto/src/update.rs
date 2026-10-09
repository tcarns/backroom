//! Checks GitHub for a newer published release of Backroom.
//!
//! Releases are created by the GitHub Actions workflow when a version tag (v1.2.3)
//! is pushed. This only asks "what's the latest release?"; it never downloads or
//! installs anything by itself.

use std::time::Duration;

/// The GitHub repository releases are published to.
pub const REPO: &str = "tcarns/backroom";
/// The running program's version (from Cargo.toml).
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
/// How often to look for updates while running.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// Without the leading "v", e.g. "0.3.0".
    pub version: String,
    /// The release page, where the download is.
    pub url: String,
    /// What changed, as written in the release notes (trimmed).
    pub notes: String,
}

pub fn releases_page() -> String {
    format!("https://github.com/{REPO}/releases/latest")
}

/// "v1.2.3", "1.2", "1.2.3-beta" -> (1, 2, 3)
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Ask GitHub for the newest release. `Ok(None)` means you're up to date
/// (or nothing has been published yet).
pub fn check() -> Result<Option<Release>, String> {
    check_against(CURRENT)
}

pub fn check_against(current: &str) -> Result<Option<Release>, String> {
    // BACKROOM_UPDATE_URL lets tests point this at a local file server.
    let url = std::env::var("BACKROOM_UPDATE_URL")
        .unwrap_or_else(|_| format!("https://api.github.com/repos/{REPO}/releases/latest"));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .provider(ureq::tls::TlsProvider::NativeTls)
                .build(),
        )
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resp = agent
        .get(&url)
        .header("User-Agent", &format!("Backroom/{current}"))
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("couldn't reach GitHub ({e})"))?;
    match resp.status().as_u16() {
        200 => {}
        404 => return Ok(None), // no releases yet
        403 | 429 => return Err("GitHub asked us to slow down; will try again later".into()),
        code => return Err(format!("GitHub answered with an error ({code})")),
    }
    let body = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("couldn't read GitHub's answer ({e})"))?;
    parse_release(&body, current)
}

pub fn parse_release(json: &str, current: &str) -> Result<Option<Release>, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("unexpected answer from GitHub ({e})"))?;
    if v["draft"].as_bool() == Some(true) || v["prerelease"].as_bool() == Some(true) {
        return Ok(None);
    }
    let tag = v["tag_name"]
        .as_str()
        .ok_or("unexpected answer from GitHub (no version)")?;
    if !is_newer(tag, current) {
        return Ok(None);
    }
    let notes: String = v["body"]
        .as_str()
        .unwrap_or("")
        .trim()
        .chars()
        .take(600)
        .collect();
    Ok(Some(Release {
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        url: v["html_url"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(releases_page),
        notes,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.3"), Some((0, 3, 0)));
        assert_eq!(parse_version("2.0.1-beta.1"), Some((2, 0, 1)));
        assert!(is_newer("v0.2.1", "0.2.0"));
        assert!(is_newer("v0.10.0", "0.9.9"));
        assert!(!is_newer("v0.2.0", "0.2.0"));
        assert!(!is_newer("v0.1.9", "0.2.0"));
        assert!(!is_newer("nonsense", "0.2.0"));
    }

    #[test]
    fn release_json() {
        let json = r#"{"tag_name":"v0.3.0","html_url":"https://github.com/tcarns/backroom/releases/tag/v0.3.0","body":"Screen sharing","draft":false,"prerelease":false}"#;
        let r = parse_release(json, "0.2.0").unwrap().unwrap();
        assert_eq!(r.version, "0.3.0");
        assert_eq!(r.notes, "Screen sharing");
        assert_eq!(parse_release(json, "0.3.0").unwrap(), None);
        let pre = json.replace(r#""prerelease":false"#, r#""prerelease":true"#);
        assert_eq!(parse_release(&pre, "0.2.0").unwrap(), None);
    }
}
