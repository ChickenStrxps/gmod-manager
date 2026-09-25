//! Compressed mod library.
//!
//! Steam keeps every subscribed addon at full size. The library keeps one zstd-compressed
//! copy of each addon Steam has already downloaded, so switching presets never needs a
//! second download. Mods from the active preset that Steam no longer holds are unpacked
//! into `garrysmod/addons/gmm_<id>/`, a folder addon GMod mounts on startup.

use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Component, Path, PathBuf},
};

const ZSTD_LEVEL: i32 = 7;
const SMALL_ZSTD_LEVEL: i32 = 19;
const SMALL_GMA_MAX_BYTES: u64 = 64 * 1024 * 1024;
const MARKER: &str = "gmm.json";

#[derive(Clone, Debug)]
pub struct SteamCopy {
    pub id: String,
    pub path: PathBuf,
    /// `*_legacy.bin`: an LZMA-compressed GMA.
    pub legacy: bool,
    pub size: u64,
    pub time_updated: u64,
}

fn steamapps(game: &Path) -> Option<&Path> {
    game.parent()?.parent()?.parent()
}

fn numeric(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit())
}

fn quoted(line: &str) -> Vec<&str> {
    line.split('"').skip(1).step_by(2).collect()
}

/// Reads `timeupdated` for each installed item from `appworkshop_4000.acf`.
fn acf_times(text: &str) -> HashMap<String, u64> {
    let mut times = HashMap::new();
    let mut depth = 0usize;
    let mut in_installed = false;
    let mut installed_depth = 0usize;
    let mut current: Option<String> = None;
    let mut pending_key: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed == "{" {
            depth += 1;
            if let Some(key) = pending_key.take() {
                if key == "WorkshopItemsInstalled" {
                    in_installed = true;
                    installed_depth = depth;
                } else if in_installed && depth == installed_depth + 1 {
                    current = Some(key);
                }
            }
            continue;
        }
        if trimmed == "}" {
            if in_installed && depth == installed_depth + 1 {
                current = None;
            }
            if in_installed && depth == installed_depth {
                in_installed = false;
            }
            depth = depth.saturating_sub(1);
            continue;
        }
        let parts = quoted(trimmed);
        match parts.as_slice() {
            [key] => pending_key = Some((*key).to_owned()),
            [key, value] => {
                if let Some(id) = &current {
                    if key.eq_ignore_ascii_case("timeupdated") {
                        if let Ok(time) = value.parse() {
                            times.insert(id.clone(), time);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    times
}

/// Lists Workshop items Steam has downloaded for Garry's Mod.
pub fn scan_steam(game: &Path) -> HashMap<String, SteamCopy> {
    let mut copies = HashMap::new();
    let Some(steamapps) = steamapps(game) else {
        return copies;
    };
    let workshop = steamapps.join("workshop");
    let times = fs::read_to_string(workshop.join("appworkshop_4000.acf"))
        .map(|text| acf_times(&text))
        .unwrap_or_default();
    let Ok(entries) = fs::read_dir(workshop.join("content").join("4000")) else {
        return copies;
    };
    for entry in entries.flatten() {
        let id = entry.file_name().to_string_lossy().into_owned();
        if !numeric(&id) || !entry.path().is_dir() {
            continue;
        }
        let Ok(files) = fs::read_dir(entry.path()) else {
            continue;
        };
        let mut best: Option<(PathBuf, bool, u64)> = None;
        for file in files.flatten() {
            let name = file.file_name().to_string_lossy().to_ascii_lowercase();
            let legacy = name.ends_with("_legacy.bin");
            if !legacy && !name.ends_with(".gma") {
                continue;
            }
            let Ok(meta) = file.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|(_, was_legacy, _)| *was_legacy && !legacy)
            {
                best = Some((file.path(), legacy, meta.len()));
            }
        }
        if let Some((path, legacy, size)) = best {
            copies.insert(
                id.clone(),
                SteamCopy {
                    time_updated: times.get(&id).copied().unwrap_or(0),
                    id,
                    path,
                    legacy,
                    size,
                },
            );
        }
    }
    copies
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryEntry {
    pub title: String,
    pub time_updated: u64,
    pub raw_size: u64,
    pub stored_size: u64,
    /// Old index files default to false, so they can be optimized on demand.
    #[serde(default)]
    pub optimized: bool,
}

pub fn can_optimize(entry: &LibraryEntry) -> bool {
    !entry.optimized && entry.raw_size > 0 && entry.raw_size <= SMALL_GMA_MAX_BYTES
}
pub struct Library {
    pub root: PathBuf,
    pub entries: BTreeMap<String, LibraryEntry>,
}

/// Counts bytes flowing through a reader and reports them.
struct Counting<'a, R> {
    inner: R,
    count: u64,
    progress: &'a mut dyn FnMut(u64),
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.count += n as u64;
        (self.progress)(self.count);
        Ok(n)
    }
}

/// Counts bytes written and checks that the stream is a GMA.
struct GmaCheck<W> {
    inner: W,
    count: u64,
    head: Vec<u8>,
}

impl<W: Write> Write for GmaCheck<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.head.len() < 4 {
            let need = 4 - self.head.len();
            self.head.extend_from_slice(&buf[..need.min(buf.len())]);
            if self.head.len() == 4 && self.head != b"GMAD" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "not a GMA addon file",
                ));
            }
        }
        let n = self.inner.write(buf)?;
        self.count += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn encoder<W: Write>(out: W, level: i32) -> Result<zstd::Encoder<'static, W>, String> {
    let mut encoder = zstd::Encoder::new(out, level).map_err(|e| e.to_string())?;
    let workers = std::thread::available_parallelism()
        .map(|n| n.get().min(4) as u32)
        .unwrap_or(1);
    if workers > 1 {
        encoder.multithread(workers).map_err(|e| e.to_string())?;
    }
    Ok(encoder)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        e.to_string()
    })
}

