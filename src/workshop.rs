//! Steam Workshop metadata (titles, sizes, authors, preview images, required items) cached beside the app.

use serde::{Deserialize, Serialize};

use std::{
    collections::{HashMap, HashSet, VecDeque},
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
    /// SteamID64 of the uploader.
    #[serde(default)]
    pub creator: String,
}

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent(concat!("GModManager/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// Steam's item pages reject ureq's TLS fingerprint with 429 even with browser headers.
/// Windows 10+ ships curl.exe; use that system client for Community HTML.
pub fn community_page(agent: &ureq::Agent, url: &str) -> Result<String, String> {
    const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36";
    #[cfg(windows)]
    {
        let _ = agent;
        let curl =
            PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()))
                .join("System32/curl.exe");
        let mut command = std::process::Command::new(curl);
        use std::os::windows::process::CommandExt;
        // curl.exe is a console program; don't flash a terminal for each author or dependency.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
        let output = command
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--max-time",
                "20",
                "--max-filesize",
                "4194304",
                "--user-agent",
                UA,
                "--header",
                "Accept: text/html,application/xhtml+xml,application/xml",
                "--header",
                "Accept-Language: en-US,en;q=0.9",
                url,
            ])
            .output()
            .map_err(|e| format!("Couldn't open Steam Community: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "Steam Community request failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        String::from_utf8(output.stdout).map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    {
        agent
            .get(url)
            .set("User-Agent", UA)
            .set("Accept", "text/html,application/xhtml+xml,application/xml")
            .set("Accept-Language", "en-US,en;q=0.9")
            .call()
            .map_err(|e| e.to_string())?
            .into_string()
            .map_err(|e| e.to_string())
    }
}
/// IDs listed under "Required items" on a Workshop page.
fn parse_required_items(html: &str) -> Vec<String> {
    const LINK: &str = "filedetails/?id=";
    let Some(start) = html.find("id=\"RequiredItems\"") else {
        return Vec::new();
    };
    let section = &html[start..];
    let end = section
        .find("<!-- created by -->")
        .or_else(|| section.find("rightSectionTopTitle"))
        .unwrap_or(section.len());
    let mut ids = Vec::new();
    let mut rest = &section[..end];
    while let Some(index) = rest.find(LINK) {
        rest = &rest[index + LINK.len()..];
        let length = rest.bytes().take_while(u8::is_ascii_digit).count();
        if length > 0 {
            let id = &rest[..length];
            if !ids.iter().any(|existing| existing == id) {
                ids.push(id.to_owned());
            }
            rest = &rest[length..];
        }
    }
    ids
}

/// Recursively collect required Workshop mods. Failure leaves the preset unchanged.
pub fn required_items(id: &str) -> Result<Vec<String>, String> {
    let agent = agent();
    required_items_with(id, |next| {
        community_page(
            &agent,
            &format!("https://steamcommunity.com/sharedfiles/filedetails/?id={next}"),
        )
    })
}

fn required_items_with(
    id: &str,
    mut fetch: impl FnMut(&str) -> Result<String, String>,
) -> Result<Vec<String>, String> {
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid Workshop ID".into());
    }
    let mut found = Vec::new();
    let mut seen = HashSet::from([id.to_owned()]);
    let mut queue = VecDeque::from([id.to_owned()]);
    while let Some(next) = queue.pop_front() {
        let html =
            fetch(&next).map_err(|e| format!("Couldn't check required mods for {next}: {e}"))?;
        for child in parse_required_items(&html) {
            if seen.insert(child.clone()) {
                if found.len() >= 50 {
                    return Err("This mod requires too many other mods to check.".into());
                }
                queue.push_back(child.clone());
                found.push(child);
            }
        }
    }
    Ok(found)
}

