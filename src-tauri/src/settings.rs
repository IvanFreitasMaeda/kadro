//! Preferences window: shortcuts and the captures folder.

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

use crate::{config, history, shortcuts};

const LABEL: &str = "settings";

pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let result = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("settings.html".into()))
        .title("Preferências do Kadro")
        .inner_size(480.0, 420.0)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .center()
        .focused(true)
        .build();
    match result {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(err) => log::error!("could not open preferences: {err}"),
    }
}

#[derive(Serialize)]
pub struct Settings {
    shortcut_area: String,
    shortcut_full: String,
    shortcut_gallery: String,
    shortcut_record: String,
    history_limit: usize,
    save_dir: String,
    platform: &'static str,
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    let save_dir = config::save_dir(&app).unwrap_or_else(|_| config::default_save_dir());
    Settings {
        shortcut_area: shortcuts::current(&app, shortcuts::Kind::Area),
        shortcut_full: shortcuts::current(&app, shortcuts::Kind::Full),
        shortcut_gallery: shortcuts::current(&app, shortcuts::Kind::Gallery),
        shortcut_record: shortcuts::current(&app, shortcuts::Kind::Record),
        history_limit: history::limit(&app),
        save_dir: save_dir.display().to_string(),
        platform: std::env::consts::OS,
    }
}

/// Saves a shortcut; `accelerator: null` restores the default. Returns the saved value.
#[tauri::command]
pub fn set_shortcut(
    app: AppHandle,
    kind: shortcuts::Kind,
    accelerator: Option<String>,
) -> Result<String, String> {
    shortcuts::set(&app, kind, accelerator)
}

/// Pauses (`true`) or resumes (`false`) the global shortcuts while one is being recorded.
#[tauri::command]
pub fn pause_shortcuts(app: AppHandle, paused: bool) {
    if paused {
        shortcuts::unregister_all(&app);
    } else {
        shortcuts::unregister_all(&app);
        shortcuts::register_all(&app);
    }
}

/// Opens a folder picker; returns the new folder, or `None` if cancelled.
#[tauri::command]
pub async fn choose_save_dir(app: AppHandle) -> Result<Option<String>, String> {
    let current = config::save_dir(&app).unwrap_or_else(|_| config::default_save_dir());
    let mut dialog = app
        .dialog()
        .file()
        .set_title("Escolha a pasta onde as capturas serão salvas")
        .set_directory(current);
    if let Some(window) = app.get_webview_window(LABEL) {
        dialog = dialog.set_parent(&window);
    }
    let Some(path) = dialog
        .blocking_pick_folder()
        .and_then(|p| p.as_path().map(|p| p.to_path_buf()))
    else {
        return Ok(None);
    };
    config::set_save_dir(&app, path.clone())?;
    Ok(Some(path.display().to_string()))
}
