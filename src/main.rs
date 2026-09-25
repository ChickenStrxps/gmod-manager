#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod library;
mod model;
mod update;
mod workshop;

use crate::library::{Library, LibraryEntry, SteamCopy};
use crate::model::{Bind, FileMapping, LocalState, Profile, Setting, WorkshopItem};
use crate::workshop::{ItemMeta, format_size};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontData, FontFamily, FontId, Frame, Id, Layout,
    Margin, Rect, RichText, Sense, Stroke, TextStyle, pos2, vec2,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::Duration,
};

const BG: Color32 = Color32::from_rgb(23, 25, 29);
const SIDEBAR: Color32 = Color32::from_rgb(18, 20, 23);
const ROW: Color32 = Color32::from_rgb(31, 34, 39);
const ROW_HOVER: Color32 = Color32::from_rgb(38, 42, 48);
const INPUT: Color32 = Color32::from_rgb(16, 18, 21);
const LINE: Color32 = Color32::from_rgb(43, 47, 54);
const TEXT: Color32 = Color32::from_rgb(226, 229, 233);
const MUTED: Color32 = Color32::from_rgb(139, 146, 157);
const FAINT: Color32 = Color32::from_rgb(96, 102, 112);
const ACCENT: Color32 = Color32::from_rgb(58, 138, 245);
const GREEN: Color32 = Color32::from_rgb(78, 190, 132);
const AMBER: Color32 = Color32::from_rgb(224, 166, 74);
const RED: Color32 = Color32::from_rgb(232, 92, 96);

const ROW_HEIGHT: f32 = 50.0;
const WORKSHOP_ITEM: &str = "https://steamcommunity.com/sharedfiles/filedetails/?id=";

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Preset,
    Library,
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Mods,
    Binds,
    Settings,
    Details,
}

#[derive(Clone)]
enum JobKind {
    Apply { launch: bool },
    Restore,
    Store,
    MoveLibrary(String),
    Update,
}

enum JobEvent {
    Progress(String, f32),
    Done(Result<String, String>),
}

struct Job {
    kind: JobKind,
    label: String,
    fraction: f32,
    rx: Receiver<JobEvent>,
}

struct Toast {
    text: String,
    error: bool,
    shown_at: f64,
}

type IconResult = (String, Option<([usize; 2], Vec<u8>)>);

/// Loads Workshop thumbnails on a few background threads.
struct Icons {
    dir: PathBuf,
    textures: HashMap<String, Option<egui::TextureHandle>>,
    queue: Option<Sender<(String, String)>>,
    results: Receiver<IconResult>,
    results_tx: Sender<IconResult>,
}

impl Icons {
    fn new(dir: PathBuf) -> Self {
        let (results_tx, results) = mpsc::channel();
        Self {
            dir,
            textures: HashMap::new(),
            queue: None,
            results,
            results_tx,
        }
    }