/// Reads the library index without creating anything on disk.
pub fn read_entries(root: &Path) -> BTreeMap<String, LibraryEntry> {
    let mut entries: BTreeMap<String, LibraryEntry> = fs::read(root.join("index.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    entries.retain(|id, _| numeric(id) && root.join(format!("{id}.gma.zst")).is_file());
    entries
}

impl Library {
    pub fn open(root: &Path) -> Result<Library, String> {
        fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
        Ok(Library {
            root: root.to_path_buf(),
            entries: read_entries(root),
        })
    }

    /// Moves every stored mod to `target`, copying when a rename crosses drives.
    pub fn move_to(
        &mut self,
        target: &Path,
        progress: &mut dyn FnMut(usize),
    ) -> Result<(), String> {
        let same = |a: &Path, b: &Path| {
            fs::canonicalize(a)
                .ok()
                .zip(fs::canonicalize(b).ok())
                .is_some_and(|(a, b)| a == b)
        };
        if same(target, &self.root) {
            return Ok(());
        }
        fs::create_dir_all(target).map_err(|e| format!("{}: {e}", target.display()))?;
        let old_root = std::mem::replace(&mut self.root, target.to_path_buf());
        // The index is written first; each root only lists files it actually holds,
        // so an interrupted move leaves both folders readable.
        self.save_index()?;
        let ids: Vec<String> = self.entries.keys().cloned().collect();
        for (index, id) in ids.iter().enumerate() {
            let from = old_root.join(format!("{id}.gma.zst"));
            let to = self.file(id);
            if fs::rename(&from, &to).is_err() {
                let tmp = to.with_extension("zst.tmp");
                fs::copy(&from, &tmp).map_err(|e| e.to_string())?;
                fs::rename(&tmp, &to).map_err(|e| e.to_string())?;
                fs::remove_file(&from).map_err(|e| e.to_string())?;
            }
            progress(index + 1);
        }
        let _ = fs::remove_file(old_root.join("index.json"));
        let _ = fs::remove_dir(&old_root);
        Ok(())
    }

    pub fn file(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.gma.zst"))
    }

    fn save_index(&self) -> Result<(), String> {
        let bytes = serde_json::to_vec_pretty(&self.entries).map_err(|e| e.to_string())?;
        write_atomic(&self.root.join("index.json"), &bytes)
    }

    pub fn store(
        &mut self,
        copy: &SteamCopy,
        title: &str,
        progress: &mut dyn FnMut(u64),
    ) -> Result<(), String> {
        if !numeric(&copy.id) {
            return Err(format!("Invalid Workshop ID: {}", copy.id));
        }
        let optimized = !copy.legacy && copy.size <= SMALL_GMA_MAX_BYTES;
        let target = self.file(&copy.id);
        let tmp = target.with_extension("zst.tmp");
        let result = (|| -> Result<(u64, u64), String> {
            let source =
                fs::File::open(&copy.path).map_err(|e| format!("{}: {e}", copy.path.display()))?;
            let mut reader = BufReader::new(Counting {
                inner: source,
                count: 0,
                progress,
            });
            let out = BufWriter::new(fs::File::create(&tmp).map_err(|e| e.to_string())?);
            let level = if optimized {
                SMALL_ZSTD_LEVEL
            } else {
                ZSTD_LEVEL
            };
            let mut check = GmaCheck {
                inner: encoder(out, level)?,
                count: 0,
                head: Vec::with_capacity(4),
            };
            if copy.legacy {
                lzma_rs::lzma_decompress(&mut reader, &mut check)
                    .map_err(|e| format!("Could not unpack {}: {e}", copy.path.display()))?;
            } else {
                io::copy(&mut reader, &mut check).map_err(|e| e.to_string())?;
            }
            if check.head != b"GMAD" {
                return Err("Steam file is not a GMA addon.".into());
            }
            let raw = check.count;
            let mut out = check.inner.finish().map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
            out.into_inner()
                .map_err(|e| e.to_string())?
                .sync_all()
                .map_err(|e| e.to_string())?;
            let stored = fs::metadata(&tmp).map_err(|e| e.to_string())?.len();
            Ok((raw, stored))
        })();
        let (raw_size, stored_size) = match result {
            Ok(sizes) => sizes,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        };
        if let Err(e) = fs::rename(&tmp, &target) {
            let _ = fs::remove_file(&tmp);
            return Err(e.to_string());
        }
        self.entries.insert(
            copy.id.clone(),
            LibraryEntry {
                title: title.to_owned(),
                time_updated: copy.time_updated,
                raw_size,
                stored_size,
                optimized,
            },
        );
        self.save_index()
    }
    /// Recompress an older library copy without needing the Steam download. Keep the original
    /// until a complete replacement is smaller; interrupted runs leave it readable.
    pub fn optimize(&mut self, id: &str, progress: &mut dyn FnMut(u64)) -> Result<u64, String> {
        let entry = self.entries.get(id).ok_or("Mod is not in the library.")?;
        if !can_optimize(entry) {
            return Ok(0);
        }
        let raw_size = entry.raw_size;
        let target = self.file(id);
        let previous = fs::metadata(&target).map_err(|e| e.to_string())?.len();
        let tmp = target.with_extension("zst.tmp");
        let result = (|| -> Result<u64, String> {
            let file = fs::File::open(&target).map_err(|e| e.to_string())?;
            let decoder = zstd::Decoder::new(file).map_err(|e| e.to_string())?;
            let out = BufWriter::new(fs::File::create(&tmp).map_err(|e| e.to_string())?);
            let mut encoder = encoder(out, SMALL_ZSTD_LEVEL)?;
            let mut reader = Counting {
                inner: decoder,
                count: 0,
                progress,
            };
            io::copy(&mut reader, &mut encoder).map_err(|e| e.to_string())?;
            if reader.count != raw_size {
                return Err("Library copy changed size while optimizing.".into());
            }
            let mut out = encoder.finish().map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
            out.into_inner()
                .map_err(|e| e.to_string())?
                .sync_all()
                .map_err(|e| e.to_string())?;
            Ok(fs::metadata(&tmp).map_err(|e| e.to_string())?.len())
        })();
        let stored = match result {
            Ok(stored) => stored,
            Err(error) => {
                let _ = fs::remove_file(&tmp);
                return Err(error);
            }
        };
        if stored < previous {
            if let Err(error) = fs::rename(&tmp, &target) {
                let _ = fs::remove_file(&tmp);
                return Err(error.to_string());
            }
        } else {
            fs::remove_file(&tmp).map_err(|e| e.to_string())?;
        }
        let entry = self.entries.get_mut(id).expect("Checked above");
        entry.stored_size = stored.min(previous);
        entry.optimized = true;
        self.save_index()?;
        Ok(previous.saturating_sub(stored))
    }

    pub fn remove(&mut self, id: &str) -> Result<(), String> {
        if self.entries.remove(id).is_some() {
            match fs::remove_file(self.file(id)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
            self.save_index()?;
        }
        Ok(())
    }
}

pub fn folder_name(id: &str) -> String {
    format!("gmm_{id}")
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Marker {
    id: String,
    #[serde(default)]
    title: String,
    time_updated: u64,
}

fn read_marker(folder: &Path) -> Option<Marker> {
    serde_json::from_slice(&fs::read(folder.join(MARKER)).ok()?).ok()
}

/// Library mods currently unpacked into `garrysmod/addons`, with their version.
pub fn installed_copies(game: &Path) -> HashMap<String, u64> {
    let mut found = HashMap::new();
    let Ok(entries) = fs::read_dir(game.join("addons")) else {
        return found;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = name.strip_prefix("gmm_") else {
            continue;
        };
        if !numeric(id) {
            continue;
        }
        if let Some(marker) = read_marker(&entry.path()) {
            if marker.id == id {
                found.insert(marker.id, marker.time_updated);
            }
        }
    }
    found
}

fn read_exact_array<const N: usize>(reader: &mut impl Read) -> Result<[u8; N], String> {
    let mut bytes = [0u8; N];
    reader
        .read_exact(&mut bytes)
        .map_err(|_| "Addon file ended early.".to_owned())?;
    Ok(bytes)
}

fn read_cstr(reader: &mut impl Read) -> Result<String, String> {
    let mut bytes = Vec::new();
    loop {
        let [b] = read_exact_array::<1>(reader)?;
        if b == 0 {
            break;
        }
        bytes.push(b);
        if bytes.len() > 64 * 1024 {
            return Err("Addon header is corrupt.".into());
        }
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Validates a path stored inside a GMA and converts it to a relative path.
fn safe_entry(name: &str) -> Result<PathBuf, String> {
    let bad = || format!("Addon contains an unsafe path: {name}");
    if name.is_empty() || name.contains(['\\', ':', '\0']) || name.starts_with('/') {
        return Err(bad());
    }
    let mut path = PathBuf::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(bad());
        }
        path.push(part);
    }
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err(bad());
    }
    if path.as_os_str().eq_ignore_ascii_case(MARKER) {
        return Err(bad());
    }
    Ok(path)
}

fn extract_gma(
    reader: &mut impl Read,
    target: &Path,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, String> {
    if &read_exact_array::<4>(reader)? != b"GMAD" {
        return Err("Library copy is not a GMA addon.".into());
    }
    let [version] = read_exact_array::<1>(reader)?;
    if version > 3 {
        return Err(format!("Unsupported GMA version {version}."));
    }
    read_exact_array::<16>(reader)?; // steamid + timestamp
    if version > 1 {
        while !read_cstr(reader)?.is_empty() {}
    }
    for _ in 0..3 {
        read_cstr(reader)?; // name, description, author
    }
    read_exact_array::<4>(reader)?; // addon version
    let mut index = Vec::new();
    loop {
        if u32::from_le_bytes(read_exact_array::<4>(reader)?) == 0 {
            break;
        }
        let name = read_cstr(reader)?;
        let size = i64::from_le_bytes(read_exact_array::<8>(reader)?);
        read_exact_array::<4>(reader)?; // crc
        if size < 0 {
            return Err("Addon index is corrupt.".into());
        }
        index.push((safe_entry(&name)?, size as u64));
    }
    let mut written = 0u64;
    let mut seen = HashSet::new();
    for (relative, size) in index {
        let path = target.join(&relative);
        if !seen.insert(relative.to_string_lossy().to_ascii_lowercase()) {
            // Duplicate entry: later data wins, matching gmad's behaviour.
            let _ = fs::remove_file(&path);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = BufWriter::new(
            fs::File::create(&path).map_err(|e| format!("{}: {e}", relative.display()))?,
        );
        let copied =
            io::copy(&mut reader.by_ref().take(size), &mut out).map_err(|e| e.to_string())?;
        if copied != size {
            return Err("Addon file ended early.".into());
        }
        out.flush().map_err(|e| e.to_string())?;
        written += size;
        progress(written);
    }
    Ok(written)
}

/// Unpacks a library mod into `garrysmod/addons/gmm_<id>/`.
pub fn install(
    library: &Library,
    id: &str,
    game: &Path,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, String> {
    let entry = library
        .entries
        .get(id)
        .ok_or_else(|| format!("{id} is not in the library."))?;
    let addons = game.join("addons");
    fs::create_dir_all(&addons).map_err(|e| e.to_string())?;
    let target = addons.join(folder_name(id));
    let tmp = addons.join(format!("{}.tmp", folder_name(id)));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).map_err(|e| e.to_string())?;
    }
    let result = (|| -> Result<u64, String> {
        fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let file = fs::File::open(library.file(id)).map_err(|e| e.to_string())?;
        let mut decoder = zstd::Decoder::new(file).map_err(|e| e.to_string())?;
        let written = extract_gma(&mut decoder, &tmp, progress)?;
        let marker = Marker {
            id: id.to_owned(),
            title: entry.title.clone(),
            time_updated: entry.time_updated,
        };
        fs::write(
            tmp.join(MARKER),
            serde_json::to_vec_pretty(&marker).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        Ok(written)
    })();
    let written = match result {
        Ok(written) => written,
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
    };
    if target.exists() {
        if let Err(e) = uninstall(game, id) {
            let _ = fs::remove_dir_all(&tmp);
            return Err(e);
        }
    }
    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_dir_all(&tmp);
        e.to_string()
    })?;
    Ok(written)
}

/// Removes an unpacked library mod. Folders without a matching marker are left alone.
pub fn uninstall(game: &Path, id: &str) -> Result<(), String> {
    let folder = game.join("addons").join(folder_name(id));
    if !folder.exists() {
        return Ok(());
    }
    match read_marker(&folder) {
        Some(marker) if marker.id == id => {
            fs::remove_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))
        }
        _ => Err(format!(
            "{} was not created by GMod Manager; leaving it in place.",
            folder.display()
        )),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Steam copies to compress into the library.
    pub store: Vec<String>,
    /// Library mods to unpack into `garrysmod/addons`.
    pub install: Vec<String>,
    /// Unpacked library mods to remove.
    pub uninstall: Vec<String>,
    /// Items Steam should keep or download.
    pub subscribe: Vec<String>,
    /// Items safely stored in the library that Steam no longer needs to keep.
    pub unsubscribe: Vec<String>,
}

pub fn plan(
    wanted: &[String],
    steam: &HashMap<String, SteamCopy>,
    library: &BTreeMap<String, LibraryEntry>,
    installed: &HashMap<String, u64>,
    latest: &HashMap<String, u64>,
    enabled: bool,
) -> Plan {
    let mut seen = HashSet::new();
    let wanted: Vec<&String> = wanted
        .iter()
        .filter(|id| seen.insert(id.as_str()))
        .collect();
    let mut plan = Plan::default();
    let mut sorted_installed: Vec<&String> = installed.keys().collect();
    sorted_installed.sort();
    if !enabled {
        plan.subscribe = wanted.into_iter().cloned().collect();
        plan.uninstall = sorted_installed.into_iter().cloned().collect();
        return plan;
    }
    for id in &wanted {
        let stored = library.get(id.as_str());
        if let Some(copy) = steam.get(id.as_str()) {
            plan.subscribe.push((*id).clone());
            if stored.is_none_or(|entry| entry.time_updated < copy.time_updated) {
                plan.store.push((*id).clone());
            }
            if installed.contains_key(id.as_str()) {
                plan.uninstall.push((*id).clone());
            }
        } else if let Some(entry) = stored.filter(|entry| {
            latest
                .get(id.as_str())
                .is_none_or(|newest| entry.time_updated >= *newest)
        }) {
            if installed.get(id.as_str()) != Some(&entry.time_updated) {
                plan.install.push((*id).clone());
            }
        } else {
            plan.subscribe.push((*id).clone());
            if installed.contains_key(id.as_str()) {
                plan.uninstall.push((*id).clone());
            }
        }
    }
    let wanted: HashSet<&str> = wanted.iter().map(|id| id.as_str()).collect();
    for id in sorted_installed {
        if !wanted.contains(id.as_str()) {
            plan.uninstall.push(id.clone());
        }
    }
    let mut unsubscribe: Vec<String> = steam
        .values()
        .filter(|copy| !wanted.contains(copy.id.as_str()))
        .filter(|copy| {
            library
                .get(&copy.id)
                .is_some_and(|entry| entry.time_updated >= copy.time_updated)
        })
        .map(|copy| copy.id.clone())
        .collect();
    unsubscribe.sort();
    plan.unsubscribe = unsubscribe;
    plan
}

/// Builds a minimal GMA for tests.
#[cfg(test)]
pub(crate) fn test_gma(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = b"GMAD".to_vec();
    out.push(3);
    out.extend_from_slice(&[0; 16]);
    out.push(0); // no required content
    for text in ["Test addon", "{}", "Author"] {
        out.extend_from_slice(text.as_bytes());
        out.push(0);
    }
    out.extend_from_slice(&1i32.to_le_bytes());
    for (number, (name, data)) in files.iter().enumerate() {
        out.extend_from_slice(&(number as u32 + 1).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(&(data.len() as i64).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    for (_, data) in files {
        out.extend_from_slice(data);
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_gma as gma;

    fn copy(dir: &Path, id: &str, bytes: &[u8], legacy: bool, time: u64) -> SteamCopy {
        let path = dir.join(format!(
            "{id}{}",
            if legacy { "_legacy.bin" } else { ".gma" }
        ));
        if legacy {
            let mut packed = Vec::new();
            lzma_rs::lzma_compress(&mut &bytes[..], &mut packed).unwrap();
            fs::write(&path, packed).unwrap();
        } else {
            fs::write(&path, bytes).unwrap();
        }
        SteamCopy {
            id: id.into(),
            size: fs::metadata(&path).unwrap().len(),
            path,
            legacy,
            time_updated: time,
        }
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    fn entry(time: u64) -> LibraryEntry {
        LibraryEntry {
            time_updated: time,
            ..Default::default()
        }
    }

    fn steam_copy(id: &str, time: u64) -> (String, SteamCopy) {
        (
            id.into(),
            SteamCopy {
                id: id.into(),
                path: PathBuf::new(),
                legacy: false,
                size: 1,
                time_updated: time,
            },
        )
    }

    #[test]
    fn plan_uses_steam_then_library_then_download() {
        let steam: HashMap<_, _> = [steam_copy("1", 10), steam_copy("9", 5), steam_copy("8", 5)]
            .into_iter()
            .collect();
        let library: BTreeMap<_, _> = [
            ("1".to_string(), entry(9)),
            ("2".to_string(), entry(20)),
            ("3".to_string(), entry(1)),
            ("9".to_string(), entry(5)),
        ]
        .into_iter()
        .collect();
        let installed: HashMap<_, _> = [
            ("1".to_string(), 9),
            ("3".to_string(), 1),
            ("7".to_string(), 4),
        ]
        .into_iter()
        .collect();
        let latest: HashMap<_, _> = [("3".to_string(), 2)].into_iter().collect();
        let plan = plan(
            &ids(&["1", "2", "3", "4", "1"]),
            &steam,
            &library,
            &installed,
            &latest,
            true,
        );
        assert_eq!(plan.subscribe, ids(&["1", "3", "4"]));
        assert_eq!(
            plan.store,
            ids(&["1"]),
            "outdated library copy is refreshed"
        );
        assert_eq!(plan.install, ids(&["2"]));
        assert_eq!(plan.uninstall, ids(&["1", "3", "7"]));
        assert_eq!(
            plan.unsubscribe,
            ids(&["9"]),
            "8 is not stored, so Steam keeps it"
        );
    }

    #[test]
    fn plan_skips_reinstall_of_current_copy_and_disabled_mode_uses_steam_only() {
        let library: BTreeMap<_, _> = [("2".to_string(), entry(20))].into_iter().collect();
        let installed: HashMap<_, _> = [("2".to_string(), 20)].into_iter().collect();
        let steam: HashMap<_, _> = [steam_copy("5", 1)].into_iter().collect();
        let current = plan(
            &ids(&["2"]),
            &HashMap::new(),
            &library,
            &installed,
            &HashMap::new(),
            true,
        );
        assert_eq!(current, Plan::default());
        let off = plan(
            &ids(&["2", "5"]),
            &steam,
            &library,
            &installed,
            &HashMap::new(),
            false,
        );
        assert_eq!(off.subscribe, ids(&["2", "5"]));
        assert_eq!(off.uninstall, ids(&["2"]));
        assert!(off.store.is_empty() && off.install.is_empty() && off.unsubscribe.is_empty());
    }

    #[test]
    fn store_and_install_round_trip_plain_and_legacy() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("garrysmod");
        fs::create_dir_all(game.join("addons")).unwrap();
        let mut library = Library::open(&temp.path().join("library")).unwrap();
        let body = vec![7u8; 300_000];
        let bytes = gma(&[
            ("lua/autorun/a.lua", b"print(1)"),
            ("materials/x.vtf", &body),
        ]);
        for (id, legacy) in [("100", false), ("200", true)] {
            let source = copy(temp.path(), id, &bytes, legacy, 42);
            library.store(&source, "Test", &mut |_| {}).unwrap();
            assert_eq!(library.entries[id].raw_size, bytes.len() as u64);
            assert!(library.entries[id].stored_size < bytes.len() as u64);
            install(&library, id, &game, &mut |_| {}).unwrap();
            let folder = game.join("addons").join(folder_name(id));
            assert_eq!(
                fs::read(folder.join("lua/autorun/a.lua")).unwrap(),
                b"print(1)"
            );
            assert_eq!(fs::read(folder.join("materials/x.vtf")).unwrap(), body);
        }
        let reopened = Library::open(&temp.path().join("library")).unwrap();
        assert_eq!(reopened.entries.len(), 2);
        let installed = installed_copies(&game);
        assert_eq!(installed.get("100"), Some(&42));
        assert_eq!(installed.get("200"), Some(&42));
        uninstall(&game, "100").unwrap();
        assert!(!game.join("addons/gmm_100").exists());
        assert!(!installed_copies(&game).contains_key("100"));
    }

    #[test]
    fn optimizes_old_library_copy_without_losing_addon_files() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("garrysmod");
        let mut library = Library::open(&temp.path().join("library")).unwrap();
        let mut seed = vec![0; 3 * 1024 * 1024];
        let mut state = 1u64;
        for chunk in seed.chunks_mut(8) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            chunk.copy_from_slice(&state.to_le_bytes()[..chunk.len()]);
        }
        let mut body = seed.clone();
        body.extend_from_slice(&seed);
        let bytes = gma(&[("materials/repeated.bin", &body)]);
        let old = zstd::encode_all(bytes.as_slice(), ZSTD_LEVEL).unwrap();
        fs::write(library.file("100"), &old).unwrap();
        // A pre-update index has no `optimized` field.
        fs::write(
            library.root.join("index.json"),
            format!(
                r#"{{"100":{{"title":"Repetitions","timeUpdated":10,"rawSize":{},"storedSize":{}}}}}"#,
                bytes.len(),
                old.len()
            ),
        )
        .unwrap();
        let mut library = Library::open(&library.root).unwrap();
        assert!(can_optimize(&library.entries["100"]));
        let saved = library.optimize("100", &mut |_| {}).unwrap();
        assert!(
            saved > 100_000,
            "strong compression should reduce this archive"
        );
        assert_eq!(library.entries["100"].stored_size, old.len() as u64 - saved);
        assert!(library.entries["100"].optimized);
        assert_eq!(library.optimize("100", &mut |_| {}).unwrap(), 0);
        let library = Library::open(&library.root).unwrap();
        assert!(!can_optimize(&library.entries["100"]));
        install(&library, "100", &game, &mut |_| {}).unwrap();
        assert_eq!(
            fs::read(game.join("addons/gmm_100/materials/repeated.bin")).unwrap(),
            body
        );
    }

    #[test]
    fn failed_optimization_preserves_original_library_copy() {
        let temp = tempfile::tempdir().unwrap();
        let mut library = Library::open(&temp.path().join("library")).unwrap();
        let original = b"invalid zstd frame";
        fs::write(library.file("100"), original).unwrap();
        library.entries.insert(
            "100".into(),
            LibraryEntry {
                raw_size: 100,
                stored_size: original.len() as u64,
                ..Default::default()
            },
        );
        assert!(library.optimize("100", &mut |_| {}).is_err());
        assert_eq!(fs::read(library.file("100")).unwrap(), original);
        assert!(!library.file("100").with_extension("zst.tmp").exists());
        assert!(!library.entries["100"].optimized);
    }

    #[test]
    fn unsafe_entry_fails_and_keeps_previous_install() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("garrysmod");
        let mut library = Library::open(&temp.path().join("library")).unwrap();
        let good = copy(temp.path(), "300", &gma(&[("lua/a.lua", b"ok")]), false, 1);
        library.store(&good, "Good", &mut |_| {}).unwrap();
        install(&library, "300", &game, &mut |_| {}).unwrap();
        for name in ["../evil.lua", "lua/../../x.lua", "C:/x.lua", "lua\\x.lua"] {
            let bad = copy(temp.path(), "300", &gma(&[(name, b"bad")]), false, 2);
            library.store(&bad, "Bad", &mut |_| {}).unwrap();
            assert!(
                install(&library, "300", &game, &mut |_| {}).is_err(),
                "{name}"
            );
            assert_eq!(
                fs::read(game.join("addons/gmm_300/lua/a.lua")).unwrap(),
                b"ok"
            );
            assert!(!game.join("addons/gmm_300.tmp").exists());
            assert!(!temp.path().join("evil.lua").exists());
        }
    }

    #[test]
    fn store_rejects_non_gma_and_uninstall_keeps_foreign_folders() {
        let temp = tempfile::tempdir().unwrap();
        let game = temp.path().join("garrysmod");
        let mut library = Library::open(&temp.path().join("library")).unwrap();
        let junk = copy(temp.path(), "400", b"not an addon", false, 1);
        assert!(library.store(&junk, "Junk", &mut |_| {}).is_err());
        assert!(library.entries.is_empty());
        assert!(!library.file("400").exists());
        fs::create_dir_all(game.join("addons/gmm_500")).unwrap();
        assert!(uninstall(&game, "500").is_err());
        assert!(game.join("addons/gmm_500").exists());
    }

    #[test]
    fn scans_steam_content_and_update_times() {
        let temp = tempfile::tempdir().unwrap();
        let steamapps = temp.path().join("steamapps");
        let game = steamapps.join("common/GarrysMod/garrysmod");
        fs::create_dir_all(&game).unwrap();
        let content = steamapps.join("workshop/content/4000");
        fs::create_dir_all(content.join("11")).unwrap();
        fs::create_dir_all(content.join("22")).unwrap();
        fs::create_dir_all(content.join("notes")).unwrap();
        fs::write(content.join("11/addon.gma"), b"GMAD").unwrap();
        fs::write(content.join("22/99_legacy.bin"), b"xx").unwrap();
        fs::write(
            steamapps.join("workshop/appworkshop_4000.acf"),
            "\"AppWorkshop\"\n{\n\t\"appid\"\t\t\"4000\"\n\t\"WorkshopItemsInstalled\"\n\t{\n\t\t\"11\"\n\t\t{\n\t\t\t\"size\"\t\t\"4\"\n\t\t\t\"timeupdated\"\t\t\"123\"\n\t\t}\n\t}\n\t\"WorkshopItemDetails\"\n\t{\n\t\t\"22\"\n\t\t{\n\t\t\t\"timeupdated\"\t\t\"999\"\n\t\t}\n\t}\n}\n",
        )
        .unwrap();
        let found = scan_steam(&game);
        assert_eq!(found.len(), 2);
        assert_eq!(found["11"].time_updated, 123);
        assert!(!found["11"].legacy);
        assert!(found["22"].legacy);
        assert_eq!(found["22"].time_updated, 0, "only installed items count");
    }
}
