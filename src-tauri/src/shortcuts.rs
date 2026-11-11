//! Global keyboard shortcuts, configurable in the preferences window.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{capture, config, gallery, tray};

/// Select an area (Cmd+Shift+2 on macOS, Ctrl+Shift+2 on Windows).
pub const DEFAULT_AREA: &str = "CommandOrControl+Shift+2";
/// Whole screen under the cursor, preselected.
pub const DEFAULT_FULL: &str = "CommandOrControl+Shift+1";
/// History gallery. Not 3, 4 or 5: macOS uses Cmd+Shift+3/4/5 for its own screenshots.
pub const DEFAULT_GALLERY: &str = "CommandOrControl+Shift+0";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Area,
    Full,
    Gallery,
}

pub const ALL: [Kind; 3] = [Kind::Area, Kind::Full, Kind::Gallery];

/// The accelerator currently configured for `kind`.
pub fn current(app: &AppHandle, kind: Kind) -> String {
    let state = app.state::<config::ConfigState>();
    let config = state.0.lock().unwrap();
    match kind {
        Kind::Area => config.shortcut_area.clone(),
        Kind::Full => config.shortcut_full.clone(),
        Kind::Gallery => config.shortcut_gallery.clone(),
    }
    .unwrap_or_else(|| default(kind).to_string())
}

fn default(kind: Kind) -> &'static str {
    match kind {
        Kind::Area => DEFAULT_AREA,
        Kind::Full => DEFAULT_FULL,
        Kind::Gallery => DEFAULT_GALLERY,
    }
}

/// Which action the pressed shortcut belongs to.
fn kind_of(app: &AppHandle, shortcut: &Shortcut) -> Option<Kind> {
    ALL.into_iter().find(|&kind| {
        current(app, kind)
            .parse::<Shortcut>()
            .is_ok_and(|s| &s == shortcut)
    })
}

pub fn plugin() -> tauri::plugin::TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let app = app.app_handle();
            match kind_of(app, shortcut) {
                Some(Kind::Gallery) => gallery::toggle(app),
                Some(Kind::Full) => capture::start(app, true),
                _ => capture::start(app, false),
            }
        })
        .build()
}

/// Registers the configured shortcuts. Failures are logged: another app may own the
/// combination, and the tray icon still works.
pub fn register_all(app: &AppHandle) {
    for kind in ALL {
        let accelerator = current(app, kind);
        if let Err(err) = app.global_shortcut().register(accelerator.as_str()) {
            log::warn!("could not register {accelerator}: {err}");
        }
    }
}

/// Turns the shortcuts off while the user records a new one, so pressing the old
/// combination doesn't start a capture.
pub fn unregister_all(app: &AppHandle) {
    if let Err(err) = app.global_shortcut().unregister_all() {
        log::warn!("could not unregister shortcuts: {err}");
    }
}

/// Validates, saves and applies a new shortcut. `None` restores the default.
pub fn set(app: &AppHandle, kind: Kind, accelerator: Option<String>) -> Result<String, String> {
    let accelerator = accelerator.unwrap_or_else(|| default(kind).to_string());
    let parsed: Shortcut = accelerator
        .parse()
        .map_err(|_| format!("Atalho inválido: {accelerator}"))?;
    if kind_of(app, &parsed).is_some_and(|other| other != kind) {
        return Err("Esse atalho já está sendo usado por outra ação do Kadro.".into());
    }

    // Try the new combination before saving it: the OS refuses ones that another
    // app already owns.
    unregister_all(app);
    if let Err(err) = app.global_shortcut().register(parsed) {
        register_all(app);
        return Err(format!(
            "Não foi possível usar esse atalho. Talvez outro app já use ({err})."
        ));
    }
    let _ = app.global_shortcut().unregister(parsed);

    {
        let state = app.state::<config::ConfigState>();
        let mut config = state.0.lock().unwrap();
        let value = (accelerator != default(kind)).then(|| accelerator.clone());
        match kind {
            Kind::Area => config.shortcut_area = value,
            Kind::Full => config.shortcut_full = value,
            Kind::Gallery => config.shortcut_gallery = value,
        }
        config::store(app, &config)?;
    }
    register_all(app);
    tray::refresh_accelerators(app);
    Ok(accelerator)
}