    fn start(&mut self, ctx: &egui::Context) -> &Sender<(String, String)> {
        self.queue.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel::<(String, String)>();
            let rx = Arc::new(Mutex::new(rx));
            for _ in 0..4 {
                let rx = Arc::clone(&rx);
                let out = self.results_tx.clone();
                let dir = self.dir.clone();
                let ctx = ctx.clone();
                std::thread::spawn(move || {
                    loop {
                        let next = rx.lock().ok().and_then(|rx| rx.recv().ok());
                        let Some((id, url)) = next else { break };
                        let image = workshop::load_icon(&dir, &id, &url).ok();
                        if out.send((id, image)).is_err() {
                            break;
                        }
                        ctx.request_repaint();
                    }
                });
            }
            tx
        })
    }

    fn get(&mut self, ctx: &egui::Context, id: &str, url: &str) -> Option<egui::TextureId> {
        if let Some(slot) = self.textures.get(id) {
            return slot.as_ref().map(|t| t.id());
        }
        if url.is_empty() && !workshop::icon_path(&self.dir, id).is_file() {
            return None;
        }
        self.textures.insert(id.to_owned(), None);
        let _ = self.start(ctx).send((id.to_owned(), url.to_owned()));
        None
    }

    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok((id, image)) = self.results.try_recv() {
            let texture = image.map(|(size, rgba)| {
                ctx.load_texture(
                    format!("icon-{id}"),
                    egui::ColorImage::from_rgba_unmultiplied(size, &rgba),
                    egui::TextureOptions::LINEAR,
                )
            });
            self.textures.insert(id, texture);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ModState {
    Unavailable,
    Steam,
    Unpacked,
    Library,
    Missing,
}

impl ModState {
    fn describe(self) -> (&'static str, Color32) {
        match self {
            Self::Unavailable => ("Removed from Workshop", RED),
            Self::Steam => ("Installed", GREEN),
            Self::Unpacked => ("Installed from library", GREEN),
            Self::Library => ("In library", ACCENT),
            Self::Missing => ("Downloads on launch", FAINT),
        }
    }
}

struct Row {
    id: String,
    title: String,
    size: Option<u64>,
    author: String,
    state: ModState,
    url: String,
}

struct App {
    dir: PathBuf,
    state: LocalState,
    profiles: Vec<Profile>,
    selected: usize,
    view: View,
    tab: Tab,
    path_input: String,
    toast: Option<Toast>,
    edited_at: Option<f64>,
    save_error: Option<String>,

    meta: HashMap<String, ItemMeta>,
    meta_rx: Option<Receiver<Result<HashMap<String, ItemMeta>, String>>>,
    meta_requested: HashSet<String>,
    authors: HashMap<String, String>,
    authors_rx: Option<Receiver<HashMap<String, String>>>,
    authors_requested: HashSet<String>,
    add_rx: Option<
        Receiver<
            Result<
                (
                    String,
                    core::WorkshopSearchItem,
                    Vec<WorkshopItem>,
                    HashMap<String, ItemMeta>,
                ),
                String,
            >,
        >,
    >,
    icons: Icons,

    steam: HashMap<String, SteamCopy>,
    installed: HashMap<String, u64>,
    library: BTreeMap<String, LibraryEntry>,
    workshop_status: Option<core::WorkshopStatus>,
    last_scan: f64,

    filter: String,
    adding: bool,
    query: String,
    results: Vec<core::WorkshopSearchItem>,
    results_query: String,
    page: usize,
    search_rx: Option<Receiver<Result<(String, usize, Vec<core::WorkshopSearchItem>), String>>>,
    refresh_rx: Option<
        Receiver<Result<(String, Profile, usize, usize, HashMap<String, ItemMeta>), String>>,
    >,

    job: Option<Job>,
    new_preset: Option<String>,
    review: Option<Result<core::Prepared, String>>,
    update: Option<update::Release>,
    update_rx: Option<Receiver<Result<Option<update::Release>, String>>>,
    update_manual: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        configure_fonts(&cc.egui_ctx);
        configure_style(&cc.egui_ctx);
        let dir = core::app_dir();
        let mut state = core::load_state(&dir);
        if state.game_path.is_empty() {
            if let Some(path) = core::detect_game() {
                state.game_path = core::display_path(&path);
            }
        } else {
            state.game_path = core::display_path(Path::new(&state.game_path));
        }
        let (profiles, toast) = match core::load_profiles(&dir) {
            Ok(profiles) => (profiles, None),
            Err(e) => (
                vec![serde_json::from_str(core::BUILTIN).expect("built-in preset")],
                Some(Toast {
                    text: e,
                    error: true,
                    shown_at: 0.0,
                }),
            ),
        };
        let selected = profiles
            .iter()
            .position(|p| p.id == state.selected_profile)
            .unwrap_or(0);
        let mut app = Self {
            icons: Icons::new(dir.clone()),
            meta: workshop::load_meta(&dir),
            authors: workshop::load_authors(&dir),
            authors_rx: None,
            authors_requested: HashSet::new(),
            add_rx: None,
            dir,
            path_input: state.game_path.clone(),
            state,
            profiles,
            selected,
            view: View::Preset,
            tab: Tab::Mods,
            toast,
            edited_at: None,
            save_error: None,
            meta_rx: None,
            meta_requested: HashSet::new(),
            steam: HashMap::new(),
            installed: HashMap::new(),
            library: BTreeMap::new(),
            workshop_status: None,
            last_scan: 0.0,
            filter: String::new(),
            adding: false,
            query: String::new(),
            results: Vec::new(),
            results_query: String::new(),
            page: 1,
            search_rx: None,
            refresh_rx: None,
            job: None,
            new_preset: None,
            review: None,
            update: None,
            update_rx: None,
            update_manual: false,
        };
        app.rescan();
        update::cleanup();
        if !cfg!(debug_assertions) {
            app.check_updates(&cc.egui_ctx, false);
        }
        app
    }

    // ---------- state helpers ----------

    fn profile(&self) -> &Profile {
        &self.profiles[self.selected]
    }

    fn profile_mut(&mut self) -> &mut Profile {
        &mut self.profiles[self.selected]
    }

    fn game(&self) -> Option<PathBuf> {
        core::normalize_game_path(Path::new(&self.path_input))
    }

    fn library_root(&self) -> PathBuf {
        if self.state.library_path.is_empty() {
            self.dir.join("library")
        } else {
            PathBuf::from(&self.state.library_path)
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.add_rx.is_some()
    }

    fn notify(&mut self, ctx: &egui::Context, result: Result<String, String>) {
        let (text, error) = match result {
            Ok(text) => (text, false),
            Err(text) => (text, true),
        };
        self.toast = Some(Toast {
            text,
            error,
            shown_at: ctx.input(|i| i.time),
        });
    }

    fn remember(&mut self) {
        self.state.game_path = self.path_input.clone();
        self.state.selected_profile = self.profile().id.clone();
        let _ = core::save_state(&self.dir, &self.state);
    }

    /// Marks the preset as edited; it is saved automatically a moment later.
    fn touch(&mut self, ctx: &egui::Context) {
        self.edited_at = Some(ctx.input(|i| i.time));
        self.review = None;
    }

    fn save_now(&mut self) -> bool {
        self.edited_at = None;
        match core::save_profile(&self.dir, self.profile()) {
            Ok(()) => {
                self.save_error = None;
                true
            }
            Err(e) => {
                self.save_error = Some(e);
                false
            }
        }
    }

    fn autosave(&mut self, ctx: &egui::Context) {
        if let Some(at) = self.edited_at {
            let now = ctx.input(|i| i.time);
            if now - at > 0.6 {
                self.save_now();
            } else {
                ctx.request_repaint_after(Duration::from_millis(650));
            }
        }
    }

    fn select(&mut self, index: usize) {
        if self.edited_at.is_some() {
            self.save_now();
        }
        self.selected = index;
        self.view = View::Preset;
        self.review = None;
        self.results.clear();
        self.results_query.clear();
        self.filter.clear();
        self.save_error = None;
        self.remember();
    }

    fn rescan(&mut self) {
        match self.game() {
            Some(game) => {
                self.steam = library::scan_steam(&game);
                self.installed = library::installed_copies(&game);
                self.workshop_status = core::read_workshop_status(&game);
            }
            None => {
                self.steam.clear();
                self.installed.clear();
                self.workshop_status = None;
            }
        }
        self.library = library::read_entries(&self.library_root());
    }

    fn title_of<'a>(&'a self, item: &'a WorkshopItem) -> &'a str {
        item.title
            .as_deref()
            .filter(|t| !t.is_empty())
            .or_else(|| {
                self.meta
                    .get(&item.id)
                    .map(|m| m.title.as_str())
                    .filter(|t| !t.is_empty())
            })
            .unwrap_or("Unknown item")
    }

    fn mod_state(&self, item: &WorkshopItem) -> ModState {
        if !item.available {
            ModState::Unavailable
        } else if self.steam.contains_key(&item.id) {
            ModState::Steam
        } else if self.installed.contains_key(&item.id) {
            ModState::Unpacked
        } else if self.library.contains_key(&item.id) {
            ModState::Library
        } else {
            ModState::Missing
        }
    }

    /// Total Workshop size of a preset's available mods, and whether every size is known.
    fn preset_size(&self, profile: &Profile) -> (u64, bool) {
        let mut total = 0;
        let mut complete = true;
        for item in profile.workshop.items.iter().filter(|i| i.available) {
            match self.meta.get(&item.id) {
                Some(meta) if meta.size > 0 => total += meta.size,
                _ => complete = false,
            }
        }
        (total, complete)
    }

    fn size_text(&self, profile: &Profile) -> String {
        let (bytes, complete) = self.preset_size(profile);
        if bytes == 0 {
            "—".into()
        } else if complete {
            format_size(bytes)
        } else {
            format!("{}+", format_size(bytes))
        }
    }

    // ---------- background work ----------

    fn ensure_meta(&mut self, ctx: &egui::Context) {
        if self.meta_rx.is_some() {
            return;
        }
        let mut ids: Vec<String> = self
            .profiles
            .iter()
            .flat_map(|p| p.workshop.items.iter())
            .filter(|item| {
                !self
                    .meta
                    .get(&item.id)
                    .is_some_and(|meta| !meta.available || !meta.creator.is_empty())
            })
            .filter(|item| !self.meta_requested.contains(&item.id))
            .map(|item| item.id.clone())
            .collect();
        ids.sort();
        ids.dedup();
        if ids.is_empty() {
            return;
        }
        self.meta_requested.extend(ids.iter().cloned());
        let (tx, rx) = mpsc::channel();
        self.meta_rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(workshop::fetch_meta(&ids));
            ctx.request_repaint();
        });
    }

    fn poll_meta(&mut self) {
        let Some(result) = self.meta_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.meta_rx = None;
        if let Ok(found) = result {
            self.meta.extend(found);
            let _ = workshop::save_meta(&self.dir, &self.meta);
        }
    }

    fn author_of(&self, meta: Option<&ItemMeta>) -> &str {
        meta.and_then(|meta| self.authors.get(&meta.creator))
            .map(String::as_str)
            .unwrap_or("")
    }

    fn ensure_authors(&mut self, ctx: &egui::Context) {
        if self.authors_rx.is_some() {
            return;
        }
        let ids: Vec<String> = self
            .meta
            .values()
            .filter(|meta| !meta.creator.is_empty())
            .map(|meta| &meta.creator)
            .filter(|id| !self.authors.contains_key(*id) && !self.authors_requested.contains(*id))
            .take(12)
            .cloned()
            .collect();
        if ids.is_empty() {
            return;
        }
        self.authors_requested.extend(ids.iter().cloned());
        let (tx, rx) = mpsc::channel();
        self.authors_rx = Some(rx);
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let mut found = HashMap::new();
            for id in ids {
                if let Ok(name) = workshop::author_name(&id) {
                    found.insert(id, name);
                }
            }
            let _ = tx.send(found);
            repaint.request_repaint();
        });
    }

    fn poll_authors(&mut self) {
        let Some(found) = self.authors_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.authors_rx = None;
        if !found.is_empty() {
            self.authors.extend(found);
            let _ = workshop::save_authors(&self.dir, &self.authors);
        }
    }

    fn spawn_job<F>(&mut self, ctx: &egui::Context, kind: JobKind, label: &str, work: F)
    where
        F: FnOnce(&mut dyn FnMut(&str, f32)) -> Result<String, String> + Send + 'static,
    {
        if self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let mut last_label = String::new();
            let mut last_fraction = -1.0f32;
            let mut progress = |label: &str, fraction: f32| {
                let fraction = fraction.clamp(0.0, 1.0);
                if label != last_label || (fraction - last_fraction).abs() >= 0.01 {
                    last_label = label.to_owned();
                    last_fraction = fraction;
                    let _ = tx.send(JobEvent::Progress(last_label.clone(), fraction));
                    repaint.request_repaint();
                }
            };
            let result = work(&mut progress);
            let _ = tx.send(JobEvent::Done(result));
            repaint.request_repaint();
        });
        self.job = Some(Job {
            kind,
            label: label.to_owned(),
            fraction: 0.0,
            rx,
        });
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = self.job.as_mut() else {
            return;
        };
        let mut finished = None;
        while let Ok(event) = job.rx.try_recv() {
            match event {
                JobEvent::Progress(label, fraction) => {
                    job.label = label;
                    job.fraction = fraction;
                }
                JobEvent::Done(result) => finished = Some(result),
            }
        }
        let Some(result) = finished else {
            return;
        };
        let kind = self.job.take().map(|job| job.kind);
        let result = match (kind, result) {
            (Some(JobKind::Apply { launch: true }), Ok(_)) => {
                let game = self
                    .game()
                    .ok_or_else(|| "Set your GMod folder first.".to_owned());
                game.and_then(|game| core::launch_game(&game))
                    .map(|_| "Starting GMod…".to_owned())
            }
            (Some(JobKind::Update), Ok(path)) => {
                if self.edited_at.is_some() {
                    self.save_now();
                }
                self.remember();
                match std::process::Command::new(&path)
                    .current_dir(&self.dir)
                    .spawn()
                {
                    Ok(_) => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        return;
                    }
                    Err(e) => Err(format!(
                        "Updated, but couldn't restart: {e}. Open the app again."
                    )),
                }
            }
            (Some(JobKind::MoveLibrary(path)), Ok(message)) => {
                self.state.library_path = path;
                let _ = core::save_state(&self.dir, &self.state);
                Ok(message)
            }
            (_, result) => result,
        };
        self.notify(ctx, result);
        self.rescan();
    }

    fn start_apply(&mut self, ctx: &egui::Context, launch: bool) {
        if self.busy() {
            return;
        }
        if !self.save_now() {
            let error = self.save_error.clone().unwrap_or_default();
            self.notify(ctx, Err(format!("Fix this first: {error}")));
            return;
        }
        let Some(game) = self.game() else {
            self.notify(ctx, Err("Set your GMod folder first.".into()));
            self.view = View::Settings;
            return;
        };
        self.review = None;
        let profile = self.profile().clone();
        let root = self.state.library_enabled.then(|| self.library_root());
        let meta = self.preset_meta();
        let dir = self.dir.clone();
        let label = if launch { "Getting ready" } else { "Applying" };
        self.spawn_job(ctx, JobKind::Apply { launch }, label, move |progress| {
            let mut library = root.as_deref().map(Library::open).transpose()?;
            let prepared = core::prepare(&game, &profile, library.as_ref(), &meta)?;
            core::execute(&dir, &game, &profile, &prepared, library.as_mut(), progress)
        });
    }

    /// Cached Workshop details for the selected preset's mods.
    fn preset_meta(&self) -> HashMap<String, ItemMeta> {
        self.profile()
            .workshop
            .items
            .iter()
            .filter_map(|item| Some((item.id.clone(), self.meta.get(&item.id)?.clone())))
            .collect()
    }

    /// Looks for a newer release in the background. `manual` reports "up to date" too.
    fn check_updates(&mut self, ctx: &egui::Context, manual: bool) {
        if self.update_rx.is_some() {
            return;
        }
        self.update_manual = manual;
        let (tx, rx) = mpsc::channel();
        self.update_rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::check());
            ctx.request_repaint();
        });
    }

    fn poll_update(&mut self, ctx: &egui::Context) {
        let Some(result) = self.update_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.update_rx = None;
        match result {
            Ok(Some(release)) => {
                if self.update_manual {
                    self.notify(
                        ctx,
                        Ok(format!("Version {} is available.", release.version)),
                    );
                }
                self.update = Some(release);
            }
            Ok(None) if self.update_manual => {
                self.notify(ctx, Ok("You're on the latest version.".into()))
            }
            Err(e) if self.update_manual => self.notify(ctx, Err(e)),
            _ => {}
        }
    }

    fn start_update(&mut self, ctx: &egui::Context) {
        let Some(release) = self.update.clone() else {
            return;
        };
        let label = format!("Downloading {}", release.version);
        self.spawn_job(ctx, JobKind::Update, &label, move |progress| {
            let label = format!("Downloading {}", release.version);
            update::install(&release, &mut |fraction| progress(&label, fraction))
                .map(|path| path.to_string_lossy().into_owned())
        });
    }

    fn start_restore(&mut self, ctx: &egui::Context) {
        let Some(game) = self.game() else {
            self.notify(ctx, Err("Set your GMod folder first.".into()));
            return;
        };
        let root = self.library_root();
        let dir = self.dir.clone();
        self.spawn_job(ctx, JobKind::Restore, "Undoing", move |progress| {
            let library = root.is_dir().then(|| Library::open(&root)).transpose()?;
            core::restore_latest(&dir, &game, library.as_ref(), progress)
        });
    }

    /// Compresses every preset mod Steam has on disk that the library lacks or has outdated.
    fn start_store_all(&mut self, ctx: &egui::Context) {
        let mut titles = BTreeMap::new();
        for profile in &self.profiles {
            for item in profile.workshop.items.iter().filter(|i| i.available) {
                let Some(copy) = self.steam.get(&item.id) else {
                    continue;
                };
                if self
                    .library
                    .get(&item.id)
                    .is_some_and(|e| e.time_updated >= copy.time_updated)
                {
                    continue;
                }
                titles.insert(item.id.clone(), self.title_of(item).to_owned());
            }
        }
        if titles.is_empty() {
            self.notify(
                ctx,
                Ok("Everything installed is already in the library.".into()),
            );
            return;
        }
        let Some(game) = self.game() else {
            self.notify(ctx, Err("Set your GMod folder first.".into()));
            return;
        };
        let root = self.library_root();
        self.spawn_job(ctx, JobKind::Store, "Compressing", move |progress| {
            let mut library = Library::open(&root)?;
            let steam = library::scan_steam(&game);
            let count = titles.len();
            for (index, (id, title)) in titles.iter().enumerate() {
                let Some(copy) = steam.get(id) else { continue };
                let label = format!("Compressing {title} ({}/{count})", index + 1);
                let total = copy.size.max(1) as f32;
                library.store(copy, title, &mut |done| {
                    progress(&label, done as f32 / total)
                })?;
            }
            Ok(format!("Saved {count} mods to the library."))
        });
    }

    fn change_library_folder(&mut self, ctx: &egui::Context) {
        let Some(target) = rfd::FileDialog::new()
            .set_title("Choose a folder for the mod library")
            .set_directory(self.library_root().parent().unwrap_or(&self.dir))
            .pick_folder()
        else {
            return;
        };
        let current = self.library_root();
        if library::read_entries(&target).is_empty() && !self.library.is_empty() {
            let count = self.library.len();
            let path = target.to_string_lossy().into_owned();
            self.spawn_job(
                ctx,
                JobKind::MoveLibrary(path),
                "Moving library",
                move |progress| {
                    let mut library = Library::open(&current)?;
                    library.move_to(&target, &mut |done| {
                        progress("Moving library", done as f32 / count.max(1) as f32)
                    })?;
                    Ok("Library moved.".into())
                },
            );
        } else if self.library.is_empty() || library::read_entries(&current).is_empty() {
            self.state.library_path = target.to_string_lossy().into_owned();
            let _ = core::save_state(&self.dir, &self.state);
            self.rescan();
            self.notify(ctx, Ok("Library folder changed.".into()));
        } else {
            self.notify(
                ctx,
                Err("That folder already has a different library.".into()),
            );
        }
    }

    fn remove_from_library(&mut self, ctx: &egui::Context, id: &str) {
        let result = Library::open(&self.library_root()).and_then(|mut l| l.remove(id));
        if let Err(e) = result {
            self.notify(ctx, Err(e));
        }
        self.rescan();
    }

    fn open_review(&mut self) {
        let result = match self.game() {
            None => Err("Set your GMod folder first.".into()),
            Some(game) => {
                let library = if self.state.library_enabled {
                    Some(Library {
                        root: self.library_root(),
                        entries: self.library.clone(),
                    })
                } else {
                    None
                };
                core::prepare(&game, self.profile(), library.as_ref(), &self.preset_meta())
            }
        };
        self.review = Some(result);
    }

    fn search(&mut self, ctx: &egui::Context, page: usize) {
        if self.search_rx.is_some() {
            return;
        }
        let query = self.query.trim().to_owned();
        if query.chars().count() < 2 {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.search_rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = core::search_workshop(&query, page).map(|items| (query, page, items));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn poll_search(&mut self, ctx: &egui::Context) {
        let Some(result) = self.search_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.search_rx = None;
        match result {
            Ok((query, page, items)) => {
                for item in &items {
                    self.meta.insert(item.id.clone(), item.meta.clone());
                }
                if page == 1 {
                    self.results = items;
                } else {
                    let known: HashSet<String> =
                        self.results.iter().map(|r| r.id.clone()).collect();
                    self.results
                        .extend(items.into_iter().filter(|i| !known.contains(&i.id)));
                }
                self.results_query = query;
                self.page = page;
                if self.results.is_empty() {
                    self.notify(ctx, Ok("No mods found.".into()));
                }
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    fn start_add(&mut self, ctx: &egui::Context, result: core::WorkshopSearchItem) {
        if self.add_rx.is_some() {
            return;
        }
        let profile_id = self.profile().id.clone();
        let (tx, rx) = mpsc::channel();
        self.add_rx = Some(rx);
        let repaint = ctx.clone();
        std::thread::spawn(move || {
            let result = (|| {
                let ids = workshop::required_items(&result.id)?;
                let meta = workshop::dependency_meta(&ids)?;
                let dependencies = ids
                    .iter()
                    .map(|id| WorkshopItem {
                        id: id.clone(),
                        title: meta.get(id).map(|item| item.title.clone()),
                        available: true,
                        manually_added: true,
                    })
                    .collect();
                Ok((profile_id, result, dependencies, meta))
            })();
            let _ = tx.send(result);
            repaint.request_repaint();
        });
    }

    fn poll_add(&mut self, ctx: &egui::Context) {
        let Some(result) = self.add_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.add_rx = None;
        match result {
            Ok((profile_id, result, dependencies, meta)) => {
                self.meta.extend(meta);
                let _ = workshop::save_meta(&self.dir, &self.meta);
                if let Some(index) = self.profiles.iter().position(|p| p.id == profile_id) {
                    let profile = &mut self.profiles[index];
                    let mut present: HashSet<String> = profile
                        .workshop
                        .items
                        .iter()
                        .map(|item| item.id.clone())
                        .collect();
                    let root = WorkshopItem {
                        id: result.id,
                        title: Some(result.meta.title),
                        available: true,
                        manually_added: true,
                    };
                    let mut count = 0;
                    for item in std::iter::once(root).chain(dependencies) {
                        if present.insert(item.id.clone()) {
                            profile.workshop.items.push(item);
                            count += 1;
                        }
                    }
                    if count > 0 {
                        profile.sync.addons = true;
                        if self.selected == index {
                            self.review = None;
                        }
                    }
                    let saved = core::save_profile(&self.dir, profile);
                    self.notify(
                        ctx,
                        saved.map(|_| {
                            format!(
                                "Added {count} mod{}; Steam downloads missing mods on Play.",
                                if count == 1 { "" } else { "s" }
                            )
                        }),
                    );
                }
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    fn refresh_collection(&mut self, ctx: &egui::Context) {
        if self.refresh_rx.is_some() {
            return;
        }
        let mut profile = self.profile().clone();
        let (tx, rx) = mpsc::channel();
        self.refresh_rx = Some(rx);
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let id = profile.id.clone();
            let result = core::refresh_collection(&mut profile)
                .map(|(added, removed, meta)| (id, profile, added, removed, meta));
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn poll_refresh(&mut self, ctx: &egui::Context) {
        let Some(result) = self.refresh_rx.as_ref().and_then(|rx| rx.try_recv().ok()) else {
            return;
        };
        self.refresh_rx = None;
        match result {
            Ok((id, profile, added, removed, meta)) => {
                self.meta.extend(meta);
                let _ = workshop::save_meta(&self.dir, &self.meta);
                if let Some(index) = self.profiles.iter().position(|p| p.id == id) {
                    self.profiles[index].workshop = profile.workshop;
                    self.profiles[index].source.collection_url = profile.source.collection_url;
                    let saved = core::save_profile(&self.dir, &self.profiles[index]);
                    self.review = None;
                    self.notify(
                        ctx,
                        saved.map(|_| match (added, removed) {
                            (0, 0) => "Collection is unchanged.".into(),
                            _ => format!("Collection updated: {added} added, {removed} removed."),
                        }),
                    );
                }
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    // ---------- preset management ----------

    fn create_preset(&mut self, ctx: &egui::Context, name: &str, copy: bool) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let base: String = name
            .to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let base = if base.is_empty() {
            "preset".to_owned()
        } else {
            base
        };
        let mut id = base.clone();
        let mut n = 2;
        while self.profiles.iter().any(|p| p.id == id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        let mut profile = if copy {
            self.profile().clone()
        } else {
            Profile::blank(name, &id)
        };
        profile.id = id;
        profile.name = name.to_owned();
        match core::save_profile(&self.dir, &profile) {
            Ok(()) => {
                self.profiles.push(profile);
                self.select(self.profiles.len() - 1);
                self.tab = if copy { Tab::Mods } else { Tab::Details };
                self.new_preset = None;
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    fn import_preset(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("GMod Manager preset", &["zip", "json"])
            .pick_file()
        else {
            return;
        };
        match core::import_preset_file(&path) {
            Ok(mut profile) => {
                if self.profiles.iter().any(|p| p.id == profile.id) {
                    let base = profile.id.clone();
                    let mut n = 2;
                    while self.profiles.iter().any(|p| p.id == format!("{base}-{n}")) {
                        n += 1;
                    }
                    profile.id = format!("{base}-{n}");
                    profile.name = format!("{} ({n})", profile.name);
                }
                match core::save_profile(&self.dir, &profile) {
                    Ok(()) => {
                        let name = profile.name.clone();
                        self.profiles.push(profile);
                        self.select(self.profiles.len() - 1);
                        self.notify(ctx, Ok(format!("Imported {name}.")));
                    }
                    Err(e) => self.notify(ctx, Err(e)),
                }
            }
            Err(e) => self.notify(ctx, Err(format!("Import failed: {e}"))),
        }
    }

    fn export_preset(&mut self, ctx: &egui::Context) {
        let Some(mut path) = rfd::FileDialog::new()
            .add_filter("GMod Manager preset", &["zip"])
            .set_file_name(format!("{}.zip", self.profile().id))
            .save_file()
        else {
            return;
        };
        path.set_extension("zip");
        let result = core::export_preset_zip(self.profile(), &path);
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        self.notify(
            ctx,
            result.map(|_| format!("Saved {name}. Send it to a friend.")),
        );
    }

    fn browse_game(&mut self, ctx: &egui::Context) {
        let start = self
            .game()
            .map(|p| PathBuf::from(core::display_path(&p)))
            .unwrap_or_else(|| self.dir.clone());
        let Some(path) = rfd::FileDialog::new()
            .set_title("Choose your Garry's Mod folder")
            .set_directory(start)
            .pick_folder()
        else {
            return;
        };
        match core::normalize_game_path(&path) {
            Some(game) => {
                self.path_input = core::display_path(&game);
                self.remember();
                self.rescan();
                self.notify(ctx, Ok("Found Garry's Mod.".into()));
            }
            None => self.notify(ctx, Err("That isn't a Garry's Mod folder.".into())),
        }
    }

    fn capture_binds(&mut self, ctx: &egui::Context) {
        let result = self
            .game()
            .ok_or_else(|| "Set your GMod folder first.".to_owned())
            .and_then(|game| core::read_game_binds(&game));
        match result {
            Ok(binds) => {
                let n = binds.len();
                self.profile_mut().binds = binds;
                self.profile_mut().sync.binds = true;
                self.touch(ctx);
                self.notify(
                    ctx,
                    Ok(format!("Copied {n} binds. Untick any you don't want.")),
                );
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    fn capture_settings(&mut self, ctx: &egui::Context) {
        let result = self
            .game()
            .ok_or_else(|| "Set your GMod folder first.".to_owned())
            .and_then(|game| {
                core::capture_selected_settings(&game, &mut self.profiles[self.selected].convars)
            });
        match result {
            Ok(n) => {
                self.touch(ctx);
                self.notify(ctx, Ok(format!("Read {n} values from GMod.")));
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    fn browse_mapping(&mut self, ctx: &egui::Context, index: usize) {
        let Some(game) = self.game() else {
            self.notify(ctx, Err("Set your GMod folder first.".into()));
            return;
        };
        let mapped = game.join(&self.profile().file_mappings[index].path);
        let start = match mapped.parent() {
            Some(parent) if parent.is_dir() => parent.to_path_buf(),
            _ => game.join("cfg"),
        };
        let Some(path) = rfd::FileDialog::new()
            .set_directory(core::display_path(&start))
            .pick_file()
        else {
            return;
        };
        match core::capture_game_file(&game, &path) {
            Ok((relative, content)) => {
                let file = &mut self.profile_mut().file_mappings[index];
                file.path = relative;
                file.content = content;
                file.sync = true;
                self.touch(ctx);
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    fn read_mapping(&mut self, ctx: &egui::Context, index: usize) {
        let path = self.profile().file_mappings[index].path.clone();
        if path.trim().is_empty() {
            self.browse_mapping(ctx, index);
            return;
        }
        let result = self
            .game()
            .ok_or_else(|| "Set your GMod folder first.".to_owned())
            .and_then(|game| core::read_mapped_file(&game, &path));
        match result {
            Ok(content) => {
                let file = &mut self.profile_mut().file_mappings[index];
                file.content = content;
                file.sync = true;
                self.touch(ctx);
                self.notify(ctx, Ok(format!("Read {path}.")));
            }
            Err(e) => self.notify(ctx, Err(e)),
        }
    }

    // ---------- UI ----------

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("sidebar")
            .exact_width(236.0)
            .resizable(false)
            .frame(
                Frame::new()
                    .fill(SIDEBAR)
                    .inner_margin(Margin::symmetric(10, 14)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(semibold("GMod Manager", 15.0));
                });
                ui.add_space(18.0);
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.label(RichText::new("Presets").size(12.0).color(FAINT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add(icon_button("+"))
                            .on_hover_text("New preset")
                            .clicked()
                        {
                            self.new_preset = Some(String::new());
                        }
                        if ui
                            .add(
                                egui::Button::new(RichText::new("Import").size(12.0).color(MUTED))
                                    .frame(false),
                            )
                            .on_hover_text("Open a preset file someone shared")
                            .clicked()
                        {
                            self.import_preset(ctx);
                        }
                    });
                });
                ui.add_space(4.0);
                let bottom_space = if self.update.is_some() { 150.0 } else { 110.0 };
                let mut clicked = None;
                egui::ScrollArea::vertical()
                    .max_height((ui.available_height() - bottom_space).max(60.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for index in 0..self.profiles.len() {
                            if self.preset_entry(ui, index) {
                                clicked = Some(index);
                            }
                        }
                    });
                if let Some(index) = clicked {
                    self.select(index);
                }
                ui.with_layout(Layout::bottom_up(Align::LEFT), |ui| {
                    let found = self.game().is_some();
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        dot(ui, if found { GREEN } else { AMBER });
                        ui.label(
                            RichText::new(if found {
                                "GMod found"
                            } else {
                                "GMod folder not set"
                            })
                            .size(12.0)
                            .color(MUTED),
                        );
                    });
                    ui.add_space(8.0);
                    if let Some(version) = self.update.as_ref().map(|r| r.version.clone()) {
                        let label = format!("Update to {version}");
                        let response = nav_item(ui, &label, false);
                        ui.painter().circle_filled(
                            response.rect.right_center() - vec2(14.0, 0.0),
                            4.0,
                            ACCENT,
                        );
                        if response
                            .on_hover_text("Download the new version and restart")
                            .clicked()
                        {
                            self.start_update(ctx);
                        }
                    }
                    if nav_item(ui, "Settings", self.view == View::Settings).clicked() {
                        self.view = View::Settings;
                    }
                    let library_label = if self.library.is_empty() {
                        "Library".to_owned()
                    } else {
                        format!("Library  ·  {}", format_size(self.library_stored()))
                    };
                    if nav_item(ui, &library_label, self.view == View::Library).clicked() {
                        self.view = View::Library;
                    }
                });
            });
    }

    fn library_stored(&self) -> u64 {
        self.library.values().map(|e| e.stored_size).sum()
    }

    fn preset_entry(&mut self, ui: &mut egui::Ui, index: usize) -> bool {
        let active = self.view == View::Preset && index == self.selected;
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::click());
        let painter = ui.painter_at(rect);
        if active {
            painter.rect_filled(rect, 7.0, ROW);
            painter.rect_filled(
                Rect::from_min_size(rect.min + vec2(0.0, 12.0), vec2(3.0, rect.height() - 24.0)),
                2.0,
                ACCENT,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, 7.0, ROW.gamma_multiply(0.6));
        }
        let profile = &self.profiles[index];
        let first = profile
            .workshop
            .items
            .iter()
            .find(|i| i.available)
            .map(|i| i.id.clone());
        let name = profile.name.clone();
        let count = profile.workshop.items.len();
        let size = self.size_text(profile);
        let avatar = Rect::from_min_size(rect.min + vec2(10.0, 9.0), vec2(32.0, 32.0));
        match first {
            Some(id) => {
                let url = self
                    .meta
                    .get(&id)
                    .map(|m| m.preview_url.clone())
                    .unwrap_or_default();
                self.draw_icon(ui, avatar, &id, &url, &name, 6);
            }
            None => letter_tile(ui, avatar, &name, 6),
        }
        let text_left = avatar.right() + 10.0;
        let width = rect.right() - text_left - 8.0;
        paint_line(
            ui,
            pos2(text_left, rect.top() + 9.0),
            &name,
            FontId::proportional(14.0),
            if active {
                TEXT
            } else {
                TEXT.gamma_multiply(0.85)
            },
            width,
        );
        paint_line(
            ui,
            pos2(text_left, rect.top() + 28.0),
            &format!("{count} mods · {size}"),
            FontId::proportional(12.0),
            FAINT,
            width,
        );
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
    }

    fn draw_icon(
        &mut self,
        ui: &egui::Ui,
        rect: Rect,
        id: &str,
        url: &str,
        title: &str,
        radius: u8,
    ) {
        match self.icons.get(ui.ctx(), id, url) {
            Some(texture) => {
                egui::Image::from_texture((texture, rect.size()))
                    .corner_radius(radius)
                    .paint_at(ui, rect);
            }
            None => letter_tile(ui, rect, title, radius),
        }
    }

    fn header(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let profile = self.profile();
        let items = &profile.workshop.items;
        let unavailable = items.iter().filter(|i| !i.available).count();
        let missing = items
            .iter()
            .filter(|i| self.mod_state(i) == ModState::Missing)
            .count();
        let name = profile.name.clone();
        let summary = format!("{} mods · {}", items.len(), self.size_text(profile));
        let has_edits = !profile.optional_custom_files.source_url.is_empty();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(semibold(&name, 24.0));
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.label(RichText::new(summary).color(MUTED));
                    if missing > 0 {
                        ui.label(RichText::new(format!(" · {missing} to download")).color(MUTED));
                    }
                    if unavailable > 0 {
                        ui.label(RichText::new(format!(" · {unavailable} removed")).color(AMBER))
                            .on_hover_text(
                                "These were taken down from the Workshop and are skipped.",
                            );
                    }
                    if let Some(error) = &self.save_error {
                        ui.label(RichText::new(format!("  ·  Not saved: {error}")).color(RED));
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(job) = &self.job {
                    ui.vertical(|ui| {
                        ui.set_width(260.0);
                        ui.add(
                            egui::ProgressBar::new(job.fraction)
                                .desired_height(6.0)
                                .fill(ACCENT)
                                .corner_radius(3),
                        );
                        ui.label(RichText::new(&job.label).size(12.0).color(MUTED));
                    });
                    return;
                }
                ui.menu_button(RichText::new("…").size(18.0), |ui| {
                    ui.set_min_width(190.0);
                    if ui.button("Apply without launching").clicked() {
                        self.start_apply(ctx, false);
                    }
                    if ui.button("Review changes…").clicked() {
                        self.open_review();
                    }
                    if ui.button("Undo last apply").clicked() {
                        self.start_restore(ctx);
                    }
                    ui.separator();
                    if ui.button("Share as file…").clicked() {
                        self.export_preset(ctx);
                    }
                    if ui.button("Duplicate").clicked() {
                        let name = format!("{} copy", self.profile().name);
                        self.create_preset(ctx, &name, true);
                    }
                    ui.separator();
                    if ui.button("Open Workshop collection").clicked() {
                        let url = collection_url(self.profile());
                        self.open_link(ctx, &url);
                    }
                    if has_edits && ui.button("Open creator's edited files").clicked() {
                        let url = self.profile().optional_custom_files.source_url.clone();
                        self.open_link(ctx, &url);
                    }
                });
                ui.add_space(4.0);
                if ui
                    .add_enabled(
                        !self.busy(),
                        primary_button("▶  Play").min_size(vec2(112.0, 36.0)),
                    )
                    .on_hover_text("Apply this preset and start Garry's Mod")
                    .clicked()
                {
                    self.start_apply(ctx, true);
                }
            });
        });
    }

    fn open_link(&mut self, ctx: &egui::Context, url: &str) {
        if let Err(e) = open_url(url) {
            self.notify(ctx, Err(e));
        }
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        let profile = self.profile();
        let tabs = [
            (Tab::Mods, format!("Mods  {}", profile.workshop.items.len())),
            (Tab::Binds, format!("Binds  {}", profile.binds.len())),
            (
                Tab::Settings,
                format!(
                    "Settings  {}",
                    profile.convars.len() + profile.file_mappings.len()
                ),
            ),
            (Tab::Details, "Details".to_owned()),
        ];
        let top = ui.cursor().top();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 22.0;
            for (tab, label) in tabs {
                let active = self.tab == tab;
                let response = ui
                    .add(
                        egui::Label::new(RichText::new(label).size(14.0).color(if active {
                            TEXT
                        } else {
                            MUTED
                        }))
                        .sense(Sense::click()),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                if active {
                    ui.painter().hline(
                        response.rect.x_range(),
                        response.rect.bottom() + 7.0,
                        Stroke::new(2.0, ACCENT),
                    );
                }
                if response.clicked() {
                    self.tab = tab;
                }
            }
        });
        let y = ui.cursor().top().max(top + 26.0) + 1.0;
        ui.painter()
            .hline(ui.max_rect().x_range(), y, Stroke::new(1.0, LINE));
        ui.add_space(14.0);
    }

    fn preset_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        self.header(ui, ctx);
        ui.add_space(14.0);
        self.tabs(ui);
        match self.tab {
            Tab::Mods => self.mods_tab(ui, ctx),
            Tab::Binds => self.binds_tab(ui, ctx),
            Tab::Settings => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.settings_tab(ui, ctx));
            }
            Tab::Details => {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.details_tab(ui, ctx));
            }
        }
    }

    fn mods_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("Filter mods")
                    .desired_width(260.0),
            );
            if let Some(status) = &self.workshop_status {
                if status.profile == self.profile().id && status.total > 0 {
                    ui.label(
                        RichText::new(format!(
                            "Steam: {} of {} subscribed",
                            status.subscribed, status.total
                        ))
                        .size(12.0)
                        .color(FAINT),
                    )
                    .on_hover_text("Reported by GMod the last time it started.");
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if self.adding { "Done" } else { "+  Add mods" };
                if ui.add(soft_button(label)).clicked() {
                    self.adding = !self.adding;
                }
            });
        });
        ui.add_space(8.0);
        if self.adding {
            self.add_panel(ui, ctx);
            ui.add_space(10.0);
        }
        let filter = self.filter.trim().to_lowercase();
        let rows: Vec<Row> = self
            .profile()
            .workshop
            .items
            .iter()
            .filter(|item| {
                filter.is_empty()
                    || item.id.contains(&filter)
                    || self.title_of(item).to_lowercase().contains(&filter)
                    || self
                        .author_of(self.meta.get(&item.id))
                        .to_lowercase()
                        .contains(&filter)
            })
            .map(|item| {
                let meta = self.meta.get(&item.id);
                Row {
                    id: item.id.clone(),
                    title: self.title_of(item).to_owned(),
                    size: meta.map(|m| m.size).filter(|s| *s > 0),
                    author: self.author_of(meta).to_owned(),
                    state: self.mod_state(item),
                    url: meta.map(|m| m.preview_url.clone()).unwrap_or_default(),
                }
            })
            .collect();
        if rows.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new(if filter.is_empty() {
                        "No mods yet. Use Add mods to find some."
                    } else {
                        "Nothing matches that filter."
                    })
                    .color(MUTED),
                );
            });
            return;
        }
        let mut remove = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for row in &rows[range] {
                    if self.mod_row(ui, ctx, row) {
                        remove = Some(row.id.clone());
                    }
                }
            });
        if let Some(id) = remove {
            self.profile_mut()
                .workshop
                .items
                .retain(|item| item.id != id);
            self.touch(ctx);
        }
    }

    /// Draws one mod. Returns true when the user removes it.
    fn mod_row(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, row: &Row) -> bool {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click());
        let hovered = ui.rect_contains_pointer(rect);
        if hovered {
            ui.painter()
                .rect_filled(rect.shrink2(vec2(0.0, 2.0)), 7.0, ROW);
        }
        let icon = Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(36.0, 36.0));
        self.draw_icon(ui, icon, &row.id, &row.url, &row.title, 5);
        let dim = row.state == ModState::Unavailable;
        let right_reserved = 150.0;
        let text_left = icon.right() + 12.0;
        let width = rect.right() - text_left - right_reserved;
        paint_line(
            ui,
            pos2(text_left, rect.top() + 8.0),
            &row.title,
            FontId::proportional(14.0),
            if dim { FAINT } else { TEXT },
            width,
        );
        let (state_text, state_color) = row.state.describe();
        let painter = ui.painter();
        painter.circle_filled(pos2(text_left + 3.0, rect.top() + 34.0), 3.0, state_color);
        paint_line(
            ui,
            pos2(text_left + 11.0, rect.top() + 27.0),
            &if row.author.is_empty() {
                state_text.to_owned()
            } else {
                format!("by {} · {state_text}", row.author)
            },
            FontId::proportional(12.0),
            MUTED,
            width,
        );
        let mut removed = false;
        let size_right = if hovered {
            let button = Rect::from_center_size(
                pos2(rect.right() - 20.0, rect.center().y),
                vec2(26.0, 26.0),
            );
            if put_free(ui, button, icon_button("×"), true)
                .on_hover_text("Remove from preset")
                .clicked()
            {
                removed = true;
            }
            rect.right() - 44.0
        } else {
            rect.right() - 14.0
        };
        let size_text = row.size.map(format_size).unwrap_or_else(|| "—".into());
        ui.painter().text(
            pos2(size_right, rect.center().y),
            Align2::RIGHT_CENTER,
            size_text,
            FontId::proportional(13.0),
            if dim { FAINT } else { MUTED },
        );
        if let Some(entry) = self.library.get(&row.id) {
            response.clone().on_hover_text(format!(
                "In library: {} (full size {})",
                format_size(entry.stored_size),
                format_size(entry.raw_size)
            ));
        }
        let url = format!("{WORKSHOP_ITEM}{}", row.id);
        response.context_menu(|ui| {
            if ui.button("Open on Workshop").clicked() {
                self.open_link(ctx, &url);
            }
            if ui.button("Copy ID").clicked() {
                ui.ctx().copy_text(row.id.clone());
            }
            if ui.button("Remove from preset").clicked() {
                removed = true;
            }
        });
        if response.double_clicked() {
            self.open_link(ctx, &url);
        }
        removed
    }

    fn add_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        Frame::new()
            .fill(ROW)
            .corner_radius(10)
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text("Search the Workshop, or paste a link or ID")
                            .desired_width(ui.available_width() - 100.0),
                    );
                    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if self.search_rx.is_some() {
                        ui.add(egui::Spinner::new().size(18.0));
                    } else if ui.add(primary_button("Search")).clicked() || enter {
                        self.search(ctx, 1);
                    }
                });
                if self.add_rx.is_some() {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label("Checking required mods…");
                    });
                }
                if self.results.is_empty() {
                    return;
                }
                ui.add_space(8.0);
                let present: HashSet<String> = self
                    .profile()
                    .workshop
                    .items
                    .iter()
                    .map(|i| i.id.clone())
                    .collect();
                let mut add = None;
                egui::ScrollArea::vertical()
                    .id_salt("search-results")
                    .max_height(280.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for index in 0..self.results.len() {
                            let result = self.results[index].clone();
                            let (rect, response) = ui.allocate_exact_size(
                                vec2(ui.available_width(), 46.0),
                                Sense::click(),
                            );
                            if ui.rect_contains_pointer(rect) {
                                ui.painter().rect_filled(rect, 6.0, ROW_HOVER);
                            }
                            let icon =
                                Rect::from_min_size(rect.min + vec2(6.0, 7.0), vec2(32.0, 32.0));
                            self.draw_icon(
                                ui,
                                icon,
                                &result.id,
                                &result.meta.preview_url,
                                &result.meta.title,
                                5,
                            );
                            let left = icon.right() + 10.0;
                            let width = rect.right() - left - 110.0;
                            paint_line(
                                ui,
                                pos2(left, rect.top() + 6.0),
                                &result.meta.title,
                                FontId::proportional(14.0),
                                TEXT,
                                width,
                            );
                            paint_line(
                                ui,
                                pos2(left, rect.top() + 25.0),
                                &if self.author_of(Some(&result.meta)).is_empty() {
                                    format!(
                                        "{} · {} subscribers",
                                        format_size(result.meta.size),
                                        compact_number(result.meta.subscribers)
                                    )
                                } else {
                                    format!(
                                        "by {} · {} · {} subscribers",
                                        self.author_of(Some(&result.meta)),
                                        format_size(result.meta.size),
                                        compact_number(result.meta.subscribers)
                                    )
                                },
                                FontId::proportional(12.0),
                                FAINT,
                                width,
                            );
                            let button = Rect::from_min_size(
                                pos2(rect.right() - 86.0, rect.center().y - 14.0),
                                vec2(78.0, 28.0),
                            );
                            let added = present.contains(&result.id);
                            if put_free(
                                ui,
                                button,
                                egui::Button::new(if added { "Added" } else { "Add" })
                                    .selected(added),
                                self.add_rx.is_none() && !added,
                            )
                            .clicked()
                            {
                                add = Some(result.clone());
                            }
                            if response.double_clicked() {
                                self.open_link(ctx, &format!("{WORKSHOP_ITEM}{}", result.id));
                            }
                        }
                        if self.results.len() >= 30 * self.page && self.search_rx.is_none() {
                            ui.add_space(6.0);
                            if ui.add(soft_button("More results")).clicked() {
                                self.query = self.results_query.clone();
                                self.search(ctx, self.page + 1);
                            }
                        }
                    });
                if let Some(result) = add {
                    self.start_add(ctx, result);
                }
            });
    }

    fn binds_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut changed = false;
        ui.horizontal(|ui| {
            changed |= toggle_row(
                ui,
                &mut self.profiles[self.selected].sync.binds,
                "Apply binds",
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(soft_button("+  Add")).clicked() {
                    self.profiles[self.selected].binds.push(Bind {
                        key: String::new(),
                        command: String::new(),
                        sync: true,
                    });
                    changed = true;
                }
                if ui
                    .add(soft_button("Copy from GMod"))
                    .on_hover_text("Read your current binds from GMod's config.cfg")
                    .clicked()
                {
                    self.capture_binds(ctx);
                }
            });
        });
        ui.add_space(10.0);
        if self.profile().binds.is_empty() {
            empty_state(ui, "No binds yet. Copy yours from GMod or add one.");
        }
        let mut remove = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (index, bind) in self.profiles[self.selected].binds.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut bind.sync, "").changed();
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut bind.key)
                                    .hint_text("Key")
                                    .desired_width(90.0),
                            )
                            .changed();
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut bind.command)
                                    .hint_text("Command")
                                    .desired_width(ui.available_width() - 40.0),
                            )
                            .changed();
                        if ui.add(icon_button("×")).on_hover_text("Delete").clicked() {
                            remove = Some(index);
                        }
                    });
                }
            });
        if let Some(index) = remove {
            self.profile_mut().binds.remove(index);
            changed = true;
        }
        if changed {
            self.touch(ctx);
        }
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut changed = false;
        changed |= toggle_row(
            ui,
            &mut self.profiles[self.selected].sync.settings,
            "Apply settings and config files",
        );
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(semibold("Console variables", 15.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(soft_button("+  Add")).clicked() {
                    self.profiles[self.selected].convars.push(Setting {
                        name: String::new(),
                        value: String::new(),
                        sync: true,
                    });
                    changed = true;
                }
                if ui
                    .add(soft_button("Read values from GMod"))
                    .on_hover_text("Fill in the ticked variables from your current GMod config")
                    .clicked()
                {
                    self.capture_settings(ctx);
                }
            });
        });
        ui.add_space(4.0);
        if self.profile().convars.is_empty() {
            empty_state(ui, "e.g. fov_desired 90");
        }
        let mut remove = None;
        for (index, setting) in self.profiles[self.selected].convars.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut setting.sync, "").changed();
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut setting.name)
                            .hint_text("Name")
                            .desired_width(220.0),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut setting.value)
                            .hint_text("Value")
                            .desired_width(ui.available_width() - 40.0),
                    )
                    .changed();
                if ui.add(icon_button("×")).on_hover_text("Delete").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            self.profile_mut().convars.remove(index);
            changed = true;
        }
        ui.add_space(22.0);
        ui.horizontal(|ui| {
            ui.label(semibold("Config files", 15.0)).on_hover_text(
                "Files under cfg/, data/ or settings/ that mods save their options in.",
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(soft_button("+  Add")).clicked() {
                    self.profiles[self.selected]
                        .file_mappings
                        .push(FileMapping {
                            path: String::new(),
                            content: String::new(),
                            sync: true,
                        });
                    changed = true;
                }
            });
        });
        ui.add_space(4.0);
        if self.profile().file_mappings.is_empty() {
            empty_state(ui, "For mods that keep their options in a file.");
        }
        let mut remove = None;
        let mut browse = None;
        let mut read = None;
        for (index, file) in self.profiles[self.selected]
            .file_mappings
            .iter_mut()
            .enumerate()
        {
            Frame::new()
                .fill(ROW)
                .corner_radius(8)
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut file.sync, "").changed();
                        changed |= ui
                            .add(
                                egui::TextEdit::singleline(&mut file.path)
                                    .hint_text("data/mod/settings.txt")
                                    .desired_width(ui.available_width() - 190.0),
                            )
                            .changed();
                        if ui.add(soft_button("Browse")).clicked() {
                            browse = Some(index);
                        }
                        if ui
                            .add(soft_button("Read"))
                            .on_hover_text("Load this file's current contents from GMod")
                            .clicked()
                        {
                            read = Some(index);
                        }
                        if ui.add(icon_button("×")).on_hover_text("Delete").clicked() {
                            remove = Some(index);
                        }
                    });
                    egui::CollapsingHeader::new(
                        RichText::new(format!("Contents · {} lines", file.content.lines().count()))
                            .size(12.0)
                            .color(MUTED),
                    )
                    .id_salt(("file", index))
                    .show(ui, |ui| {
                        changed |= ui
                            .add(
                                egui::TextEdit::multiline(&mut file.content)
                                    .code_editor()
                                    .desired_rows(6)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                    });
                });
            ui.add_space(6.0);
        }
        if changed {
            self.touch(ctx);
        }
        if let Some(index) = browse {
            self.browse_mapping(ctx, index);
        }
        if let Some(index) = read {
            self.read_mapping(ctx, index);
        }
        if let Some(index) = remove {
            self.profile_mut().file_mappings.remove(index);
            self.touch(ctx);
        }
    }

    fn details_tab(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut changed = false;
        let width = ui.available_width().min(620.0);
        let profile = &mut self.profiles[self.selected];
        field_label(ui, "Name");
        changed |= ui
            .add(egui::TextEdit::singleline(&mut profile.name).desired_width(width))
            .changed();
        field_label(ui, "Description");
        changed |= ui
            .add(
                egui::TextEdit::multiline(&mut profile.description)
                    .desired_rows(3)
                    .desired_width(width),
            )
            .changed();
        field_label(ui, "Workshop collection ID");
        let mut refresh = false;
        ui.horizontal(|ui| {
            changed |= ui
                .add(
                    egui::TextEdit::singleline(&mut profile.workshop.collection_id)
                        .hint_text("e.g. 3428047388")
                        .desired_width(200.0),
                )
                .changed();
            if self.refresh_rx.is_some() {
                ui.add(egui::Spinner::new().size(16.0));
            } else if ui
                .add(soft_button("Update mods from collection"))
                .on_hover_text("Replace the collection's mods with its current contents. Mods you added yourself stay.")
                .clicked()
            {
                refresh = true;
            }
        });
        field_label(ui, "Video link");
        changed |= ui
            .add(
                egui::TextEdit::singleline(&mut profile.source.video_url)
                    .hint_text("https://")
                    .desired_width(width),
            )
            .changed();
        field_label(ui, "Creator's edited files");
        changed |= ui
            .add(
                egui::TextEdit::singleline(&mut profile.optional_custom_files.source_url)
                    .hint_text("Optional download link")
                    .desired_width(width),
            )
            .changed();
        ui.add_space(18.0);
        ui.label(semibold("When applying", 15.0));
        ui.add_space(6.0);
        changed |= toggle_row(ui, &mut profile.sync.addons, "Install and enable the mods");
        changed |= toggle_row(ui, &mut profile.sync.binds, "Apply binds");
        changed |= toggle_row(
            ui,
            &mut profile.sync.settings,
            "Apply settings and config files",
        );
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            if ui.add(soft_button("Share as file…")).clicked() {
                self.export_preset(ctx);
            }
            if ui.add(soft_button("Duplicate")).clicked() {
                let name = format!("{} copy", self.profile().name);
                self.create_preset(ctx, &name, true);
            }
        });
        if changed {
            self.touch(ctx);
        }
        if refresh {
            self.save_now();
            self.refresh_collection(ctx);
        }
    }

    fn library_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label(semibold("Library", 24.0));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(job) = &self.job {
                    ui.vertical(|ui| {
                        ui.set_width(260.0);
                        ui.add(
                            egui::ProgressBar::new(job.fraction)
                                .desired_height(6.0)
                                .fill(ACCENT),
                        );
                        ui.label(RichText::new(&job.label).size(12.0).color(MUTED));
                    });
                }
            });
        });
        ui.label(
            RichText::new("Download each mod once. Mods you aren't using are kept compressed.")
                .color(MUTED),
        );
        ui.add_space(16.0);
        let mut enabled = self.state.library_enabled;
        if toggle_row(ui, &mut enabled, "Use the library") {
            self.state.library_enabled = enabled;
            let _ = core::save_state(&self.dir, &self.state);
        }
        ui.label(
            RichText::new(if enabled {
                "Play saves each preset's mods here once. Saved mods you aren't using are dropped from Steam, then unpacked from here when you need them again."
            } else {
                "Off: Steam keeps every mod at full size."
            })
            .size(12.0)
            .color(FAINT),
        );
        ui.add_space(16.0);
        let stored = self.library_stored();
        let raw: u64 = self.library.values().map(|e| e.raw_size).sum();
        let steam_total: u64 = self.steam.values().map(|c| c.size).sum();
        ui.horizontal(|ui| {
            stat(ui, &self.library.len().to_string(), "mods stored");
            stat(ui, &format_size(stored), "on disk");
            stat(
                ui,
                &format_size(raw.saturating_sub(stored)),
                "saved by compression",
            );
            stat(ui, &format_size(steam_total), "held by Steam");
        });
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Folder").color(MUTED));
            let root = self.library_root();
            ui.label(
                RichText::new(core::display_path(&root))
                    .size(12.0)
                    .color(FAINT),
            );
            if ui
                .add_enabled(!self.busy(), soft_button("Change…"))
                .clicked()
            {
                self.change_library_folder(ctx);
            }
            if ui.add(soft_button("Open")).clicked() {
                let _ = std::fs::create_dir_all(&root);
                if let Err(e) = shell_open(&root.to_string_lossy()) {
                    self.notify(ctx, Err(e));
                }
            }
        });
        if self
            .library_root()
            .to_string_lossy()
            .to_ascii_lowercase()
            .contains("onedrive")
        {
            ui.label(
                RichText::new(
                    "This folder syncs to OneDrive. Pick a local folder so mods aren't uploaded.",
                )
                .size(12.0)
                .color(AMBER),
            );
        }
        ui.add_space(6.0);
        if ui
            .add_enabled(!self.busy(), soft_button("Save installed mods now"))
            .on_hover_text("Compress every preset mod Steam has already downloaded")
            .clicked()
        {
            self.start_store_all(ctx);
        }
        ui.add_space(14.0);
        ui.painter().hline(
            ui.max_rect().x_range(),
            ui.cursor().top(),
            Stroke::new(1.0, LINE),
        );
        ui.add_space(8.0);
        if self.library.is_empty() {
            empty_state(ui, "Nothing stored yet.");
            return;
        }
        let used: HashMap<&str, usize> = self
            .profiles
            .iter()
            .flat_map(|p| p.workshop.items.iter())
            .fold(HashMap::new(), |mut map, item| {
                *map.entry(item.id.as_str()).or_default() += 1;
                map
            });
        let rows: Vec<(String, LibraryEntry, usize)> = self
            .library
            .iter()
            .map(|(id, entry)| {
                (
                    id.clone(),
                    entry.clone(),
                    used.get(id.as_str()).copied().unwrap_or(0),
                )
            })
            .collect();
        let mut remove = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for (id, entry, presets) in &rows[range] {
                    let (rect, _) = ui.allocate_exact_size(
                        vec2(ui.available_width(), ROW_HEIGHT),
                        Sense::hover(),
                    );
                    let hovered = ui.rect_contains_pointer(rect);
                    if hovered {
                        ui.painter()
                            .rect_filled(rect.shrink2(vec2(0.0, 2.0)), 7.0, ROW);
                    }
                    let icon = Rect::from_min_size(rect.min + vec2(8.0, 7.0), vec2(36.0, 36.0));
                    let url = self
                        .meta
                        .get(id)
                        .map(|m| m.preview_url.clone())
                        .unwrap_or_default();
                    self.draw_icon(ui, icon, id, &url, &entry.title, 5);
                    let left = icon.right() + 12.0;
                    let width = rect.right() - left - 190.0;
                    paint_line(
                        ui,
                        pos2(left, rect.top() + 8.0),
                        &entry.title,
                        FontId::proportional(14.0),
                        TEXT,
                        width,
                    );
                    let status = if self.installed.contains_key(id) {
                        "Installed from library"
                    } else if self.steam.contains_key(id) {
                        "Also installed by Steam"
                    } else {
                        "Compressed"
                    };
                    let used_text = match presets {
                        0 => "not in any preset".to_owned(),
                        1 => "1 preset".to_owned(),
                        n => format!("{n} presets"),
                    };
                    paint_line(
                        ui,
                        pos2(left, rect.top() + 27.0),
                        &format!("{status} · {used_text}"),
                        FontId::proportional(12.0),
                        FAINT,
                        width,
                    );
                    let size_right = if hovered {
                        let button = Rect::from_center_size(
                            pos2(rect.right() - 20.0, rect.center().y),
                            vec2(26.0, 26.0),
                        );
                        if put_free(ui, button, icon_button("×"), !self.busy())
                            .on_hover_text("Delete from library")
                            .clicked()
                        {
                            remove = Some(id.clone());
                        }
                        rect.right() - 44.0
                    } else {
                        rect.right() - 14.0
                    };
                    ui.painter().text(
                        pos2(size_right, rect.center().y - 8.0),
                        Align2::RIGHT_CENTER,
                        format_size(entry.stored_size),
                        FontId::proportional(13.0),
                        MUTED,
                    );
                    ui.painter().text(
                        pos2(size_right, rect.center().y + 9.0),
                        Align2::RIGHT_CENTER,
                        format!("of {}", format_size(entry.raw_size)),
                        FontId::proportional(11.0),
                        FAINT,
                    );
                }
            });
        if let Some(id) = remove {
            self.remove_from_library(ctx, &id);
        }
    }

    fn settings_view(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(semibold("Settings", 24.0));
        ui.add_space(18.0);
        ui.label(semibold("Garry's Mod folder", 15.0));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let edit = ui.add(
                egui::TextEdit::singleline(&mut self.path_input)
                    .desired_width((ui.available_width() - 110.0).min(560.0)),
            );
            if edit.lost_focus() {
                self.remember();
                self.rescan();
            }
            if ui.add(soft_button("Browse…")).clicked() {
                self.browse_game(ctx);
            }
        });
        ui.horizontal(|ui| {
            let found = self.game().is_some();
            dot(ui, if found { GREEN } else { AMBER });
            ui.label(
                RichText::new(if found {
                    "Found"
                } else {
                    "Pick the GarrysMod folder inside steamapps\\common"
                })
                .size(12.0)
                .color(MUTED),
            );
        });
        ui.add_space(22.0);
        ui.label(semibold("Undo", 15.0));
        ui.add_space(4.0);
        ui.label(
            RichText::new("Puts back the GMod files the last apply changed.")
                .size(12.0)
                .color(MUTED),
        );
        ui.add_space(4.0);
        if ui
            .add_enabled(!self.busy(), soft_button("Undo last apply"))
            .clicked()
        {
            self.start_restore(ctx);
        }
        ui.add_space(22.0);
        ui.label(semibold("About", 15.0));
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!(
                "Version {}. Presets, backups and caches are stored next to the app, so the whole folder can be moved or zipped.",
                env!("CARGO_PKG_VERSION")
            ))
            .size(12.0)
            .color(MUTED),
        );
        ui.horizontal(|ui| {
            if let Some(release) = self.update.clone() {
                if ui
                    .add_enabled(
                        !self.busy(),
                        primary_button(&format!("Update to {}", release.version)),
                    )
                    .clicked()
                {
                    self.start_update(ctx);
                }
            } else if self.update_rx.is_some() {
                ui.add(egui::Spinner::new().size(16.0));
            } else if ui.add(soft_button("Check for updates")).clicked() {
                self.check_updates(ctx, true);
            }
            if ui.add(soft_button("Open app folder")).clicked() {
                if let Err(e) = shell_open(&self.dir.to_string_lossy()) {
                    self.notify(ctx, Err(e));
                }
            }
        });
    }

    fn new_preset_modal(&mut self, ctx: &egui::Context) {
        let Some(mut name) = self.new_preset.take() else {
            return;
        };
        let mut action = None;
        let modal = egui::Modal::new(Id::new("new-preset"))
            .frame(modal_frame())
            .show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(semibold("New preset", 18.0));
                ui.add_space(10.0);
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .hint_text("Name")
                        .desired_width(f32::INFINITY),
                );
                edit.request_focus();
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let ready = !name.trim().is_empty();
                    if ui.add_enabled(ready, primary_button("Create")).clicked() || (enter && ready)
                    {
                        action = Some(false);
                    }
                    if ui
                        .add_enabled(ready, soft_button("Copy current"))
                        .on_hover_text("Start from the selected preset")
                        .clicked()
                    {
                        action = Some(true);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(soft_button("Cancel")).clicked() {
                            action = Some(false);
                            name.clear();
                        }
                    });
                });
            });
        match action {
            Some(copy) if !name.trim().is_empty() => self.create_preset(ctx, &name, copy),
            Some(_) => {}
            None if modal.should_close() => {}
            None => self.new_preset = Some(name),
        }
    }

    fn review_modal(&mut self, ctx: &egui::Context) {
        let Some(review) = &self.review else {
            return;
        };
        let mut close = false;
        let mut run = None;
        let modal = egui::Modal::new(Id::new("review"))
            .frame(modal_frame())
            .show(ctx, |ui| {
                ui.set_width(520.0);
                ui.label(semibold("What Play will do", 18.0));
                ui.add_space(10.0);
                match review {
                    Err(e) => {
                        ui.label(RichText::new(e).color(RED));
                    }
                    Ok(prepared) => {
                        let plan = &prepared.plan;
                        let steam_new = plan
                            .subscribe
                            .iter()
                            .filter(|id| !self.steam.contains_key(*id))
                            .count();
                        let lines = [
                            (plan.store.len(), "compressed into the library"),
                            (plan.install.len(), "unpacked from the library"),
                            (steam_new, "downloaded by Steam"),
                            (plan.uninstall.len(), "removed from GMod (still in library)"),
                            (plan.unsubscribe.len(), "dropped from Steam (kept in library)"),
                        ];
                        let mut any = false;
                        for (count, text) in lines {
                            if count > 0 {
                                any = true;
                                ui.label(format!("{count} mods {text}"));
                            }
                        }
                        for note in &prepared.preview.notes {
                            any = true;
                            ui.label(note);
                        }
                        if !any && prepared.preview.changes.is_empty() {
                            ui.label(RichText::new("Nothing to change.").color(MUTED));
                        }
                        if !prepared.preview.changes.is_empty() {
                            ui.add_space(8.0);
                            ui.label(RichText::new("Files").size(12.0).color(FAINT));
                            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                                for change in &prepared.preview.changes {
                                    let name = change.relative.display().to_string().replace('\\', "/");
                                    let tag = if change.before.is_some() { "edit" } else { "new" };
                                    egui::CollapsingHeader::new(format!("{name}  ·  {tag}"))
                                        .id_salt(&name)
                                        .show(ui, |ui| {
                                            let text = String::from_utf8_lossy(&change.after);
                                            ui.label(
                                                RichText::new(text.chars().take(3000).collect::<String>())
                                                    .monospace()
                                                    .size(11.0)
                                                    .color(MUTED),
                                            );
                                        });
                                }
                            });
                        }
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Every changed file is backed up first. Settings → Undo puts it back.")
                                .size(12.0)
                                .color(FAINT),
                        );
                    }
                }
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    let ok = review.is_ok();
                    if ui.add_enabled(ok, primary_button("▶  Play")).clicked() {
                        run = Some(true);
                    }
                    if ui.add_enabled(ok, soft_button("Apply only")).clicked() {
                        run = Some(false);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(soft_button("Close")).clicked() {
                            close = true;
                        }
                    });
                });
            });
        if close || modal.should_close() {
            self.review = None;
        }
        if let Some(launch) = run {
            self.start_apply(ctx, launch);
        }
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let Some(toast) = &self.toast else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let life = if toast.error { 9.0 } else { 4.0 };
        if now - toast.shown_at > life {
            self.toast = None;
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        let mut dismiss = false;
        egui::Area::new(Id::new("toast"))
            .anchor(Align2::CENTER_BOTTOM, vec2(118.0, -22.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let response = Frame::new()
                    .fill(Color32::from_rgb(44, 48, 55))
                    .stroke(Stroke::new(1.0, if toast.error { RED } else { LINE }))
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 9))
                    .show(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.horizontal(|ui| {
                            dot(ui, if toast.error { RED } else { GREEN });
                            ui.label(RichText::new(&toast.text).color(TEXT));
                        });
                    })
                    .response
                    .interact(Sense::click());
                dismiss = response.clicked();
            });
        if dismiss {
            self.toast = None;
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.icons.poll(ctx);
        self.poll_meta();
        self.poll_authors();
        self.poll_search(ctx);
        self.poll_add(ctx);
        self.poll_refresh(ctx);
        self.poll_job(ctx);
        self.poll_update(ctx);
        self.ensure_meta(ctx);
        self.ensure_authors(ctx);
        self.autosave(ctx);
        let now = ctx.input(|i| i.time);
        if now - self.last_scan > 5.0 && self.job.is_none() {
            self.last_scan = now;
            self.rescan();
        }
        ctx.request_repaint_after(Duration::from_secs(5));
        if self.job.is_some()
            || self.search_rx.is_some()
            || self.refresh_rx.is_some()
            || self.add_rx.is_some()
        {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        self.sidebar(ctx);
        egui::CentralPanel::default()
            .frame(Frame::new().fill(BG).inner_margin(Margin {
                left: 28,
                right: 28,
                top: 22,
                bottom: 12,
            }))
            .show(ctx, |ui| match self.view {
                View::Preset => self.preset_view(ui, ctx),
                View::Library => self.library_view(ui, ctx),
                View::Settings => {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| self.settings_view(ui, ctx));
                }
            });
        self.new_preset_modal(ctx);
        self.review_modal(ctx);
        self.toast(ctx);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.edited_at.is_some() {
            self.save_now();
        }
        self.remember();
    }
}

