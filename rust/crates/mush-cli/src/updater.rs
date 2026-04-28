use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

const RELEASES_LATEST_URL: &str = "https://api.github.com/repos/sedoy26/mush/releases/latest";
const PREFS_FILE: &str = ".mush-cli-user.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct UserPrefs {
    pub auto_update: bool,
}

impl Default for UserPrefs {
    fn default() -> Self {
        Self { auto_update: true }
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Debug, Deserialize)]
struct ReleaseInfo {
    tag_name: String,
    assets: Vec<ReleaseAsset>,
}

pub fn load_user_prefs(base_dir: &Path) -> UserPrefs {
    let path = base_dir.join(PREFS_FILE);
    let Ok(text) = fs::read_to_string(&path) else {
        return UserPrefs::default();
    };
    serde_json::from_str::<UserPrefs>(&text).unwrap_or_default()
}

pub fn save_user_prefs(base_dir: &Path, prefs: &UserPrefs) -> Result<()> {
    let path = base_dir.join(PREFS_FILE);
    let text = serde_json::to_string_pretty(prefs).context("serialize user prefs")?;
    fs::write(path, text).context("write user prefs")?;
    Ok(())
}

pub fn spawn_auto_update_check(base_dir: PathBuf, current_version: String) -> Receiver<String> {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let message = match check_and_download_update(&base_dir, &current_version) {
            Ok(Some(msg)) => msg,
            Ok(None) => return,
            Err(e) => format!("Auto-update check failed: {e}"),
        };
        let _ = tx.send(message);
    });
    rx
}

fn check_and_download_update(base_dir: &Path, current_version: &str) -> Result<Option<String>> {
    let release = fetch_latest_release_json(current_version)?;

    let current = parse_tag_version(current_version).context("parse current version")?;
    let latest = match parse_tag_version(&release.tag_name) {
        Some(v) => v,
        None => return Ok(None),
    };
    let latest_text = format!("{}.{}.{}", latest.0, latest.1, latest.2);
    if latest <= current {
        return Ok(None);
    }

    let wanted = target_asset_name();
    let Some(asset) = release.assets.iter().find(|a| a.name == wanted) else {
        return Ok(Some(format!(
            "Update {latest_text} available, but no asset for this platform ({wanted})"
        )));
    };

    let updates_dir = base_dir.join("updates");
    fs::create_dir_all(&updates_dir).context("create updates dir")?;
    let out_path = updates_dir.join(&asset.name);
    if let Ok(meta) = fs::metadata(&out_path) {
        if meta.len() == asset.size {
            return Ok(Some(format!(
                "Update v{latest_text} already downloaded: {}",
                out_path.display()
            )));
        }
    }

    let tmp_path = out_path.with_extension("download");
    download_with_curl(&asset.browser_download_url, &tmp_path, current_version)?;
    fs::rename(&tmp_path, &out_path)
        .with_context(|| format!("move {} to {}", tmp_path.display(), out_path.display()))?;

    Ok(Some(format!(
        "Update v{latest_text} downloaded: {}",
        out_path.display()
    )))
}

fn fetch_latest_release_json(current_version: &str) -> Result<ReleaseInfo> {
    let output = Command::new("curl")
        .arg("--fail")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--connect-timeout")
        .arg("6")
        .arg("--max-time")
        .arg("30")
        .arg("-H")
        .arg("Accept: application/vnd.github+json")
        .arg("-H")
        .arg(format!("User-Agent: mush-cli/{current_version}"))
        .arg(RELEASES_LATEST_URL)
        .output()
        .context("spawn curl for latest release")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("curl latest release failed: {stderr}");
    }
    serde_json::from_slice::<ReleaseInfo>(&output.stdout).context("parse latest release json")
}

fn download_with_curl(url: &str, out_path: &Path, current_version: &str) -> Result<()> {
    let status = Command::new("curl")
        .arg("--fail")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--connect-timeout")
        .arg("10")
        .arg("--max-time")
        .arg("120")
        .arg("-H")
        .arg(format!("User-Agent: mush-cli/{current_version}"))
        .arg("-o")
        .arg(out_path)
        .arg(url)
        .status()
        .with_context(|| format!("spawn curl download for {}", out_path.display()))?;
    if !status.success() {
        anyhow::bail!("curl download failed for {}", out_path.display());
    }
    Ok(())
}

fn parse_tag_version(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches('v');
    let mut parts = trimmed.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch_part = parts.next()?;
    let patch_digits: String = patch_part
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let patch = patch_digits.parse::<u64>().ok()?;
    Some((major, minor, patch))
}

fn target_asset_name() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "mush-cli-macos-aarch64.tar.gz"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "mush-cli-macos-x86_64.tar.gz"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "mush-cli-linux-x86_64.tar.gz"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "mush-cli-windows-x86_64.zip"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64")
    )))]
    {
        "unsupported-platform"
    }
}