/// Load details for every requirement; reject missing or non-GMod items rather than silently omit them.
pub fn dependency_meta(ids: &[String]) -> Result<HashMap<String, ItemMeta>, String> {
    let mut found = HashMap::new();
    let agent = agent();
    for chunk in ids.chunks(100) {
        let details = fetch_details(&agent, chunk)?;
        let items = details["response"]["publishedfiledetails"]
            .as_array()
            .ok_or("Steam did not return dependency details")?;
        for item in items {
            let id = item["publishedfileid"].as_str().unwrap_or_default();
            if !chunk.iter().any(|requested| requested == id) {
                continue;
            }
            if item["result"].as_i64() != Some(1) || item["consumer_app_id"].as_u64() != Some(4000)
            {
                return Err(format!("Required mod {id} is unavailable for GMod."));
            }
        }
        found.extend(parse_details(&details));
    }
    for id in ids {
        if !found.get(id).is_some_and(|meta| meta.available) {
            return Err(format!("Required mod {id} is unavailable."));
        }
    }
    Ok(found)
}

/// Display name of a Steam user, from their public profile.
pub fn author_name(steam_id: &str) -> Result<String, String> {
    if steam_id.is_empty() || !steam_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Invalid Steam ID".into());
    }
    let xml = community_page(
        &agent(),
        &format!("https://steamcommunity.com/profiles/{steam_id}/?xml=1"),
    )?;
    let start = xml
        .find("<steamID><![CDATA[")
        .ok_or("Profile has no name")?
        + "<steamID><![CDATA[".len();
    let end = xml[start..].find("]]>").ok_or("Profile has no name")?;
    let name = xml[start..start + end].trim();
    if name.is_empty() {
        return Err("Profile has no name".into());
    }
    Ok(name.to_owned())
}

fn authors_path(dir: &Path) -> PathBuf {
    dir.join("cache").join("authors.json")
}

/// SteamID64 → display name.
pub fn load_authors(dir: &Path) -> HashMap<String, String> {
    fs::read(authors_path(dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

pub fn save_authors(dir: &Path, authors: &HashMap<String, String>) -> Result<(), String> {
    let path = authors_path(dir);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(
        &path,
        serde_json::to_vec(authors).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
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
                creator: item["creator"].as_str().unwrap_or_default().to_owned(),
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
             "preview_url": "https://x/", "time_updated": 5, "subscriptions": 7,
             "creator": "76561198027025876"},
            {"publishedfileid": "2", "result": 9}
        ]}});
        let meta = parse_details(&value);
        assert_eq!(meta["1"].size, 2048);
        assert_eq!(meta["1"].creator, "76561198027025876");
        assert_eq!(meta["1"].time_updated, 5);
        assert!(meta["1"].available);
        assert!(!meta["2"].available);
    }

    #[test]
    fn required_items_walks_transitive_links_without_leaking_creator_links_or_cycles() {
        let mut visited = Vec::new();
        let found = required_items_with("10", |id| {
            visited.push(id.to_owned());
            Ok(match id {
                "10" => r#"<div id="RequiredItems"><a href="https://steamcommunity.com/workshop/filedetails/?id=20">Base</a><a href="https://steamcommunity.com/workshop/filedetails/?id=30">Extra</a></div><!-- created by --><a href="https://steamcommunity.com/workshop/filedetails/?id=99">Not required</a>"#,
                "20" => r#"<div id="RequiredItems"><a href="https://steamcommunity.com/workshop/filedetails/?id=10">Cycle</a><a href="https://steamcommunity.com/workshop/filedetails/?id=40">Nested</a></div><!-- created by -->"#,
                _ => "<div>No required items</div>",
            }.to_owned())
        }).unwrap();
        assert_eq!(found, ["20", "30", "40"]);
        assert_eq!(visited, ["10", "20", "30", "40"]);
    }

    #[test]
    fn required_items_reports_fetch_errors_instead_of_incomplete_list() {
        let error = required_items_with("10", |id| {
            if id == "10" {
                Ok(r#"<div id="RequiredItems"><a href="https://steamcommunity.com/workshop/filedetails/?id=20"></a></div>"#.into())
            } else {
                Err("Steam offline".into())
            }
        }).unwrap_err();
        assert!(error.contains("20") && error.contains("Steam offline"));
    }
}