// ---------- widgets & style ----------

fn semibold(text: &str, size: f32) -> RichText {
    RichText::new(text)
        .size(size)
        .color(TEXT)
        .family(FontFamily::Name("semibold".into()))
}

fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}

fn empty_state(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).color(FAINT));
    ui.add_space(6.0);
}

fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(
        RichText::new(text)
            .color(Color32::WHITE)
            .family(FontFamily::Name("semibold".into())),
    )
    .fill(ACCENT)
    .stroke(Stroke::NONE)
    .min_size(vec2(0.0, 30.0))
}

fn soft_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(TEXT))
        .fill(ROW_HOVER)
        .min_size(vec2(0.0, 30.0))
}

/// Places a widget at `rect` without moving the parent's cursor, so painted rows keep their height.
fn put_free(
    ui: &mut egui::Ui,
    rect: Rect,
    widget: impl egui::Widget,
    enabled: bool,
) -> egui::Response {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    if !enabled {
        child.disable();
    }
    child.add_sized(rect.size(), widget)
}

fn icon_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).size(17.0).color(MUTED))
        .frame(false)
        .min_size(vec2(26.0, 26.0))
}

fn nav_item(ui: &mut egui::Ui, text: &str, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    if active {
        ui.painter().rect_filled(rect, 7.0, ROW);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 7.0, ROW.gamma_multiply(0.6));
    }
    ui.painter().text(
        pos2(rect.left() + 12.0, rect.center().y),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        if active { TEXT } else { MUTED },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(8.0, 14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 3.5, color);
}

