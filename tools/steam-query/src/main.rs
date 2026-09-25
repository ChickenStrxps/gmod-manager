//! Isolated Steamworks process: the manager remains a standalone EXE and loads the game's
//! steam_api64.dll only for an explicit dependency check.
use std::{io::{self, BufRead, Write}, time::{Duration, Instant}};
use steamworks::{AppId, Client, PublishedFileId};

fn query(client: &Client, ids: Vec<u64>) -> Result<serde_json::Value, String> {
    let requested: std::collections::HashSet<u64> = ids.iter().copied().collect();
    let items = ids.into_iter().map(PublishedFileId).collect();
    let handle = client.ugc().query_items(items).map_err(|e| format!("Steam query failed: {e:?}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    handle.include_children(true).fetch(move |result| {
        let mapped = result.map(|results| {
            (0..results.returned_results()).filter_map(|index| {
                let item = results.get(index)?;
                let children = results.get_children(index)?;
                Some((item.published_file_id.0, item.consumer_app_id.map(|id| id.0),
                    children.into_iter().map(|id| id.0).collect::<Vec<_>>()))
            }).collect::<Vec<_>>()
        }).map_err(|e| format!("Steam query failed: {e:?}"));
        let _ = tx.send(mapped);
    });
    let start = Instant::now();
    let rows = loop {
        client.run_callbacks();
        if let Ok(rows) = rx.try_recv() { break rows?; }
        if start.elapsed() > Duration::from_secs(25) {
            return Err("Steam dependency query timed out.".into());
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let mut found = serde_json::Map::new();
    for (id, app, children) in rows {
        if !requested.contains(&id) || app != Some(4000) {
            return Err(format!("Workshop item {id} is not available for GMod."));
        }
        found.insert(id.to_string(), serde_json::json!(children));
    }
    if found.len() != requested.len() {
        return Err("Steam did not return every requested Workshop item.".into());
    }
    Ok(serde_json::Value::Object(found))
}

fn main() {
    // The app passes JSON arrays on stdin, one batch per line. Only JSON goes to stdout.
    let client = Client::init_app(AppId(4000));
    let stdin = io::stdin();
    let mut output = io::stdout().lock();
    for line in stdin.lock().lines() {
        let result = match &client {
            Ok(client) => line.map_err(|e| e.to_string())
                .and_then(|line| serde_json::from_str::<Vec<u64>>(&line).map_err(|e| e.to_string()))
                .and_then(|ids| query(client, ids)),
            Err(error) => Err(format!("Could not connect to Steam for GMod: {error:?}")),
        };
        let reply = match result {
            Ok(items) => serde_json::json!({"items": items}),
            Err(error) => serde_json::json!({"error": error}),
        };
        if writeln!(output, "{reply}").is_err() || output.flush().is_err() { break; }
    }
}
