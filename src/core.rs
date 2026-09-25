use crate::library::{self, Library, Plan, SteamCopy};
use crate::model::{Bind, LocalState, Profile, Setting, WorkshopItem};
use crate::workshop::{self, ItemMeta};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const BUILTIN: &str = include_str!("../presets/lowgan-portal-movement-3.json");
const BEGIN: &str = "// GMM MANAGED BEGIN";
const END: &str = "// GMM MANAGED END";
const MENU_BEGIN: &str = "-- GMM WORKSHOP BEGIN";
const MENU_END: &str = "-- GMM WORKSHOP END";

#[derive(Clone, Debug)]
pub struct WorkshopSearchItem {
    pub id: String,
    pub meta: ItemMeta,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WorkshopStatus {
    pub profile: String,
    pub total: usize,
    pub subscribed: usize,
}

pub fn read_workshop_status(game: &Path) -> Option<WorkshopStatus> {
    let game = normalize_game_path(game)?;
    let status_path = game.join("data/gmm_workshop_status.json");
    let status_modified = fs::metadata(&status_path).ok()?.modified().ok()?;
    let script_modified = fs::metadata(game.join("lua/menu/gmm_workshop.lua"))
        .ok()?
        .modified()
        .ok()?;
    if status_modified < script_modified {
        return None;
    }
    let bytes = fs::read(status_path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn app_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    } else {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

pub fn load_state(dir: &Path) -> LocalState {
    fs::read(dir.join("state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_state(dir: &Path, state: &LocalState) -> Result<(), String> {
    write_json(&dir.join("state.json"), state)
}

pub fn load_profiles(dir: &Path) -> Result<Vec<Profile>, String> {
    let folder = dir.join("presets");
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let built_in_path = folder.join("lowgan-portal-movement-3.json");
    if !built_in_path.exists() {
        fs::write(&built_in_path, BUILTIN).map_err(|e| e.to_string())?;
    }
    let mut profiles = Vec::new();
    for entry in fs::read_dir(folder).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.path().extension().is_some_and(|s| s == "json") {
            let bytes = fs::read(entry.path()).map_err(|e| e.to_string())?;
            let profile: Profile = serde_json::from_slice(&bytes)
                .map_err(|e| format!("{}: {e}", entry.path().display()))?;
            validate_profile(&profile)?;
            profiles.push(profile);
        }
    }
    profiles.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(profiles)
}

pub fn save_profile(dir: &Path, profile: &Profile) -> Result<(), String> {
    validate_profile(profile)?;
    let folder = dir.join("presets");
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    write_json(&folder.join(format!("{}.json", profile.id)), profile)
}

const MAX_PRESET_JSON_BYTES: usize = 16 * 1024 * 1024;
const MAX_PRESET_ARCHIVE_BYTES: u64 = 32 * 1024 * 1024;

pub fn export_preset_zip(profile: &Profile, path: &Path) -> Result<(), String> {
    validate_profile(profile)?;
    let mut json = serde_json::to_vec_pretty(profile).map_err(|e| e.to_string())?;
    json.push(b'\n');
    if json.len() > MAX_PRESET_JSON_BYTES {
        return Err("Preset is too large to share (16 MB limit).".into());
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    writer
        .start_file("preset.json", options)
        .map_err(|e| e.to_string())?;
    writer.write_all(&json).map_err(|e| e.to_string())?;
    writer
        .start_file("README.txt", options)
        .map_err(|e| e.to_string())?;
    writer
        .write_all(b"GMod Manager preset\r\n\r\nOpen GMod Manager and click Import in the sidebar, then pick this ZIP. It holds the whole preset (Workshop IDs, links, binds, settings, and config contents) but not the mods themselves. Press Play with Steam online and GMod downloads them.\r\n")
        .map_err(|e| e.to_string())?;
    let bytes = writer.finish().map_err(|e| e.to_string())?.into_inner();
    replace_file(path, &bytes)
}

pub fn import_preset_file(path: &Path) -> Result<Profile, String> {
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_PRESET_ARCHIVE_BYTES {
        return Err("Preset file is too large (32 MB limit).".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let json = if bytes.starts_with(b"PK\x03\x04") {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let entry = archive
            .by_name("preset.json")
            .map_err(|_| "ZIP is missing preset.json.".to_owned())?;
        if entry.size() > MAX_PRESET_JSON_BYTES as u64 {
            return Err("Preset JSON inside ZIP is too large (16 MB limit).".into());
        }
        let mut json = Vec::new();
        entry
            .take((MAX_PRESET_JSON_BYTES + 1) as u64)
            .read_to_end(&mut json)
            .map_err(|e| e.to_string())?;
        if json.len() > MAX_PRESET_JSON_BYTES {
            return Err("Preset JSON inside ZIP is too large (16 MB limit).".into());
        }
        json
    } else {
        if bytes.len() > MAX_PRESET_JSON_BYTES {
            return Err("Preset JSON is too large (16 MB limit).".into());
        }
        bytes
    };
    let profile: Profile = serde_json::from_slice(&json).map_err(|e| e.to_string())?;
    validate_profile(&profile)?;
    Ok(profile)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn normalize_game_path(input: &Path) -> Option<PathBuf> {
    let direct = input.join("cfg");
    if direct.is_dir() && input.join("addons").is_dir() {
        return fs::canonicalize(input).ok();
    }
    let child = input.join("garrysmod");
    if child.join("cfg").is_dir() && child.join("addons").is_dir() {
        return fs::canonicalize(child).ok();
    }
    None
}

pub fn display_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        rest.to_owned()
    } else {
        text.into_owned()
    }
}

pub fn launch_game(game: &Path) -> Result<(), String> {
    if normalize_game_path(game).is_none() {
        return Err("Choose a valid Garry's Mod folder before launching.".into());
    }
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let steam_path: String = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Software\\Valve\\Steam")
        .and_then(|key| key.get_value("SteamPath"))
        .map_err(|_| "Steam was not found. Install or start Steam first.".to_owned())?;
    let steam = PathBuf::from(steam_path).join("steam.exe");
    if !steam.is_file() {
        return Err(format!(
            "Steam executable was not found: {}",
            steam.display()
        ));
    }
    std::process::Command::new(&steam)
        .args(["-applaunch", "4000"])
        .spawn()
        .map_err(|e| format!("Could not launch Garry's Mod through Steam: {e}"))?;
    Ok(())
}

pub fn detect_game() -> Option<PathBuf> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};
    let root = RegKey::predef(HKEY_CURRENT_USER);
    let steam: String = root
        .open_subkey("Software\\Valve\\Steam")
        .ok()?
        .get_value("SteamPath")
        .ok()?;
    let steam = PathBuf::from(steam);
    let mut libraries = vec![steam.clone()];
    if let Ok(text) = fs::read_to_string(steam.join("steamapps/libraryfolders.vdf")) {
        for line in text.lines() {
            let parts = quoted_parts(line);
            if parts.first().is_some_and(|p| p == "path") && parts.len() >= 2 {
                libraries.push(PathBuf::from(parts[1].replace("\\\\", "\\")));
            }
        }
    }
    for lib in libraries {
        let manifest = lib.join("steamapps/appmanifest_4000.acf");
        if let Ok(text) = fs::read_to_string(manifest) {
            let install = text
                .lines()
                .find_map(|line| {
                    let p = quoted_parts(line);
                    if p.first().is_some_and(|s| s == "installdir") && p.len() >= 2 {
                        Some(p[1].clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "GarrysMod".into());
            if let Some(path) = normalize_game_path(&lib.join("steamapps/common").join(install)) {
                return Some(path);
            }
        }
    }
    None
}

fn quoted_parts(line: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut iter = line.chars();
    while let Some(c) = iter.next() {
        if c == '"' {
            let mut current = String::new();
            for next in iter.by_ref() {
                if next == '"' {
                    break;
                }
                current.push(next);
            }
            result.push(current);
        }
    }
    result
}

pub fn read_game_binds(game: &Path) -> Result<Vec<Bind>, String> {
    let text = fs::read_to_string(game.join("cfg/config.cfg")).map_err(|e| e.to_string())?;
    let mut binds = Vec::new();
    for line in text.lines() {
        let q = quoted_parts(line);
        if line.trim_start().to_ascii_lowercase().starts_with("bind ") && q.len() >= 2 {
            binds.push(Bind {
                key: q[0].clone(),
                command: q[1].clone(),
                sync: valid_key(&q[0]) && safe_value(&q[1]),
            });
        }
    }
    Ok(binds)
}

pub fn capture_selected_settings(game: &Path, settings: &mut [Setting]) -> Result<usize, String> {
    let mut values = HashMap::new();
    for rel in ["cfg/config.cfg", "cfg/autoexec.cfg"] {
        if let Ok(text) = fs::read_to_string(game.join(rel)) {
            for line in text.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with("//") {
                    continue;
                }
                let mut words = line.splitn(2, char::is_whitespace);
                let Some(key) = words.next() else {
                    continue;
                };
                if let Some(value) = words.next() {
                    values.insert(
                        key.to_ascii_lowercase(),
                        value.trim().trim_matches('"').to_owned(),
                    );
                }
            }
        }
    }
    let mut count = 0;
    for setting in settings {
        if setting.sync {
            if let Some(value) = values.get(&setting.name.to_ascii_lowercase()) {
                setting.value = value.clone();
                count += 1;
            }
        }
    }
    Ok(count)
}

pub fn refresh_collection(
    profile: &mut Profile,
) -> Result<(usize, usize, HashMap<String, ItemMeta>), String> {
    let id = profile.workshop.collection_id.clone();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Enter a numeric Steam collection ID first.".into());
    }
    let agent = workshop::agent();
    let url = "https://api.steampowered.com/ISteamRemoteStorage/GetCollectionDetails/v1/";
    let body = agent
        .post(url)
        .send_form(&[("collectioncount", "1"), ("publishedfileids[0]", &id)])
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let collection = &value["response"]["collectiondetails"][0];
    if collection["result"].as_i64() != Some(1) {
        return Err("Steam could not load that collection.".into());
    }
    let ids: Vec<String> = collection["children"]
        .as_array()
        .ok_or("Steam returned no items")?
        .iter()
        .filter(|x| x["filetype"].as_i64() == Some(0))
        .filter_map(|x| x["publishedfileid"].as_str().map(str::to_owned))
        .collect();
    let mut meta = HashMap::new();
    for chunk in ids.chunks(100) {
        meta.extend(workshop::parse_details(&workshop::fetch_details(
            &agent, chunk,
        )?));
    }
    let manual: Vec<_> = profile
        .workshop
        .items
        .iter()
        .filter(|item| item.manually_added)
        .cloned()
        .collect();
    let old: HashSet<_> = profile
        .workshop
        .items
        .iter()
        .filter(|item| !item.manually_added)
        .map(|item| item.id.clone())
        .collect();
    let new: HashSet<_> = ids.iter().cloned().collect();
    let added = new.difference(&old).count();
    let removed = old.difference(&new).count();
    profile.workshop.items = ids
        .into_iter()
        .map(|item_id| {
            let found = meta.get(&item_id);
            WorkshopItem {
                title: found
                    .filter(|m| !m.title.is_empty())
                    .map(|m| m.title.clone()),
                available: found.is_some_and(|m| m.available),
                id: item_id,
                manually_added: false,
            }
        })
        .collect();
    for item in manual {
        if !new.contains(&item.id) {
            profile.workshop.items.push(item);
        }
    }
    profile.source.collection_url =
        format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}");
    Ok((added, removed, meta))
}

fn workshop_ids_from_search_html(html: &str) -> Vec<String> {
    const LINK: &str = "https://steamcommunity.com/sharedfiles/filedetails/?id=";
    let mut rest = html;
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    while let Some(index) = rest.find(LINK) {
        rest = &rest[index + LINK.len()..];
        let length = rest.bytes().take_while(u8::is_ascii_digit).count();
        if length > 0 {
            let id = &rest[..length];
            if seen.insert(id.to_owned()) {
                ids.push(id.to_owned());
                if ids.len() == 30 {
                    break;
                }
            }
        }
        rest = &rest[length..];
    }
    ids
}

pub fn search_workshop(query: &str, page: usize) -> Result<Vec<WorkshopSearchItem>, String> {
    let query = query.trim();
    if query.chars().count() < 2 || query.chars().count() > 100 {
        return Err("Type at least 2 characters.".into());
    }
    if !(1..=100).contains(&page) {
        return Err("Search page is out of range.".into());
    }
    let agent = workshop::agent();
    let direct_id = if query.bytes().all(|b| b.is_ascii_digit()) {
        Some(query.to_owned())
    } else {
        query.split("id=").nth(1).and_then(|tail| {
            let id: String = tail.chars().take_while(char::is_ascii_digit).collect();
            (!id.is_empty()).then_some(id)
        })
    };
    let ids = if let Some(id) = direct_id {
        vec![id]
    } else {
        let html = agent
            .get("https://steamcommunity.com/workshop/browse/")
            .query("appid", "4000")
            .query("searchtext", query)
            .query("browsesort", "textsearch")
            .query("section", "readytouseitems")
            .query("p", &page.to_string())
            .call()
            .map_err(|e| format!("Workshop search failed: {e}"))?
            .into_string()
            .map_err(|e| e.to_string())?;
        workshop_ids_from_search_html(&html)
    };
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let details = workshop::fetch_details(&agent, &ids)?;
    let mut found = workshop::parse_details(&details);
    let apps: HashMap<String, u64> = details["response"]["publishedfiledetails"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    Some((
                        item["publishedfileid"].as_str()?.to_owned(),
                        item["consumer_app_id"].as_u64()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ids
        .into_iter()
        .filter(|id| apps.get(id) == Some(&4000))
        .filter_map(|id| {
            let meta = found.remove(&id)?;
            meta.available.then_some(WorkshopSearchItem { id, meta })
        })
        .collect())
}

pub fn validate_profile(profile: &Profile) -> Result<(), String> {
    if profile.schema_version != 1 {
        return Err("Unsupported profile schema version.".into());
    }
    if profile.id.is_empty()
        || !profile
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err("Profile ID must contain lowercase letters, digits, or hyphens.".into());
    }
    if profile.name.trim().is_empty() {
        return Err("Profile needs a name.".into());
    }
    let mut keys = HashSet::new();
    for bind in &profile.binds {
        if !bind.active() {
            continue;
        }
        if !valid_key(&bind.key) || !safe_value(&bind.command) {
            return Err(if bind.key.trim().is_empty() {
                format!("Bind \"{}\" needs a key.", bind.command)
            } else {
                format!("Bind for {} has an invalid key or command.", bind.key)
            });
        }
        if bind.sync && !keys.insert(bind.key.to_ascii_lowercase()) {
            return Err(format!("Duplicate synced key: {}", bind.key));
        }
    }
    let mut names = HashSet::new();
    for setting in &profile.convars {
        if !setting.active() {
            continue;
        }
        if !valid_name(&setting.name) || !safe_value(&setting.value) {
            return Err(format!(
                "Setting \"{}\" has an invalid name or value.",
                setting.name
            ));
        }
        if setting.sync && !names.insert(setting.name.to_ascii_lowercase()) {
            return Err(format!("Duplicate synced setting: {}", setting.name));
        }
    }
    for item in &profile.workshop.items {
        if item.id.is_empty() || !item.id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("Invalid Workshop ID: {}", item.id));
        }
    }
    let mut paths = HashSet::new();
    for file in &profile.file_mappings {
        if !file.active() {
            continue;
        }
        checked_mapping(&file.path)?;
        if !paths.insert(file.path.to_ascii_lowercase()) {
            return Err(format!("Duplicate synced config path: {}", file.path));
        }
    }
    Ok(())
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b';' && b != b'\\')
}
fn valid_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
fn safe_value(value: &str) -> bool {
    !value.contains(['\r', '\n', '"', '\\', '\0'])
}

fn checked_mapping(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(format!("Unsafe config path: {value}"));
    }
    let mut parts = path.components();
    let root = parts
        .next()
        .and_then(|x| x.as_os_str().to_str())
        .unwrap_or("");
    if !["cfg", "data", "settings"].contains(&root) || parts.next().is_none() {
        return Err(format!(
            "Config path must be under cfg/, data/, or settings/: {value}"
        ));
    }
    let blocked = [
        "cfg/config.cfg",
        "cfg/autoexec.cfg",
        "cfg/addonnomount.txt",
        "cfg/gmm_active.cfg",
        "settings/addonpresets.txt",
    ];
    if blocked.iter().any(|x| x.eq_ignore_ascii_case(value)) {
        return Err(format!("The manager owns this path: {value}"));
    }
    Ok(path.to_path_buf())
}

pub fn read_mapped_file(game: &Path, relative: &str) -> Result<String, String> {
    let game = normalize_game_path(game).ok_or("Choose a valid Garry's Mod folder.")?;
    let relative = checked_mapping(relative)?;
    ensure_inside_game(&game, &relative)?;
    fs::read_to_string(game.join(relative)).map_err(|e| e.to_string())
}

pub fn capture_game_file(game: &Path, selected: &Path) -> Result<(String, String), String> {
    let game = normalize_game_path(game).ok_or("Choose a valid Garry's Mod folder.")?;
    let selected = fs::canonicalize(selected).map_err(|e| e.to_string())?;
    let relative = selected
        .strip_prefix(&game)
        .map_err(|_| "Choose a file inside this Garry's Mod folder.".to_owned())?;
    let relative = relative.to_string_lossy().replace('\\', "/");
    checked_mapping(&relative)?;
    let content = read_mapped_file(&game, &relative)?;
    Ok((relative, content))
}

fn ensure_inside_game(game: &Path, relative: &Path) -> Result<(), String> {
    let root = fs::canonicalize(game).map_err(|e| e.to_string())?;
    let target = game.join(relative);
    let mut parent = game
        .join(relative)
        .parent()
        .ok_or("Config path has no parent")?
        .to_path_buf();
    while !parent.exists() {
        parent = parent
            .parent()
            .ok_or("Config path is outside GMod")?
            .to_path_buf();
    }
    let resolved = fs::canonicalize(parent).map_err(|e| e.to_string())?;
    if !resolved.starts_with(&root) {
        return Err(format!(
            "Config path leaves the GMod folder: {}",
            relative.display()
        ));
    }
    if target.exists() {
        let resolved_target = fs::canonicalize(&target).map_err(|e| e.to_string())?;
        if !resolved_target.starts_with(&root) {
            return Err(format!(
                "Config path leaves the GMod folder: {}",
                relative.display()
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Change {
    pub relative: PathBuf,
    pub before: Option<Vec<u8>>,
    pub after: Vec<u8>,
}

#[derive(Clone, Debug, Default)]
pub struct Preview {
    pub changes: Vec<Change>,
    pub notes: Vec<String>,
}

fn add_change(
    preview: &mut Preview,
    game: &Path,
    relative: &str,
    after: Vec<u8>,
) -> Result<(), String> {
    ensure_inside_game(game, Path::new(relative))?;
    let path = game.join(relative);
    let before = match fs::read(&path) {
        Ok(b) => Some(b),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.to_string()),
    };
    if before.as_deref() != Some(after.as_slice()) {
        preview.changes.push(Change {
            relative: PathBuf::from(relative),
            before,
            after,
        });
    }
    Ok(())
}

fn without_managed_block(source: &str, begin: &str, end_marker: &str) -> Result<String, String> {
    if let Some(start) = source.find(begin) {
        let end_rel = source[start..]
            .find(end_marker)
            .ok_or_else(|| format!("Managed block is missing its end marker: {begin}"))?;
        let end = start + end_rel + end_marker.len();
        let mut result = String::new();
        result.push_str(&source[..start]);
        result.push_str(source[end..].trim_start_matches(['\r', '\n']));
        return Ok(result);
    }
    Ok(source.to_owned())
}

fn lua_list(ids: &[String]) -> String {
    ids.iter().map(|id| format!("  \"{id}\",\n")).collect()
}

/// Stable token for a list so the offload step runs once per plan, not on every launch.
fn list_token(ids: &[String]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in ids.join(",").bytes() {
        hash = (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

const MENU_TAB_JS: &str = include_str!("menu_tab.js");

/// Wraps text in a Lua long string whose delimiter doesn't occur in it.
fn lua_long_string(text: &str) -> String {
    let mut level = 1;
    while text.contains(&format!("]{}]", "=".repeat(level))) {
        level += 1;
    }
    let eq = "=".repeat(level);
    format!("[{eq}[{text}]{eq}]")
}

/// The data shown in the GMod Manager tab of GMod's Addons menu, as a JSON literal.
fn menu_catalog(profile: &Profile, meta: &HashMap<String, ItemMeta>) -> (Vec<String>, String) {
    let mut ids = Vec::new();
    let mut mods = Vec::new();
    let mut total = 0;
    for item in profile.workshop.items.iter().filter(|x| x.available) {
        let info = meta.get(&item.id);
        let size = info.map_or(0, |m| m.size);
        total += size;
        let title = item
            .title
            .clone()
            .filter(|t| !t.is_empty())
            .or_else(|| info.map(|m| m.title.clone()).filter(|t| !t.is_empty()))
            .unwrap_or_else(|| format!("Workshop item {}", item.id));
        let preview = info
            .map(|m| m.preview_url.as_str())
            .filter(|url| url.starts_with("https://"))
            .map(|url| {
                let separator = if url.contains('?') { '&' } else { '?' };
                format!(
                    "{url}{separator}imw=256&imh=256&ima=fit&impolicy=Letterbox&letterbox=false"
                )
            })
            .unwrap_or_default();
        ids.push(item.id.clone());
        mods.push(serde_json::json!({
            "id": item.id,
            "title": title,
            "size": if size > 0 { workshop::format_size(size) } else { String::new() },
            "preview": preview,
        }));
    }
    let catalog = serde_json::json!({
        "preset": profile.name,
        "size": if total > 0 { workshop::format_size(total) } else { String::new() },
        "mods": mods,
    });
    // JSON is valid JavaScript except for these two line separators.
    let json = catalog
        .to_string()
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    (ids, json)
}

fn workshop_menu_script(
    profile: &Profile,
    plan: &Plan,
    meta: &HashMap<String, ItemMeta>,
) -> String {
    let (catalog_ids, catalog) = menu_catalog(profile, meta);
    let script = "-- Generated by GMod Manager.\n\
local ids = {\n__IDS__}\n\
local drop = {\n__DROP__}\n\
local catalogIds = {\n__CATALOG_IDS__}\n\
local catalog = __CATALOG__\n\
local tabScript = __TAB_JS__\n\
local requested = 0\n\
-- Adds the GMod Manager tab to the Addons page of the main menu.\n\
function GMMMenuTab()\n\
  if not IsValid(pnlMainMenu) then return end\n\
  local states = {}\n\
  for i, id in ipairs(catalogIds) do\n\
    if file.Exists(\"addons/gmm_\" .. id .. \"/gmm.json\", \"MOD\") then states[i] = \"local\"\n\
    elseif steamworks and steamworks.IsSubscribed and steamworks.IsSubscribed(id) then states[i] = \"steam\"\n\
    else states[i] = \"missing\" end\n\
  end\n\
  pnlMainMenu:Call(tabScript .. \"\\nGMMInit(\" .. catalog .. \", \" .. util.TableToJSON(states) .. \");\")\n\
end\n\
timer.Simple(1, GMMMenuTab)\n\
timer.Create(\"GMMMenuTabCheck\", 5, 0, function()\n\
  if IsValid(pnlMainMenu) then pnlMainMenu:Call(\"window.GMM || lua.Run('GMMMenuTab()')\") end\n\
end)\n\
local function report()\n\
  local subscribed = 0\n\
  for _, id in ipairs(ids) do\n\
    if steamworks.IsSubscribed(id) then subscribed = subscribed + 1 end\n\
  end\n\
  pcall(function() file.Write(\"gmm_workshop_status.json\", util.TableToJSON({ profile = \"__PROFILE__\", ids = ids, total = #ids, subscribed = subscribed, requested = requested })) end)\n\
end\n\
local function mount()\n\
  for _, id in ipairs(ids) do\n\
    if steamworks.IsSubscribed(id) then steamworks.SetShouldMountAddon(id, true) end\n\
  end\n\
  steamworks.ApplyAddons()\n\
  report()\n\
  GMMMenuTab()\n\
end\n\
timer.Simple(2, function()\n\
  if not steamworks or not steamworks.Subscribe or not steamworks.IsSubscribed then return end\n\
  if #drop > 0 and steamworks.Unsubscribe and file.Read(\"gmm_workshop_drop.txt\", \"DATA\") ~= \"__TOKEN__\" then\n\
    -- These mods are kept compressed in the GMod Manager library.\n\
    for _, id in ipairs(drop) do\n\
      if steamworks.IsSubscribed(id) then steamworks.Unsubscribe(id) end\n\
    end\n\
    file.Write(\"gmm_workshop_drop.txt\", \"__TOKEN__\")\n\
  end\n\
  for _, id in ipairs(ids) do\n\
    if not steamworks.IsSubscribed(id) then\n\
      steamworks.Subscribe(id)\n\
      requested = requested + 1\n\
    end\n\
  end\n\
  mount()\n\
  hook.Add(\"WorkshopSubscriptionsChanged\", \"GMMWorkshopStatus\", function()\n\
    timer.Create(\"GMMWorkshopStatus\", 1, 1, mount)\n\
  end)\n\
  timer.Simple(15, mount)\n\
end)\n";
    // Generated values last, so nothing inside them is mistaken for a placeholder.
    script
        .replace("__TOKEN__", &list_token(&plan.unsubscribe))
        .replace("__PROFILE__", &profile.id)
        .replace("__IDS__", &lua_list(&plan.subscribe))
        .replace("__DROP__", &lua_list(&plan.unsubscribe))
        .replace("__CATALOG_IDS__", &lua_list(&catalog_ids))
        .replace("__TAB_JS__", &lua_long_string(MENU_TAB_JS))
        .replace("__CATALOG__", &lua_long_string(&catalog))
}

/// Everything an apply will do: library work plus the small config file changes.
pub struct Prepared {
    pub plan: Plan,
    pub preview: Preview,
    pub steam: HashMap<String, SteamCopy>,
}

/// Plans library work and builds the config preview for a preset.
/// `library` is `None` when the compressed library is turned off.
pub fn prepare(
    game: &Path,
    profile: &Profile,
    library: Option<&Library>,
    meta: &HashMap<String, ItemMeta>,
) -> Result<Prepared, String> {
    let latest: HashMap<String, u64> = meta
        .iter()
        .filter(|(_, m)| m.time_updated > 0)
        .map(|(id, m)| (id.clone(), m.time_updated))
        .collect();
    let game = normalize_game_path(game).ok_or("Choose a valid Garry's Mod folder.")?;
    let steam = library::scan_steam(&game);
    let plan = if profile.sync.addons {
        let wanted: Vec<String> = profile
            .workshop
            .items
            .iter()
            .filter(|x| x.available)
            .map(|x| x.id.clone())
            .collect();
        let empty = Default::default();
        library::plan(
            &wanted,
            &steam,
            library.map_or(&empty, |l| &l.entries),
            &library::installed_copies(&game),
            &latest,
            library.is_some(),
        )
    } else {
        Plan::default()
    };
    let preview = build_preview(&game, profile, &plan, meta)?;
    Ok(Prepared {
        plan,
        preview,
        steam,
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonChanges {
    #[serde(default)]
    pub installed: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}

/// Runs library work, then writes config files with a backup.
/// `progress` receives a short label and a 0–1 fraction for the current step.
pub fn execute(
    dir: &Path,
    game: &Path,
    profile: &Profile,
    prepared: &Prepared,
    mut library: Option<&mut Library>,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<String, String> {
    let game = normalize_game_path(game).ok_or("Choose a valid Garry's Mod folder.")?;
    if game_is_running(&game) {
        return Err("Close Garry's Mod first.".into());
    }
    check_fresh(&game, &prepared.preview)?;
    let plan = &prepared.plan;
    let title = |id: &str| {
        profile
            .workshop
            .items
            .iter()
            .find(|item| item.id == id)
            .and_then(|item| item.title.clone())
            .unwrap_or_else(|| id.to_owned())
    };
    if let Some(library) = library.as_deref_mut() {
        for (index, id) in plan.store.iter().enumerate() {
            let Some(copy) = prepared.steam.get(id) else {
                continue;
            };
            let name = title(id);
            let label = format!("Compressing {name} ({}/{})", index + 1, plan.store.len());
            let total = copy.size.max(1) as f32;
            library.store(copy, &name, &mut |done| {
                progress(&label, done as f32 / total)
            })?;
        }
    }
    let before = library::installed_copies(&game);
    let mut changes = AddonChanges::default();
    for id in &plan.uninstall {
        progress("Removing inactive mods", 0.0);
        library::uninstall(&game, id)?;
        if before.contains_key(id) {
            changes.removed.push(id.clone());
        }
    }
    if let Some(library) = library.as_deref() {
        for (index, id) in plan.install.iter().enumerate() {
            let label = format!(
                "Unpacking {} ({}/{})",
                title(id),
                index + 1,
                plan.install.len()
            );
            let total = library.entries.get(id).map_or(1, |e| e.raw_size.max(1)) as f32;
            library::install(library, id, &game, &mut |done| {
                progress(&label, done as f32 / total)
            })?;
            if !before.contains_key(id) {
                changes.installed.push(id.clone());
            }
        }
    }
    progress("Writing config", 1.0);
    apply_preview(dir, &game, &prepared.preview, &changes)
}

pub fn build_preview(
    game: &Path,
    profile: &Profile,
    plan: &Plan,
    meta: &HashMap<String, ItemMeta>,
) -> Result<Preview, String> {
    validate_profile(profile)?;
    let game = normalize_game_path(game).ok_or("Choose a valid Garry's Mod folder.")?;
    let mut preview = Preview::default();
    let manage_addons =
        profile.sync.addons && (!profile.workshop.items.is_empty() || !plan.unsubscribe.is_empty());
    if manage_addons {
        let rel = "settings/addonpresets.txt";
        let mut presets: serde_json::Map<String, serde_json::Value> = if game.join(rel).exists() {
            serde_json::from_slice(&fs::read(game.join(rel)).map_err(|e| e.to_string())?)
                .map_err(|e| format!("Existing addon presets could not be parsed: {e}"))?
        } else {
            serde_json::Map::new()
        };
        let title = format!("GMM · {}", profile.name);
        presets.insert(
            title.clone(),
            serde_json::json!({"name":title,"enabled":plan.subscribe,"disabled":[],"newAction":""}),
        );
        let mut output = serde_json::to_vec_pretty(&presets).map_err(|e| e.to_string())?;
        output.push(b'\n');
        add_change(&mut preview, &game, rel, output)?;
        let menu_path = game.join("lua/menu/menu.lua");
        if !menu_path.is_file() {
            return Err(format!(
                "GMod menu script was not found: {}",
                menu_path.display()
            ));
        }
        let current_menu = fs::read_to_string(&menu_path).map_err(|e| e.to_string())?;
        let mut next_menu = without_managed_block(&current_menu, MENU_BEGIN, MENU_END)?;
        // The bridge also draws the GMod Manager tab in GMod's Addons menu, so it's always installed.
        if !next_menu.ends_with('\n') {
            next_menu.push('\n');
        }
        next_menu.push_str(&format!(
            "{MENU_BEGIN}\ninclude( \"gmm_workshop.lua\" )\n{MENU_END}\n"
        ));
        add_change(
            &mut preview,
            &game,
            "lua/menu/gmm_workshop.lua",
            workshop_menu_script(profile, plan, meta).into_bytes(),
        )?;
        if next_menu != current_menu {
            add_change(
                &mut preview,
                &game,
                "lua/menu/menu.lua",
                next_menu.into_bytes(),
            )?;
        }
        let disabled_path = game.join("cfg/addonnomount.txt");
        if let Ok(text) = fs::read_to_string(disabled_path) {
            let wanted: HashSet<_> = profile
                .workshop
                .items
                .iter()
                .filter(|x| x.available)
                .map(|x| x.id.as_str())
                .collect();
            let mut removed = 0;
            let kept: Vec<_> = text
                .lines()
                .filter(|line| {
                    let matched = line
                        .split(|c: char| !c.is_ascii_digit())
                        .any(|token| wanted.contains(token));
                    if matched {
                        removed += 1;
                    }
                    !matched
                })
                .collect();
            if removed > 0 {
                let output = format!(
                    "{}{}",
                    kept.join("\n"),
                    if text.ends_with('\n') && !kept.is_empty() {
                        "\n"
                    } else {
                        ""
                    }
                );
                add_change(
                    &mut preview,
                    &game,
                    "cfg/addonnomount.txt",
                    output.into_bytes(),
                )?;
                preview
                    .notes
                    .push(format!("Turns {removed} disabled mods back on."));
            }
        }
    }
    if !manage_addons {
        let menu_path = game.join("lua/menu/menu.lua");
        if menu_path.is_file() {
            let current_menu = fs::read_to_string(&menu_path).map_err(|e| e.to_string())?;
            let next_menu = without_managed_block(&current_menu, MENU_BEGIN, MENU_END)?;
            if next_menu != current_menu {
                add_change(
                    &mut preview,
                    &game,
                    "lua/menu/menu.lua",
                    next_menu.into_bytes(),
                )?;
            }
        }
    }
    let mut cfg = String::from("// Generated by GMod Manager. Edit the profile in the manager.\n");
    let mut commands = 0;
    if profile.sync.binds {
        for bind in profile.binds.iter().filter(|b| b.active()) {
            cfg.push_str(&format!("bind \"{}\" \"{}\"\n", bind.key, bind.command));
            commands += 1;
        }
    }
    if profile.sync.settings {
        for setting in profile.convars.iter().filter(|s| s.active()) {
            cfg.push_str(&format!("{} \"{}\"\n", setting.name, setting.value));
            commands += 1;
        }
    }
    let autoexec_path = game.join("cfg/autoexec.cfg");
    let current = if autoexec_path.exists() {
        fs::read_to_string(&autoexec_path)
            .map_err(|e| format!("{}: {e}", autoexec_path.display()))?
    } else {
        String::new()
    };
    let mut new_autoexec = without_managed_block(&current, BEGIN, END)?;
    if commands > 0 {
        if !new_autoexec.is_empty() && !new_autoexec.ends_with('\n') {
            new_autoexec.push('\n');
        }
        new_autoexec.push_str(&format!("{BEGIN}\nexec gmm_active.cfg\n{END}\n"));
        add_change(&mut preview, &game, "cfg/gmm_active.cfg", cfg.into_bytes())?;
    }
    if new_autoexec != current {
        add_change(
            &mut preview,
            &game,
            "cfg/autoexec.cfg",
            new_autoexec.into_bytes(),
        )?;
    }
    for file in profile
        .file_mappings
        .iter()
        .filter(|x| x.active() && profile.sync.settings)
    {
        let relative = checked_mapping(&file.path)?;
        add_change(
            &mut preview,
            &game,
            relative.to_str().ok_or("Config path is not UTF-8")?,
            file.content.as_bytes().to_vec(),
        )?;
    }
    if profile.sync.binds && profile.binds.is_empty() {
        preview
            .notes
            .push("Binds are on, but the list is empty.".into());
    }
    if profile.sync.settings && profile.convars.is_empty() && profile.file_mappings.is_empty() {
        preview
            .notes
            .push("Settings are on, but the list is empty.".into());
    }
    Ok(preview)
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupRecord {
    relative: String,
    existed: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    game_path: String,
    files: Vec<BackupRecord>,
    #[serde(default)]
    addons: AddonChanges,
}

fn game_is_running(game: &Path) -> bool {
    use sysinfo::{ProcessesToUpdate, System};
    let install = game.parent().and_then(|path| fs::canonicalize(path).ok());
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.processes().values().any(|p| {
        let game_process = matches!(
            p.name().to_string_lossy().to_ascii_lowercase().as_str(),
            "gmod.exe" | "gmod_x64.exe" | "hl2.exe"
        );
        game_process
            && p.exe()
                .and_then(|path| fs::canonicalize(path).ok())
                .zip(install.as_ref())
                .is_none_or(|(exe, install)| exe.starts_with(install))
    })
}

fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension(format!("gmm-{}.tmp", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let from: Vec<u16> = tmp.as_os_str().encode_wide().chain([0]).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let result = unsafe {
            MoveFileExW(
                from.as_ptr(),
                to.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            let err = io::Error::last_os_error();
            let _ = fs::remove_file(&tmp);
            return Err(format!("{}: {err}", path.display()));
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn check_fresh(game: &Path, preview: &Preview) -> Result<(), String> {
    for change in &preview.changes {
        let current = match fs::read(game.join(&change.relative)) {
            Ok(b) => Some(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        };
        if current != change.before {
            return Err(format!(
                "{} changed in the meantime. Try again.",
                change.relative.display()
            ));
        }
        ensure_inside_game(game, &change.relative)?;
    }
    Ok(())
}

pub fn apply_preview(
    dir: &Path,
    game: &Path,
    preview: &Preview,
    addons: &AddonChanges,
) -> Result<String, String> {
    let game = normalize_game_path(game).ok_or("Choose a valid Garry's Mod folder.")?;
    if game_is_running(&game) {
        return Err("Close Garry's Mod first.".into());
    }
    if preview.changes.is_empty() && addons.installed.is_empty() && addons.removed.is_empty() {
        return Ok("Already up to date.".into());
    }
    check_fresh(&game, preview)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let backup_name = format!("{stamp}");
    let backup = dir.join("backups").join(&backup_name);
    fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
    let mut records = Vec::new();
    for change in &preview.changes {
        let relative = change.relative.to_string_lossy().replace('\\', "/");
        if let Some(before) = &change.before {
            let path = backup.join(&change.relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::write(path, before).map_err(|e| e.to_string())?;
        }
        records.push(BackupRecord {
            relative,
            existed: change.before.is_some(),
        });
    }
    let manifest = BackupManifest {
        game_path: game.to_string_lossy().into_owned(),
        files: records,
        addons: addons.clone(),
    };
    write_json(&backup.join("manifest.json"), &manifest)?;
    let mut written = Vec::new();
    for change in &preview.changes {
        if let Err(e) = replace_file(&game.join(&change.relative), &change.after) {
            for prior in written.into_iter().rev() {
                let prior: &Change = prior;
                let target = game.join(&prior.relative);
                if let Some(before) = &prior.before {
                    let _ = replace_file(&target, before);
                } else {
                    let _ = fs::remove_file(&target);
                }
            }
            return Err(format!(
                "Apply failed and earlier writes were rolled back: {e}"
            ));
        }
        written.push(change);
    }
    fs::write(dir.join("backups/latest.txt"), backup_name).map_err(|e| e.to_string())?;
    Ok("Preset applied.".into())
}

/// Reverts the most recent apply. Mods removed by that apply are unpacked again
/// from the library when it still has them.
pub fn restore_latest(
    dir: &Path,
    game: &Path,
    library: Option<&Library>,
    progress: &mut dyn FnMut(&str, f32),
) -> Result<String, String> {
    let game = normalize_game_path(game).ok_or("Choose a valid Garry's Mod folder.")?;
    if game_is_running(&game) {
        return Err("Close Garry's Mod first.".into());
    }
    let backup_name = fs::read_to_string(dir.join("backups/latest.txt"))
        .map_err(|_| "Nothing to undo.".to_owned())?;
    if backup_name.is_empty() || !backup_name.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Backup pointer is invalid.".into());
    }
    let folder = dir.join("backups").join(backup_name);
    let manifest: BackupManifest =
        serde_json::from_slice(&fs::read(folder.join("manifest.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if manifest.game_path != game.to_string_lossy() {
        return Err("This backup belongs to a different GMod installation.".into());
    }
    for file in &manifest.files {
        let relative = Path::new(&file.relative);
        if relative.is_absolute()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("Backup contains an unsafe path.".into());
        }
        let target = game.join(relative);
        ensure_inside_game(&game, relative)?;
        if file.existed {
            replace_file(
                &target,
                &fs::read(folder.join(relative)).map_err(|e| e.to_string())?,
            )?;
        } else if target.exists() {
            fs::remove_file(target).map_err(|e| e.to_string())?;
        }
    }
    for id in &manifest.addons.installed {
        progress("Removing mods", 0.0);
        library::uninstall(&game, id)?;
    }
    let mut missing = 0;
    for id in &manifest.addons.removed {
        match library.filter(|l| l.entries.contains_key(id)) {
            Some(library) => {
                let total = library.entries[id].raw_size.max(1) as f32;
                library::install(library, id, &game, &mut |done| {
                    progress("Unpacking mods", done as f32 / total)
                })?;
            }
            None => missing += 1,
        }
    }
    fs::remove_file(dir.join("backups/latest.txt")).map_err(|e| e.to_string())?;
    Ok(if missing > 0 {
        format!("Undid the last apply. {missing} mods are no longer in the library.")
    } else {
        "Undid the last apply.".into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_game() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("cfg")).unwrap();
        fs::create_dir_all(temp.path().join("addons")).unwrap();
        fs::create_dir_all(temp.path().join("lua/menu")).unwrap();
        fs::write(
            temp.path().join("lua/menu/menu.lua"),
            "include( \"mainmenu.lua\" )\n",
        )
        .unwrap();
        temp
    }

    #[test]
    fn display_path_hides_windows_verbatim_prefix() {
        assert_eq!(
            display_path(Path::new(r"\\?\C:\Games\GarrysMod\garrysmod")),
            r"C:\Games\GarrysMod\garrysmod"
        );
    }

    #[test]
    fn workshop_search_parser_keeps_unique_result_ids() {
        let html = r#"<a href="https://steamcommunity.com/sharedfiles/filedetails/?id=123">One</a><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=123">One again</a><a href="https://steamcommunity.com/sharedfiles/filedetails/?id=456">Two</a>"#;
        assert_eq!(workshop_ids_from_search_html(html), vec!["123", "456"]);
    }

    #[test]
    #[ignore = "requires the live Steam Workshop"]
    fn workshop_search_smoke() {
        let results = search_workshop("portal", 1).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|item| !item.id.is_empty()));
    }

    #[test]
    fn preset_zip_round_trips_every_field_and_imports_legacy_json() {
        let mut profile = Profile::blank("Test", "test");
        profile.source.collection_url =
            "https://steamcommunity.com/sharedfiles/filedetails/?id=123".into();
        profile.workshop.items.push(WorkshopItem {
            id: "123".into(),
            title: Some("Portal mod".into()),
            available: true,
            manually_added: true,
        });
        profile.binds = vec![
            Bind {
                key: "F".into(),
                command: "+use".into(),
                sync: true,
            },
            Bind {
                key: "G".into(),
                command: "say private".into(),
                sync: false,
            },
        ];
        profile.convars = vec![
            Setting {
                name: "sensitivity".into(),
                value: "2".into(),
                sync: true,
            },
            Setting {
                name: "name".into(),
                value: "private".into(),
                sync: false,
            },
        ];
        profile.file_mappings = vec![
            crate::model::FileMapping {
                path: "cfg/selected.cfg".into(),
                content: "yes".into(),
                sync: true,
            },
            crate::model::FileMapping {
                path: "cfg/private.cfg".into(),
                content: "no".into(),
                sync: false,
            },
        ];
        let temp = tempfile::tempdir().unwrap();
        let zip_path = temp.path().join("test.zip");
        export_preset_zip(&profile, &zip_path).unwrap();
        let restored = import_preset_file(&zip_path).unwrap();
        assert_eq!(
            serde_json::to_value(&restored).unwrap(),
            serde_json::to_value(&profile).unwrap()
        );
        assert_eq!(restored.workshop.items.len(), 1);
        assert_eq!(restored.binds.len(), 2);
        assert_eq!(restored.convars.len(), 2);
        assert_eq!(restored.file_mappings.len(), 2);
        assert!(!restored.sync.binds);
        assert!(!restored.sync.settings);
        let json_path = temp.path().join("old.json");
        fs::write(&json_path, serde_json::to_vec(&profile).unwrap()).unwrap();
        assert_eq!(import_preset_file(&json_path).unwrap().binds.len(), 2);
    }

    #[test]
    fn selective_sync_and_restore() {
        let game = fake_game();
        fs::write(game.path().join("cfg/autoexec.cfg"), "echo original\n").unwrap();
        let mut profile = Profile::blank("Test", "test");
        profile.sync.binds = true;
        profile.sync.settings = true;
        profile.binds = vec![Bind {
            key: "Q".into(),
            command: "+use".into(),
            sync: true,
        }];
        profile.convars = vec![
            Setting {
                name: "fov_desired".into(),
                value: "90".into(),
                sync: false,
            },
            Setting {
                name: "sensitivity".into(),
                value: "2".into(),
                sync: true,
            },
        ];
        let preview = prepare(game.path(), &profile, None, &HashMap::new())
            .unwrap()
            .preview;
        assert_eq!(preview.changes.len(), 2);
        let cfg = preview
            .changes
            .iter()
            .find(|x| x.relative == Path::new("cfg/gmm_active.cfg"))
            .unwrap();
        let output = String::from_utf8_lossy(&cfg.after);
        assert!(output.contains("bind \"Q\" \"+use\""));
        assert!(output.contains("sensitivity \"2\""));
        assert!(!output.contains("fov_desired"));
        let state = tempfile::tempdir().unwrap();
        apply_preview(
            state.path(),
            game.path(),
            &preview,
            &AddonChanges::default(),
        )
        .unwrap();
        assert!(game.path().join("cfg/gmm_active.cfg").exists());
        restore_latest(state.path(), game.path(), None, &mut |_, _| {}).unwrap();
        assert_eq!(
            fs::read_to_string(game.path().join("cfg/autoexec.cfg")).unwrap(),
            "echo original\n"
        );
        assert!(!game.path().join("cfg/gmm_active.cfg").exists());
    }

    #[test]
    fn blank_new_rows_do_not_block_saving_or_reach_config() {
        let game = fake_game();
        let mut profile = Profile::blank("Test", "test");
        profile.sync.binds = true;
        profile.sync.settings = true;
        profile.binds = vec![
            Bind {
                key: String::new(),
                command: String::new(),
                sync: true,
            },
            Bind {
                key: "F".into(),
                command: "+use".into(),
                sync: true,
            },
        ];
        profile.convars.push(Setting {
            name: String::new(),
            value: String::new(),
            sync: true,
        });
        profile.file_mappings.push(crate::model::FileMapping {
            path: String::new(),
            content: String::new(),
            sync: true,
        });
        validate_profile(&profile).unwrap();
        let preview =
            build_preview(game.path(), &profile, &Plan::default(), &HashMap::new()).unwrap();
        let cfg = preview
            .changes
            .iter()
            .find(|x| x.relative == Path::new("cfg/gmm_active.cfg"))
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&cfg.after)
                .lines()
                .skip(1)
                .collect::<Vec<_>>(),
            vec!["bind \"F\" \"+use\""]
        );
        profile.binds[0].command = "+jump".into();
        assert!(
            validate_profile(&profile).is_err(),
            "half-filled rows still validate"
        );
    }

    #[test]
    fn rejects_traversal_and_duplicate_keys() {
        let mut profile = Profile::blank("Test", "test");
        profile.file_mappings.push(crate::model::FileMapping {
            path: "cfg/../x.cfg".into(),
            content: "x".into(),
            sync: true,
        });
        assert!(validate_profile(&profile).is_err());
        profile.file_mappings.clear();
        profile.binds.push(Bind {
            key: "F".into(),
            command: "+use".into(),
            sync: true,
        });
        profile.binds.push(Bind {
            key: "f".into(),
            command: "+attack".into(),
            sync: true,
        });
        assert!(validate_profile(&profile).is_err());
    }

    #[test]
    fn capture_file_only_from_allowed_game_folders() {
        let game = fake_game();
        let allowed = game.path().join("cfg/addon.cfg");
        fs::write(&allowed, "setting 1\n").unwrap();
        let (relative, content) = capture_game_file(game.path(), &allowed).unwrap();
        assert_eq!(relative, "cfg/addon.cfg");
        assert_eq!(content, "setting 1\n");
        let outside = tempfile::NamedTempFile::new().unwrap();
        assert!(capture_game_file(game.path(), outside.path()).is_err());
    }

    #[test]
    fn merges_addon_preset_and_preserves_other_disabled_ids() {
        let game = fake_game();
        fs::create_dir_all(game.path().join("settings")).unwrap();
        fs::write(
            game.path().join("settings/addonpresets.txt"),
            r#"{"My preset":{"name":"My preset","enabled":["777"],"disabled":[],"newAction":""}}"#,
        )
        .unwrap();
        fs::write(
            game.path().join("cfg/addonnomount.txt"),
            "\"123456789\"\n\"888888888\"\n",
        )
        .unwrap();
        let mut profile = Profile::blank("Test", "test");
        profile.workshop.items.push(WorkshopItem {
            id: "123456789".into(),
            title: None,
            available: true,
            manually_added: false,
        });
        profile.workshop.items.push(WorkshopItem {
            id: "999999999".into(),
            title: None,
            available: false,
            manually_added: false,
        });
        let preview = prepare(game.path(), &profile, None, &HashMap::new())
            .unwrap()
            .preview;
        let preset = preview
            .changes
            .iter()
            .find(|x| x.relative == Path::new("settings/addonpresets.txt"))
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&preset.after).unwrap();
        assert_eq!(json["My preset"]["enabled"][0], "777");
        assert_eq!(json["GMM · Test"]["enabled"][0], "123456789");
        assert_eq!(json["GMM · Test"]["enabled"].as_array().unwrap().len(), 1);
        let disabled = preview
            .changes
            .iter()
            .find(|x| x.relative == Path::new("cfg/addonnomount.txt"))
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&disabled.after), "\"888888888\"\n");
        let menu = preview
            .changes
            .iter()
            .find(|x| x.relative == Path::new("lua/menu/menu.lua"))
            .unwrap();
        assert!(String::from_utf8_lossy(&menu.after).contains("include( \"gmm_workshop.lua\" )"));
        let script = preview
            .changes
            .iter()
            .find(|x| x.relative == Path::new("lua/menu/gmm_workshop.lua"))
            .unwrap();
        let script = String::from_utf8_lossy(&script.after);
        assert!(script.contains("steamworks.Subscribe(id)"));
        assert!(script.contains("123456789"));
        assert!(!script.contains("999999999"));
    }

    #[test]
    fn menu_tab_catalog_survives_awkward_titles() {
        let mut profile = Profile::blank("Test", "test");
        let title = "Evil ]]\"title\" ]=] \u{2028} </script>";
        profile.workshop.items = vec![
            WorkshopItem {
                id: "1".into(),
                title: Some(title.into()),
                available: true,
                manually_added: false,
            },
            WorkshopItem {
                id: "2".into(),
                title: None,
                available: false,
                manually_added: false,
            },
        ];
        let meta: HashMap<String, ItemMeta> = [(
            "1".to_string(),
            ItemMeta {
                size: 15_023_403,
                preview_url: "https://images.example/x/".into(),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect();
        let script = workshop_menu_script(&profile, &Plan::default(), &meta);
        let start = script.find("local catalog = [").unwrap() + "local catalog = ".len();
        let level = script[start + 1..]
            .bytes()
            .take_while(|b| *b == b'=')
            .count();
        let open = format!("[{}[", "=".repeat(level));
        let close = format!("]{}]", "=".repeat(level));
        let body_start = start + open.len();
        let body_end = body_start + script[body_start..].find(&close).unwrap();
        let json: serde_json::Value = serde_json::from_str(&script[body_start..body_end]).unwrap();
        assert_eq!(
            json["mods"].as_array().unwrap().len(),
            1,
            "unavailable mods are hidden"
        );
        assert_eq!(json["mods"][0]["title"], title);
        assert_eq!(json["mods"][0]["size"], "14.3 MB");
        assert!(
            json["mods"][0]["preview"]
                .as_str()
                .unwrap()
                .starts_with("https://images.example/x/?imw=")
        );
        assert!(!script[body_start..body_end].contains('\u{2028}'));
        assert!(script.contains("local catalogIds = {\n  \"1\",\n}"));
    }

    #[test]
    fn turning_off_addons_removes_the_subscription_bridge() {
        let game = fake_game();
        fs::write(game.path().join("lua/menu/menu.lua"), format!("include( \"mainmenu.lua\" )\n{MENU_BEGIN}\ninclude( \"gmm_workshop.lua\" )\n{MENU_END}\n")).unwrap();
        let mut profile = Profile::blank("Test", "test");
        profile.sync.addons = false;
        let preview =
            build_preview(game.path(), &profile, &Plan::default(), &HashMap::new()).unwrap();
        let menu = preview
            .changes
            .iter()
            .find(|change| change.relative == Path::new("lua/menu/menu.lua"))
            .unwrap();
        let text = String::from_utf8_lossy(&menu.after);
        assert_eq!(text, "include( \"mainmenu.lua\" )\n");
    }

    #[test]
    fn stale_preview_does_not_overwrite_newer_game_config() {
        let game = fake_game();
        fs::write(game.path().join("cfg/autoexec.cfg"), "echo original\n").unwrap();
        let mut profile = Profile::blank("Test", "test");
        profile.sync.binds = true;
        profile.binds.push(Bind {
            key: "F".into(),
            command: "+use".into(),
            sync: true,
        });
        let preview = prepare(game.path(), &profile, None, &HashMap::new())
            .unwrap()
            .preview;
        fs::write(game.path().join("cfg/autoexec.cfg"), "echo newer\n").unwrap();
        let state = tempfile::tempdir().unwrap();
        assert!(
            apply_preview(
                state.path(),
                game.path(),
                &preview,
                &AddonChanges::default()
            )
            .is_err()
        );
        assert_eq!(
            fs::read_to_string(game.path().join("cfg/autoexec.cfg")).unwrap(),
            "echo newer\n"
        );
    }

    #[test]
    fn library_mods_unpack_offload_and_undo() {
        let root = tempfile::tempdir().unwrap();
        let game = root.path().join("steamapps/common/GarrysMod/garrysmod");
        for dir in ["cfg", "addons", "lua/menu"] {
            fs::create_dir_all(game.join(dir)).unwrap();
        }
        fs::write(
            game.join("lua/menu/menu.lua"),
            "include( \"mainmenu.lua\" )\n",
        )
        .unwrap();
        let workshop_dir = root.path().join("steamapps/workshop");
        let addon = library::test_gma(&[("lua/autorun/a.lua", b"print(1)")]);
        for id in ["111", "222"] {
            fs::create_dir_all(workshop_dir.join("content/4000").join(id)).unwrap();
            fs::write(
                workshop_dir.join("content/4000").join(id).join("a.gma"),
                &addon,
            )
            .unwrap();
        }
        fs::write(
            workshop_dir.join("appworkshop_4000.acf"),
            "\"AppWorkshop\"\n{\n\"WorkshopItemsInstalled\"\n{\n\"111\"\n{\n\"timeupdated\" \"5\"\n}\n\"222\"\n{\n\"timeupdated\" \"5\"\n}\n}\n}\n",
        )
        .unwrap();
        let mut library = Library::open(&root.path().join("library")).unwrap();
        let steam = library::scan_steam(&game);
        library.store(&steam["222"], "Other", &mut |_| {}).unwrap();
        let loose = root.path().join("333.gma");
        fs::write(&loose, &addon).unwrap();
        let offline = SteamCopy {
            id: "333".into(),
            path: loose,
            legacy: false,
            size: addon.len() as u64,
            time_updated: 7,
        };
        library.store(&offline, "Offline", &mut |_| {}).unwrap();
        let item = |id: &str| WorkshopItem {
            id: id.into(),
            title: None,
            available: true,
            manually_added: false,
        };
        let mut profile = Profile::blank("Test", "test");
        profile.workshop.items = vec![item("111"), item("333")];
        let state = tempfile::tempdir().unwrap();

        let prepared = prepare(&game, &profile, Some(&library), &HashMap::new()).unwrap();
        assert_eq!(prepared.plan.store, vec!["111"]);
        assert_eq!(prepared.plan.install, vec!["333"]);
        assert_eq!(prepared.plan.subscribe, vec!["111"]);
        assert_eq!(prepared.plan.unsubscribe, vec!["222"]);
        execute(
            state.path(),
            &game,
            &profile,
            &prepared,
            Some(&mut library),
            &mut |_, _| {},
        )
        .unwrap();
        assert!(library.entries.contains_key("111"));
        assert_eq!(
            fs::read(game.join("addons/gmm_333/lua/autorun/a.lua")).unwrap(),
            b"print(1)"
        );
        let script = fs::read_to_string(game.join("lua/menu/gmm_workshop.lua")).unwrap();
        assert!(script.contains("local ids = {\n  \"111\",\n}"));
        assert!(script.contains("local drop = {\n  \"222\",\n}"));

        profile.workshop.items = vec![item("111")];
        let prepared = prepare(&game, &profile, Some(&library), &HashMap::new()).unwrap();
        assert_eq!(prepared.plan.uninstall, vec!["333"]);
        execute(
            state.path(),
            &game,
            &profile,
            &prepared,
            Some(&mut library),
            &mut |_, _| {},
        )
        .unwrap();
        assert!(!game.join("addons/gmm_333").exists());
        restore_latest(state.path(), &game, Some(&library), &mut |_, _| {}).unwrap();
        assert!(game.join("addons/gmm_333/lua/autorun/a.lua").exists());

        profile.workshop.items.clear();
        let prepared = prepare(&game, &profile, Some(&library), &HashMap::new()).unwrap();
        assert_eq!(prepared.plan.unsubscribe, vec!["111", "222"]);
        let script = prepared
            .preview
            .changes
            .iter()
            .find(|c| c.relative == Path::new("lua/menu/gmm_workshop.lua"))
            .expect("an empty preset still needs the bridge to drop stored mods");
        assert!(
            String::from_utf8_lossy(&script.after)
                .contains("local drop = {\n  \"111\",\n  \"222\",\n}")
        );
    }
}