fn stat(ui: &mut egui::Ui, value: &str, label: &str) {
    Frame::new()
        .fill(ROW)
        .corner_radius(8)
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_min_width(120.0);
            ui.vertical(|ui| {
                ui.label(semibold(value, 18.0));
                ui.label(RichText::new(label).size(12.0).color(FAINT));
            });
        });
}

/// An animated on/off switch with a clickable label. Returns true when toggled.
fn toggle_row(ui: &mut egui::Ui, on: &mut bool, label: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(34.0, 18.0), Sense::click());
        let text = ui
            .add(egui::Label::new(RichText::new(label).color(TEXT)).sense(Sense::click()))
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked() || text.clicked() {
            *on = !*on;
            changed = true;
        }
        let t = ui.ctx().animate_bool_responsive(response.id, *on);
        let track = lerp_color(Color32::from_rgb(62, 67, 76), ACCENT, t);
        ui.painter().rect_filled(rect, 9.0, track);
        let x = egui::lerp(rect.left() + 9.0..=rect.right() - 9.0, t);
        ui.painter()
            .circle_filled(pos2(x, rect.center().y), 7.0, Color32::WHITE);
    });
    changed
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

/// Paints one line of text, cut off with an ellipsis if it doesn't fit.
fn paint_line(
    ui: &egui::Ui,
    pos: egui::Pos2,
    text: &str,
    font: FontId,
    color: Color32,
    width: f32,
) {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(font, color),
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width(width.max(10.0));
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().galley(pos, galley, color);
}

