//! Launch at login (macOS LaunchAgent, Windows "Run" registry key).
//!
//! Kadro starts with no window, only the tray icon, so launching at login is silent.
//! It is turned on once, on the first launch; after that the system's own state is
//! the truth, so turning it off in Preferences (or in the system settings) sticks.

use tauri::{plugin::TauriPlugin, AppHandle, Manager, Wry};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

use crate::config::{self, ConfigState};

pub fn plugin() -> TauriPlugin<Wry> {
    // LaunchAgent needs no Automation permission, unlike the AppleScript launcher.
    tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None)
}

/// Turns launch at login on the first time Kadro runs.
pub fn init(app: &AppHandle) {
    let state = app.state::<ConfigState>();
    let mut config = state.0.lock().unwrap();
    if config.autostart_initialized {
        return;
    }
    if let Err(err) = app.autolaunch().enable() {
        log::error!("could not enable launch at login: {err}");
        return;
    }
    config.autostart_initialized = true;
    let _ = config::store(app, &config);
}

pub fn enabled(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

/// Turns launch at login on or off; returns the resulting state.
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    let launcher = app.autolaunch();
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|e| format!("não foi possível alterar o início automático: {e}"))?;
    {
        let state = app.state::<ConfigState>();
        let mut config = state.0.lock().unwrap();
        if !config.autostart_initialized {
            config.autostart_initialized = true;
            let _ = config::store(&app, &config);
        }
    }
    Ok(self::enabled(&app))
}
