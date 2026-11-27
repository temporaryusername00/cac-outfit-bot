//! config.json / outfits.json / published.json: the same files the Python version used.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Step {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub click: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expect_not: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub times: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_for: Option<String>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publishes: Option<bool>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Flow {
    #[serde(default)]
    pub start: Vec<Step>,
    #[serde(default)]
    pub per_item: Vec<Step>,
    #[serde(default)]
    pub finish: Vec<Step>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Recovery {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub close: BTreeMap<String, String>,
}

fn default_tol() -> i32 {
    30
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateSpec {
    #[serde(default = "default_tol")]
    pub tol: i32,
    pub pixels: Vec<(i32, i32, [u8; 3])>,
}

fn default_port() -> u16 {
    7963
}
fn default_place() -> u64 {
    7041939546
}
fn d8() -> f64 {
    8.0
}
fn d90() -> f64 {
    90.0
}
fn d30() -> f64 {
    30.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RamCfg {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub password: String,
    #[serde(default = "default_place")]
    pub place_id: u64,
    #[serde(default = "d8")]
    pub seconds_between_launches: f64,
    #[serde(default = "d90")]
    pub window_timeout: f64,
    #[serde(default = "d30")]
    pub game_load_seconds: f64,
}

impl Default for RamCfg {
    fn default() -> Self {
        RamCfg {
            port: 7963,
            password: String::new(),
            place_id: default_place(),
            seconds_between_launches: 8.0,
            window_timeout: 90.0,
            game_load_seconds: 30.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The one open Roblox window.
    #[default]
    Single,
    /// Every Roblox window that is already open.
    Attach,
    /// Launch the selected accounts through Roblox Account Manager.
    Launch,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GuiPrefs {
    #[serde(default)]
    pub mode: Mode,
    #[serde(default)]
    pub test_mode: bool,
    #[serde(default)]
    pub check_items: bool,
    /// Minimize the app while the bot runs, so it never covers a Roblox window.
    #[serde(default = "yes")]
    pub minimize_while_running: bool,
    /// Every account name the Accounts tab knows (config.accounts = the ticked ones).
    #[serde(default)]
    pub known_accounts: Vec<String>,
}

fn yes() -> bool {
    true
}

fn default_title() -> String {
    "Roblox".into()
}
fn d20() -> u32 {
    20
}
fn d3() -> u32 {
    3
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_title")]
    pub window_title: String,
    #[serde(default)]
    pub accounts: Vec<String>,
    #[serde(default)]
    pub ram: RamCfg,
    #[serde(default)]
    pub launch_window_size: Option<[i32; 2]>,
    #[serde(default = "d20")]
    pub max_publish_per_run: u32,
    #[serde(default = "d30")]
    pub seconds_between_outfits: f64,
    #[serde(default = "d3")]
    pub max_failures_per_window: u32,
    #[serde(default)]
    pub delays: BTreeMap<String, f64>,
    #[serde(default)]
    pub point_hints: BTreeMap<String, String>,
    #[serde(default)]
    pub flow: Flow,
    #[serde(default)]
    pub recovery: Recovery,
    #[serde(default)]
    pub points: BTreeMap<String, BTreeMap<String, [i32; 2]>>,
    #[serde(default)]
    pub states: BTreeMap<String, BTreeMap<String, StateSpec>>,
    #[serde(default)]
    pub gui: GuiPrefs,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Config {
    pub fn delay(&self, name: &str, default: f64) -> f64 {
        self.delays.get(name).copied().unwrap_or(default)
    }

    /// Window sizes that have both button positions and screen checks.
    pub fn sizes(&self) -> Vec<String> {
        self.points.keys().filter(|k| self.states.contains_key(*k)).cloned().collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Outfit {
    pub name: String,
    pub items: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct OutfitsFile {
    #[serde(default)]
    pub outfits: Vec<Outfit>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishedEntry {
    pub name: String,
    pub account: String,
    pub at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct PublishedFile {
    #[serde(default)]
    pub published: Vec<PublishedEntry>,
}

/// Folder holding config.json: next to the exe, one of its parents, or the working directory.
/// Settings, outfits and accounts live in `<root>/data`.
pub fn data(root: &Path) -> PathBuf {
    root.join("data")
}

/// bot.log and failure screenshots live in `<root>/logs`.
pub fn logs(root: &Path) -> PathBuf {
    root.join("logs")
}

/// Move files from the old layout (everything next to the exe) into data/ and logs/.
fn migrate(root: &Path) {
    let data = data(root);
    let logs = logs(root);
    let _ = std::fs::create_dir_all(&data);
    let _ = std::fs::create_dir_all(&logs);
    for f in ["config.json", "outfits.json", "published.json", "accounts.json"] {
        let (old, new) = (root.join(f), data.join(f));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
    for f in ["bot.log", "failures"] {
        let (old, new) = (root.join(f), logs.join(f));
        if old.exists() && !new.exists() {
            let _ = std::fs::rename(&old, &new);
        }
    }
}

/// The app's folder: the one with `data/config.json` in it (searched upwards from the exe).
pub fn data_dir() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(find_root).clone()
}

fn find_root() -> PathBuf {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        let mut dir = exe.parent().map(Path::to_path_buf);
        for _ in 0..5 {
            if let Some(d) = dir {
                candidates.push(d.clone());
                dir = d.parent().map(Path::to_path_buf);
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd);
    }
    if let Some(d) = candidates.iter().find(|d| data(d).join("config.json").exists()) {
        return d.clone();
    }
    if let Some(d) = candidates.iter().find(|d| d.join("config.json").exists()) {
        migrate(d);
        return d.clone();
    }
    candidates.first().cloned().unwrap_or_else(|| PathBuf::from("."))
}

pub fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = text.trim_start_matches('\u{feff}');
    serde_json::from_str(text).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn load_config(dir: &Path) -> Result<Config, String> {
    let path = data(dir).join("config.json");
    if !path.exists() {
        return Err(format!("config.json not found in {}", data(dir).display()));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("config.json: {e}"))
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Parse "123, 456 789" or catalog links into item IDs.
pub fn parse_ids(text: &str) -> Vec<u64> {
    text.split(|c: char| !c.is_ascii_digit())
        .filter(|s| s.len() >= 3)
        .filter_map(|s| s.parse().ok())
        .collect()
}