fn letter_tile(ui: &egui::Ui, rect: Rect, title: &str, radius: u8) {
    let letter = title
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".into());
    let hue = title
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let tints = [
        Color32::from_rgb(52, 70, 96),
        Color32::from_rgb(66, 58, 92),
        Color32::from_rgb(48, 80, 72),
        Color32::from_rgb(88, 64, 54),
        Color32::from_rgb(60, 66, 76),
    ];
    ui.painter()
        .rect_filled(rect, radius, tints[(hue % tints.len() as u32) as usize]);
    ui.painter().text(
        rect.center(),
        Align2::CENTER_CENTER,
        letter,
        FontId::proportional(rect.height() * 0.45),
        TEXT.gamma_multiply(0.85),
    );
}

fn modal_frame() -> Frame {
    Frame::new()
        .fill(Color32::from_rgb(30, 33, 38))
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(12)
        .inner_margin(Margin::same(20))
}

fn compact_number(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => format!("{:.1}k", n as f64 / 1_000.0).replace(".0k", "k"),
        _ => format!("{:.1}M", n as f64 / 1_000_000.0).replace(".0M", "M"),
    }
}

fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let fonts_dir = Path::new(&windir).join("Fonts");
    let mut load = |key: &str, file: &str| -> bool {
        match std::fs::read(fonts_dir.join(file)) {
            Ok(bytes) => {
                fonts
                    .font_data
                    .insert(key.to_owned(), Arc::new(FontData::from_owned(bytes)));
                true
            }
            Err(_) => false,
        }
    };
    let regular = load("segoe", "segoeui.ttf");
    let semibold = load("segoe-semibold", "seguisb.ttf");
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    if regular {
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .expect("proportional family")
            .insert(0, "segoe".into());
    }
    let mut heavy = fonts.families[&FontFamily::Proportional].clone();
    if semibold {
        heavy.insert(0, "segoe-semibold".into());
    }
    if heavy.is_empty() {
        heavy = fallback;
    }
    fonts
        .families
        .insert(FontFamily::Name("semibold".into()), heavy);
    ctx.set_fonts(fonts);
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = Color32::from_rgb(30, 33, 38);
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
    visuals.extreme_bg_color = INPUT;
    visuals.faint_bg_color = ROW;
    visuals.code_bg_color = INPUT;
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.5);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(6);
    let states = [
        (&mut visuals.widgets.inactive, ROW_HOVER, TEXT),
        (
            &mut visuals.widgets.hovered,
            Color32::from_rgb(48, 53, 61),
            TEXT,
        ),
        (
            &mut visuals.widgets.active,
            Color32::from_rgb(56, 62, 71),
            TEXT,
        ),
        (
            &mut visuals.widgets.open,
            Color32::from_rgb(48, 53, 61),
            TEXT,
        ),
    ];
    for (widget, fill, text) in states {
        widget.weak_bg_fill = fill;
        widget.bg_fill = fill;
        widget.bg_stroke = Stroke::NONE;
        widget.fg_stroke = Stroke::new(1.0, text);
        widget.corner_radius = CornerRadius::same(6);
        widget.expansion = 0.0;
    }
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(52, 57, 65);
    ctx.set_visuals(visuals);
    ctx.style_mut(|style| {
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(12.0, 5.0);
        style.spacing.interact_size.y = 26.0;
        style.spacing.menu_margin = Margin::same(6);
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, FontId::proportional(22.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
    });
}

fn shell_open(target: &str) -> Result<(), String> {
    use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
    let verb: Vec<u16> = "open".encode_utf16().chain([0]).collect();
    let target: Vec<u16> = target.encode_utf16().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        Err("Windows couldn't open that.".into())
    } else {
        Ok(())
    }
}

fn open_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("That link isn't a valid https:// address.".into());
    }
    shell_open(url)
}

fn collection_url(profile: &Profile) -> String {
    let id = &profile.workshop.collection_id;
    if !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()) {
        format!("{WORKSHOP_ITEM}{id}")
    } else {
        profile.source.collection_url.clone()
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([820.0, 540.0])
            .with_title("GMod Manager"),
        ..Default::default()
    };
    eframe::run_native(
        "GMod Manager",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
