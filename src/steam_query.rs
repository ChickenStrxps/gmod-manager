//! Runs Steamworks only when requested, using Garry's Mod's own Steam runtime library.
//! Embedding the query executable keeps downloads, auto-updates and portability single-file.
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

#[cfg(windows)]
const HELPER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/gmm-steam-query.exe"));

pub struct SteamQuery {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl SteamQuery {
    #[cfg(windows)]
    pub fn open(dir: &Path, game: &Path) -> Result<Self, String> {
        use std::os::windows::process::CommandExt;
        let bin = game
            .parent()
            .ok_or("GMod folder has no parent")?
            .join("bin/win64");
        if !bin.join("steam_api64.dll").is_file() {
            return Err("GMod's Steam runtime was not found in bin/win64.".into());
        }
        let cache = dir.join("cache");
        fs::create_dir_all(&cache).map_err(|e| e.to_string())?;
        let exe = cache.join("gmm-steam-query.exe");
        fs::write(&exe, HELPER).map_err(|e| format!("Could not prepare Steam query: {e}"))?;
        let path = std::env::join_paths(
            std::iter::once(bin.as_os_str().to_os_string()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
                    .map(|p| p.into_os_string()),
            ),
        )
        .map_err(|e| e.to_string())?;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = Command::new(exe)
            .env("PATH", path)
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start Steam query: {e}"))?;
        let stdin = child.stdin.take().ok_or("Steam query stdin unavailable")?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or("Steam query stdout unavailable")?,
        );
        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }

    #[cfg(not(windows))]
    pub fn open(_dir: &Path, _game: &Path) -> Result<Self, String> {
        Err("Steam dependency checks require Windows.".into())
    }

    pub fn fetch(&mut self, ids: &[String]) -> Result<HashMap<String, Vec<String>>, String> {
        let ids = ids
            .iter()
            .map(|id| {
                id.parse::<u64>()
                    .map_err(|_| format!("Invalid Workshop ID: {id}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        serde_json::to_writer(&mut self.stdin, &ids).map_err(|e| e.to_string())?;
        self.stdin.write_all(b"\n").map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        let mut response = String::new();
        if self
            .stdout
            .read_line(&mut response)
            .map_err(|e| e.to_string())?
            == 0
        {
            return Err("Steam query stopped before returning results.".into());
        }
        let value: serde_json::Value =
            serde_json::from_str(&response).map_err(|e| format!("Invalid Steam response: {e}"))?;
        if let Some(error) = value["error"].as_str() {
            return Err(error.to_owned());
        }
        let items = value
            .get("items")
            .ok_or("Steam did not return dependency data")?;
        let result: HashMap<String, Vec<u64>> = serde_json::from_value(items.clone())
            .map_err(|e| format!("Invalid Steam dependencies: {e}"))?;
        if result.len() != ids.len() {
            return Err("Steam omitted a requested Workshop item.".into());
        }
        Ok(result
            .into_iter()
            .map(|(id, children)| (id, children.into_iter().map(|id| id.to_string()).collect()))
            .collect())
    }
}

impl Drop for SteamQuery {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
