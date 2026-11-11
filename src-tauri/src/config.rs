//! User settings, stored as JSON in the app config directory.

use std::{fs, path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Folder where captures are saved. `None` means the default (Pictures/Kadro).
    pub save_dir: Option<PathBuf>,
}

pub struct ConfigState(pub Mutex<Config>);

fn config_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("config.json"))
}

pub fn load(app: &AppHandle) -> Config {
    config_path(app)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn store(app: &AppHandle, config: &Config) -> Result<(), String> {
    let path = config_path(app).ok_or("pasta de configuração indisponível")?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

pub fn default_save_dir() -> PathBuf {
    dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Kadro")
}

/// The folder captures are saved to, created if missing.
pub fn save_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .state::<ConfigState>()
        .0
        .lock()
        .unwrap()
        .save_dir
        .clone()
        .unwrap_or_else(default_save_dir);
    fs::create_dir_all(&dir)
        .map_err(|e| format!("não foi possível criar {}: {e}", dir.display()))?;
    Ok(dir)
}

pub fn set_save_dir(app: &AppHandle, dir: PathBuf) -> Result<(), String> {
    let state = app.state::<ConfigState>();
    let mut config = state.0.lock().unwrap();
    config.save_dir = Some(dir);
    store(app, &config)
}
