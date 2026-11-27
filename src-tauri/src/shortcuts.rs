//! Global keyboard shortcuts, configurable in the preferences window.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::{capture, clipboard, config, gallery, record, tray};

/// Select an area (Cmd+Shift+2 on macOS, Ctrl+Shift+2 on Windows).
pub const DEFAULT_AREA: &str = "CommandOrControl+Shift+2";
/// Whole screen under the cursor, preselected.
pub const DEFAULT_FULL: &str = "CommandOrControl+Shift+1";
/// History gallery. Not 3, 4 or 5: macOS uses Cmd+Shift+3/4/5 for its own screenshots.
pub const DEFAULT_GALLERY: &str = "CommandOrControl+Shift+0";
/// Start a recording, or stop the one running.
pub const DEFAULT_RECORD: &str = "CommandOrControl+Shift+9";
/// Copy the text in an area (OCR).
pub const DEFAULT_TEXT: &str = "CommandOrControl+Shift+8";
/// Clipboard history, like Windows' Win+V.
pub const DEFAULT_CLIPBOARD: &str = "CommandOrControl+Shift+V";
/// Draw on the screen while recording. Only registered during a recording, so it
/// doesn't take the combination away from other apps the rest of the time.
pub const DEFAULT_DRAW: &str = "CommandOrControl+Shift+D";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Area,
    Full,
    Gallery,
    Record,
    Text,
    Clipboard,
    Draw,
}

pub const ALL: [Kind; 7] = [
    Kind::Area,
    Kind::Full,
    Kind::Gallery,
    Kind::Record,
    Kind::Text,
    Kind::Clipboard,
    Kind::Draw,
];

/// The accelerator currently configured for `kind`.
pub fn current(app: &AppHandle, kind: Kind) -> String {
    let state = app.state::<config::ConfigState>();
    let config = state.0.lock().unwrap();
    match kind {
        Kind::Area => config.shortcut_area.clone(),
        Kind::Full => config.shortcut_full.clone(),
        Kind::Gallery => config.shortcut_gallery.clone(),
        Kind::Record => config.shortcut_record.clone(),
        Kind::Text => config.shortcut_text.clone(),
        Kind::Clipboard => config.shortcut_clipboard.clone(),
        Kind::Draw => config.shortcut_draw.clone(),
    }
    .unwrap_or_else(|| default(kind).to_string())
}

fn default(kind: Kind) -> &'static str {
    match kind {
        Kind::Area => DEFAULT_AREA,
        Kind::Full => DEFAULT_FULL,
        Kind::Gallery => DEFAULT_GALLERY,
        Kind::Record => DEFAULT_RECORD,
        Kind::Text => DEFAULT_TEXT,
        Kind::Clipboard => DEFAULT_CLIPBOARD,
        Kind::Draw => DEFAULT_DRAW,
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
            let kind = kind_of(app, shortcut).unwrap_or(Kind::Area);
            // Linux calls this on the X11 key-grab thread, and an action that
            // registers or drops a shortcut (stopping a recording drops the draw
            // one) would wait on that same thread forever. Run it elsewhere.
            if cfg!(target_os = "linux") {
                let app = app.clone();
                std::thread::spawn(move || run(&app, kind));
            } else {
                run(app, kind);
            }
        })
        .build()
}

/// Does what a shortcut's action does.
pub fn run(app: &AppHandle, kind: Kind) {
    match kind {
        Kind::Gallery => gallery::toggle(app),
        Kind::Record => record::toggle(app),
        Kind::Full => capture::start(app, true),
        Kind::Text => capture::start_text(app),
        Kind::Clipboard => clipboard::toggle(app),
        Kind::Draw => record::draw::toggle(app),
        Kind::Area => capture::start(app, false),
    }
}

/// The action named on the command line (`kadro --area`, `--full`, `--record`,
/// `--text`, `--gallery`, `--clipboard`, `--draw`), if any. Lets the shortcuts be set
/// in the system's own settings where Kadro can't register them (Wayland without the
/// shortcuts portal), or from scripts.
pub fn kind_from_args<S: AsRef<str>>(args: &[S]) -> Option<Kind> {
    args.iter().skip(1).find_map(|arg| match arg.as_ref() {
        "--area" | "--capture" => Some(Kind::Area),
        "--full" => Some(Kind::Full),
        "--record" => Some(Kind::Record),
        "--text" | "--ocr" => Some(Kind::Text),
        "--gallery" | "--history" => Some(Kind::Gallery),
        "--clipboard" => Some(Kind::Clipboard),
        "--draw" => Some(Kind::Draw),
        _ => None,
    })
}

/// Wayland: the X11 key grabs Kadro's windows get through XWayland only work while
/// one of them has the focus, so the shortcuts go through the portal instead.
fn use_portal() -> bool {
    cfg!(target_os = "linux") && is_wayland()
}

#[cfg(target_os = "linux")]
fn is_wayland() -> bool {
    crate::linux::is_wayland()
}

#[cfg(not(target_os = "linux"))]
fn is_wayland() -> bool {
    false
}

/// Registers the configured shortcuts. Failures are logged: another app may own the
/// combination, and the tray icon still works.
pub fn register_all(app: &AppHandle) {
    if use_portal() {
        #[cfg(target_os = "linux")]
        crate::linux::shortcuts::bind(app);
        return;
    }
    for kind in ALL {
        if kind == Kind::Draw && !record::is_recording(app) {
            continue;
        }
        let accelerator = current(app, kind);
        if let Err(err) = app.global_shortcut().register(accelerator.as_str()) {
            log::warn!("could not register {accelerator}: {err}");
        }
    }
}

/// Registers (`true`) or drops the draw shortcut when a recording starts or stops.
pub fn set_draw(app: &AppHandle, on: bool) {
    if use_portal() {
        return;
    }
    let accelerator = current(app, Kind::Draw);
    let result = if on {
        app.global_shortcut().register(accelerator.as_str())
    } else {
        app.global_shortcut().unregister(accelerator.as_str())
    };
    if let Err(err) = result {
        log::warn!("draw shortcut {accelerator}: {err}");
    }
}

/// Turns the shortcuts off while the user records a new one, so pressing the old
/// combination doesn't start a capture.
pub fn unregister_all(app: &AppHandle) {
    if use_portal() {
        return;
    }
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
    // app already owns. (Through the portal, the desktop decides when binding.)
    if !use_portal() {
        unregister_all(app);
        if let Err(err) = app.global_shortcut().register(parsed) {
            register_all(app);
            return Err(format!(
                "Não foi possível usar esse atalho. Talvez outro app já use ({err})."
            ));
        }
        let _ = app.global_shortcut().unregister(parsed);
    }

    {
        let state = app.state::<config::ConfigState>();
        let mut config = state.0.lock().unwrap();
        let value = (accelerator != default(kind)).then(|| accelerator.clone());
        match kind {
            Kind::Area => config.shortcut_area = value,
            Kind::Full => config.shortcut_full = value,
            Kind::Gallery => config.shortcut_gallery = value,
            Kind::Record => config.shortcut_record = value,
            Kind::Text => config.shortcut_text = value,
            Kind::Clipboard => config.shortcut_clipboard = value,
            Kind::Draw => config.shortcut_draw = value,
        }
        config::store(app, &config)?;
    }
    register_all(app);
    tray::refresh_accelerators(app);
    Ok(accelerator)
}
