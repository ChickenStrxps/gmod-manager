//! Steam Workshop metadata (titles, sizes, preview images) cached beside the app.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const DETAILS_URL: &str =
    "https://api.steampowered.com/ISteamRemoteStorage/GetPublishedFileDetails/v1/";
const MAX_ICON_BYTES: u64 = 8 * 1024 * 1024;
pub const ICON_SIZE: u32 = 96;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemMeta {
    #[serde(default)]
    pub title: String,
    /// Steam's reported file size in bytes (the download size).
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub preview_url: String,
    #[serde(default)]
    pub time_updated: u64,
    #[serde(default)]
    pub subscribers: u64,
    #[serde(default)]
    pub available: bool,
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("GModManager/0.5")
        .build()
}

fn meta_path(dir: &Path) -> PathBuf {
    dir.join("cache").join("workshop.json")
}

pub fn load_meta(dir: &Path) -> HashMap<String, ItemMeta> {
    fs::read(meta_path(dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_meta(dir: &Path, meta: &HashMap<String, ItemMeta>) -> Result<(), String> {
    let path = meta_path(dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec(meta).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

/// Parses a `GetPublishedFileDetails` response. Items Steam could not resolve are
/// returned with `available == false` so callers can tell them apart from unknown IDs.
pub fn parse_details(value: &serde_json::Value) -> HashMap<String, ItemMeta> {
    let mut found = HashMap::new();
    let Some(items) = value["response"]["publishedfiledetails"].as_array() else {
        return found;
    };
    for item in items {
        let Some(id) = item["publishedfileid"].as_str() else {
            continue;
        };
        let number = |key: &str| {
            item[key]
                .as_u64()
                .or_else(|| item[key].as_str().and_then(|s| s.parse().ok()))
                .unwrap_or(0)
        };
        found.insert(
            id.to_owned(),
            ItemMeta {
                title: item["title"].as_str().unwrap_or_default().to_owned(),
                size: number("file_size"),
                preview_url: item["preview_url"].as_str().unwrap_or_default().to_owned(),
                time_updated: number("time_updated"),
                subscribers: number("subscriptions"),
                available: item["result"].as_i64() == Some(1),
            },
        );
    }
    found
}

/// Requests details for the given IDs, 100 per request.
pub fn fetch_details(agent: &ureq::Agent, ids: &[String]) -> Result<serde_json::Value, String> {
    let mut form = Vec::<(String, String)>::with_capacity(ids.len() + 1);
    form.push(("itemcount".into(), ids.len().to_string()));
    for (index, id) in ids.iter().enumerate() {
        form.push((format!("publishedfileids[{index}]"), id.clone()));
    }
    let form: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let body = agent
        .post(DETAILS_URL)
        .send_form(&form)
        .map_err(|e| format!("Steam request failed: {e}"))?
        .into_string()
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

pub fn fetch_meta(ids: &[String]) -> Result<HashMap<String, ItemMeta>, String> {
    let agent = agent();
    let mut all = HashMap::new();
    for chunk in ids.chunks(100) {
        all.extend(parse_details(&fetch_details(&agent, chunk)?));
    }
    Ok(all)
}

pub fn icon_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("cache").join("icons").join(format!("{id}.png"))
}

/// Returns an RGBA thumbnail, reading the disk cache first and downloading on a miss.
pub fn load_icon(dir: &Path, id: &str, url: &str) -> Result<([usize; 2], Vec<u8>), String> {
    let path = icon_path(dir, id);
    if let Ok(image) = image::open(&path) {
        let image = image.to_rgba8();
        return Ok((
            [image.width() as usize, image.height() as usize],
            image.into_raw(),
        ));
    }
    if !url.starts_with("https://") {
        return Err("No preview image.".into());
    }
    let separator = if url.contains('?') { '&' } else { '?' };
    let sized = format!(
        "{url}{separator}imw={ICON_SIZE}&imh={ICON_SIZE}&ima=fit&impolicy=Letterbox&letterbox=false"
    );
    let mut bytes = Vec::new();
    agent()
        .get(&sized)
        .call()
        .map_err(|e| e.to_string())?
        .into_reader()
        .take(MAX_ICON_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let image = image::load_from_memory(&bytes)
        .map_err(|e| e.to_string())?
        .resize_to_fill(ICON_SIZE, ICON_SIZE, image::imageops::FilterType::Triangle)
        .to_rgba8();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = image.save_with_format(&path, image::ImageFormat::Png);
    Ok((
        [image.width() as usize, image.height() as usize],
        image.into_raw(),
    ))
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else if value >= 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_labels_pick_readable_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(727_710), "711 KB");
        assert_eq!(format_size(15_023_403), "14.3 MB");
        assert_eq!(format_size(9_421_980_810), "8.77 GB");
    }

    #[test]
    fn details_parser_marks_missing_items_unavailable() {
        let value = serde_json::json!({"response": {"publishedfiledetails": [
            {"publishedfileid": "1", "result": 1, "title": "One", "file_size": "2048",
             "preview_url": "https://x/", "time_updated": 5, "subscriptions": 7},
            {"publishedfileid": "2", "result": 9}
        ]}});
        let meta = parse_details(&value);
        assert_eq!(meta["1"].size, 2048);
        assert_eq!(meta["1"].time_updated, 5);
        assert!(meta["1"].available);
        assert!(!meta["2"].available);
    }
}
