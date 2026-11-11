//! Global keyboard shortcuts.

use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::capture;

/// Select an area (Cmd+Shift+2 on macOS, Ctrl+Shift+2 on Windows).
pub const AREA: &str = "CommandOrControl+Shift+2";
/// Whole screen under the cursor, preselected.
pub const FULL: &str = "CommandOrControl+Shift+1";

pub fn plugin() -> tauri::plugin::TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let full = FULL.parse::<Shortcut>().is_ok_and(|s| &s == shortcut);
            capture::start(app.app_handle(), full);
        })
        .build()
}

pub fn register(app: &AppHandle) {
    for accelerator in [AREA, FULL] {
        if let Err(err) = app.global_shortcut().register(accelerator) {
            // Another app may own the combination; the tray icon still works.
            log::warn!("could not register {accelerator}: {err}");
        }
    }
}
