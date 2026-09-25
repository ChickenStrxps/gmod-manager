//! Self-update from GitHub Releases.
//!
//! A running Windows executable can be renamed but not overwritten, so an update downloads
//! the new build beside the app, moves the running one to `*.old`, puts the new one in its
//! place, and restarts. The leftover `*.old` is deleted on the next start.

use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const REPO: &str = "ChickenStrxps/gmod-manager";
const EXE_ASSET: &str = "gmod-manager.exe";
const SHA_ASSET: &str = "gmod-manager.exe.sha256";
const MAX_EXE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    exe_url: String,
    exe_size: u64,
    sha_url: String,
}

/// True when `candidate` is a higher dotted version than `current` (a leading `v` is ignored).
pub fn is_newer(candidate: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim()
            .trim_start_matches('v')
            .split(['.', '-', '+'])
            .take(3)
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    let (a, b) = (parse(candidate), parse(current));
    (0..3)
        .map(|i| a.get(i).copied().unwrap_or(0))
        .cmp((0..3).map(|i| b.get(i).copied().unwrap_or(0)))
        == std::cmp::Ordering::Greater
}

/// Asks GitHub for the latest release. `Ok(None)` means this build is current.
pub fn check() -> Result<Option<Release>, String> {
    let body = crate::workshop::agent()
        .get(&format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("Couldn't check for updates: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    let release: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let tag = release["tag_name"].as_str().unwrap_or_default();
    if !is_newer(tag, env!("CARGO_PKG_VERSION")) {
        return Ok(None);
    }
    let assets = release["assets"].as_array().cloned().unwrap_or_default();
    let find = |name: &str| {
        assets
            .iter()
            .find(|a| a["name"].as_str() == Some(name))
            .cloned()
    };
    let (Some(exe), Some(sha)) = (find(EXE_ASSET), find(SHA_ASSET)) else {
        return Ok(None);
    };
    Ok(Some(Release {
        version: tag.trim_start_matches('v').to_owned(),
        exe_url: exe["browser_download_url"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        exe_size: exe["size"].as_u64().unwrap_or(0),
        sha_url: sha["browser_download_url"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
    }))
}

fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    exe.with_file_name(name)
}

/// Removes files left behind by a previous update.
pub fn cleanup() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = fs::remove_file(sibling(&exe, ".old"));
        let _ = fs::remove_file(sibling(&exe, ".new"));
    }
}

/// Downloads and verifies the release, then swaps it in for the running executable.
/// The caller should start the new executable and exit.
pub fn install(release: &Release, progress: &mut dyn FnMut(f32)) -> Result<PathBuf, String> {
    if release.exe_size == 0 || release.exe_size > MAX_EXE_BYTES {
        return Err("The update file looks wrong; skipping it.".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let agent = crate::workshop::agent();
    let expected = agent
        .get(&release.sha_url)
        .call()
        .map_err(|e| format!("Update download failed: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let new = sibling(&exe, ".new");
    let result = (|| -> Result<(), String> {
        let mut reader = agent
            .get(&release.exe_url)
            .call()
            .map_err(|e| format!("Update download failed: {e}"))?
            .into_reader()
            .take(MAX_EXE_BYTES + 1);
        let mut file = fs::File::create(&new).map_err(|e| e.to_string())?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut total = 0u64;
        let mut head = [0u8; 2];
        loop {
            let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            for (i, byte) in buffer[..n].iter().enumerate() {
                match head.get_mut(total as usize + i) {
                    Some(slot) => *slot = *byte,
                    None => break,
                }
            }
            hasher.update(&buffer[..n]);
            file.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
            total += n as u64;
            progress(total as f32 / release.exe_size as f32);
        }
        file.sync_all().map_err(|e| e.to_string())?;
        let actual = format!("{:x}", hasher.finalize());
        if total != release.exe_size || &head != b"MZ" || actual != expected {
            return Err("The update didn't download correctly. Try again later.".into());
        }
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&new);
        return Err(e);
    }
    let old = sibling(&exe, ".old");
    let _ = fs::remove_file(&old);
    fs::rename(&exe, &old).map_err(|e| {
        let _ = fs::remove_file(&new);
        format!("Couldn't replace the app (is the folder read-only?): {e}")
    })?;
    if let Err(e) = fs::rename(&new, &exe) {
        let _ = fs::rename(&old, &exe);
        let _ = fs::remove_file(&new);
        return Err(format!("Couldn't replace the app: {e}"));
    }
    Ok(exe)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_is_numeric() {
        assert!(is_newer("v0.5.10", "0.5.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(is_newer("0.6", "0.5.1"));
        assert!(!is_newer("v0.5.1", "0.5.1"));
        assert!(!is_newer("v0.5.0", "0.5.1"));
        assert!(!is_newer("", "0.5.1"));
    }
}
