use serde::{Deserialize, Serialize};

fn yes() -> bool {
    true
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(default)]
    pub video_url: String,
    #[serde(default)]
    pub collection_url: String,
    #[serde(default)]
    pub captured_on: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WorkshopItem {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "yes")]
    pub available: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub manually_added: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Workshop {
    #[serde(default)]
    pub collection_id: String,
    #[serde(default)]
    pub items: Vec<WorkshopItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct OptionalCustomFiles {
    #[serde(default)]
    pub source_url: String,
    #[serde(default)]
    pub install_mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOptions {
    #[serde(default = "yes")]
    pub addons: bool,
    #[serde(default)]
    pub binds: bool,
    #[serde(default)]
    pub settings: bool,
}

impl Default for SyncOptions {
    fn default() -> Self {
        Self {
            addons: true,
            binds: false,
            settings: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Bind {
    pub key: String,
    pub command: String,
    #[serde(default = "yes")]
    pub sync: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub name: String,
    pub value: String,
    #[serde(default = "yes")]
    pub sync: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileMapping {
    pub path: String,
    pub content: String,
    #[serde(default = "yes")]
    pub sync: bool,
}

// Ticked rows count unless they are still completely blank (a row the user just added).
impl Bind {
    pub fn active(&self) -> bool {
        self.sync && !(self.key.trim().is_empty() && self.command.trim().is_empty())
    }
}

impl Setting {
    pub fn active(&self) -> bool {
        self.sync && !(self.name.trim().is_empty() && self.value.trim().is_empty())
    }
}

impl FileMapping {
    pub fn active(&self) -> bool {
        self.sync && !(self.path.trim().is_empty() && self.content.is_empty())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub workshop: Workshop,
    #[serde(default)]
    pub optional_custom_files: OptionalCustomFiles,
    #[serde(default)]
    pub sync: SyncOptions,
    #[serde(default)]
    pub binds: Vec<Bind>,
    #[serde(default)]
    pub convars: Vec<Setting>,
    #[serde(default)]
    pub file_mappings: Vec<FileMapping>,
}

impl Profile {
    pub fn blank(name: &str, id: &str) -> Self {
        Self {
            schema_version: 1,
            id: id.into(),
            name: name.into(),
            description: String::new(),
            source: Source::default(),
            workshop: Workshop::default(),
            optional_custom_files: OptionalCustomFiles::default(),
            sync: SyncOptions::default(),
            binds: vec![],
            convars: vec![],
            file_mappings: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalState {
    #[serde(default)]
    pub game_path: String,
    #[serde(default)]
    pub selected_profile: String,
    /// Keep mods in the compressed library and let Steam drop inactive ones.
    #[serde(default)]
    pub library_enabled: bool,
    /// Empty means `library/` beside the app.
    #[serde(default)]
    pub library_path: String,
    /// Show the Mods tab as thumbnail cards instead of a list.
    #[serde(default)]
    pub mods_grid: bool,
}
