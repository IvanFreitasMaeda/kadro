//! Global shortcuts on Wayland, through the XDG global shortcuts portal (GNOME 48+,
//! KDE Plasma, Hyprland). The desktop shows its own dialog the first time to confirm
//! the combinations, and may let the user pick others there.
//!
//! Where the portal is missing (sway and other wlroots desktops, older GNOME), the
//! user binds commands like `kadro --area` in the system's keyboard settings instead:
//! see `crate::shortcuts::run_args`.

use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Duration,
};

use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::{future::Either, StreamExt};
use serde::Serialize;
use tauri::AppHandle;

use crate::shortcuts::{self, Kind};

/// How the shortcuts work on this desktop, for the preferences window.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum Status {
    /// Not on Wayland: the usual X11 shortcuts are in use.
    X11,
    /// Asking the portal.
    Pending,
    /// The portal took them; `triggers` is what the desktop says each one is.
    Portal { triggers: Vec<(Kind, String)> },
    /// No portal: bind the commands in the system settings.
    Commands { reason: String },
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
/// Bumped on each (re)bind; an older session stops when it sees a newer number.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn status() -> Status {
    STATUS.lock().unwrap().clone().unwrap_or(Status::X11)
}

fn set_status(status: Status) {
    *STATUS.lock().unwrap() = Some(status);
}

/// The actions bound through the portal. Drawing is left out: it only matters while
/// recording, and the portal can't add and drop a shortcut on the fly without
/// asking again. The pencil on the recording bar does it instead.
const KINDS: [Kind; 6] = [
    Kind::Area,
    Kind::Full,
    Kind::Gallery,
    Kind::Record,
    Kind::Text,
    Kind::Clipboard,
];

fn id(kind: Kind) -> &'static str {
    match kind {
        Kind::Area => "area",
        Kind::Full => "full",
        Kind::Gallery => "gallery",
        Kind::Record => "record",
        Kind::Text => "text",
        Kind::Clipboard => "clipboard",
        Kind::Draw => "draw",
    }
}

fn description(kind: Kind) -> &'static str {
    match kind {
        Kind::Area => "Capturar uma área",
        Kind::Full => "Capturar a tela inteira",
        Kind::Gallery => "Abrir o histórico de capturas",
        Kind::Record => "Iniciar ou parar uma gravação",
        Kind::Text => "Copiar texto de uma área",
        Kind::Clipboard => "Abrir o histórico da área de transferência",
        Kind::Draw => "Desenhar durante a gravação",
    }
}

/// Turns a Tauri accelerator ("CommandOrControl+Shift+2") into the XDG shortcut
/// format the portal expects ("CTRL+SHIFT+2").
pub fn to_xdg(accelerator: &str) -> String {
    accelerator
        .split('+')
        .map(|part| {
            let lower = part.trim().to_ascii_lowercase();
            match lower.as_str() {
                "commandorcontrol" | "cmdorctrl" | "commandorctrl" | "cmdorcontrol"
                | "control" | "ctrl" => "CTRL".to_string(),
                "shift" => "SHIFT".to_string(),
                "alt" | "option" | "altgr" => "ALT".to_string(),
                "super" | "meta" | "cmd" | "command" | "logo" => "LOGO".to_string(),
                _ => {
                    let key = part.trim();
                    let key = key
                        .strip_prefix("Digit")
                        .or_else(|| key.strip_prefix("Key"))
                        .unwrap_or(key);
                    match key.to_ascii_lowercase().as_str() {
                        "space" => "space".into(),
                        "enter" | "return" => "Return".into(),
                        "escape" | "esc" => "Escape".into(),
                        "tab" => "Tab".into(),
                        "backspace" => "BackSpace".into(),
                        "delete" => "Delete".into(),
                        "printscreen" | "print" => "Print".into(),
                        k if k.len() == 1 => k.to_string(),
                        _ => key.to_string(),
                    }
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Binds the configured shortcuts through the portal, replacing an earlier session.
pub fn bind(app: &AppHandle) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    set_status(Status::Pending);
    let wanted: Vec<(Kind, String)> = KINDS
        .into_iter()
        .map(|k| (k, to_xdg(&shortcuts::current(app, k))))
        .collect();
    let app = app.clone();
    std::thread::Builder::new()
        .name("portal-shortcuts".into())
        .spawn(move || {
            if let Err(reason) = pollster::block_on(session(&app, generation, wanted)) {
                log::warn!("global shortcuts portal: {reason}");
                if GENERATION.load(Ordering::SeqCst) == generation {
                    set_status(Status::Commands { reason });
                }
            }
        })
        .ok();
}

async fn session(app: &AppHandle, generation: u64, wanted: Vec<(Kind, String)>) -> Result<(), String> {
    let proxy = GlobalShortcuts::new().await.map_err(|e| e.to_string())?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|e| e.to_string())?;
    let new: Vec<NewShortcut> = wanted
        .iter()
        .map(|(kind, trigger)| {
            NewShortcut::new(id(*kind), description(*kind)).preferred_trigger(trigger.as_str())
        })
        .collect();
    let bound = proxy
        .bind_shortcuts(&session, &new, None, Default::default())
        .await
        .map_err(|e| e.to_string())?
        .response()
        .map_err(|e| e.to_string())?;
    let triggers = bound
        .shortcuts()
        .iter()
        .filter_map(|s| {
            let kind = KINDS.into_iter().find(|&k| id(k) == s.id())?;
            Some((kind, s.trigger_description().to_string()))
        })
        .collect();
    if GENERATION.load(Ordering::SeqCst) != generation {
        let _ = session.close().await;
        return Ok(());
    }
    set_status(Status::Portal { triggers });

    let mut activated = proxy.receive_activated().await.map_err(|e| e.to_string())?;
    loop {
        let tick = async_io::Timer::after(Duration::from_secs(1));
        match futures_util::future::select(activated.next(), tick).await {
            Either::Left((Some(event), _)) => {
                if let Some(kind) = KINDS.into_iter().find(|&k| id(k) == event.shortcut_id()) {
                    let app = app.clone();
                    // Back on the main thread, like the X11 shortcuts.
                    let handle = app.clone();
                    let _ = handle.run_on_main_thread(move || shortcuts::run(&app, kind));
                }
            }
            Either::Left((None, _)) => return Err("o portal fechou a sessão".into()),
            Either::Right(_) => {}
        }
        if GENERATION.load(Ordering::SeqCst) != generation {
            let _ = session.close().await;
            return Ok(());
        }
    }
}

/// Tells the desktop which app is talking to the portals, so its dialogs show
/// "Kadro" and remember what the user allowed. Best effort.
pub fn register_app(identifier: &str) {
    let Ok(app_id) = identifier.parse::<ashpd::AppID>() else {
        return;
    };
    let result = pollster::block_on(async {
        futures_util::future::select(
            Box::pin(ashpd::register_host_app(app_id)),
            async_io::Timer::after(Duration::from_secs(2)),
        )
        .await
    });
    if let Either::Left((Err(err), _)) = result {
        log::debug!("portal registry: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::to_xdg;

    #[test]
    fn converts_accelerators() {
        assert_eq!(to_xdg("CommandOrControl+Shift+2"), "CTRL+SHIFT+2");
        assert_eq!(to_xdg("CommandOrControl+Shift+V"), "CTRL+SHIFT+v");
        assert_eq!(to_xdg("Alt+Super+KeyA"), "ALT+LOGO+a");
        assert_eq!(to_xdg("Ctrl+Digit9"), "CTRL+9");
        assert_eq!(to_xdg("Shift+F5"), "SHIFT+F5");
        assert_eq!(to_xdg("PrintScreen"), "Print");
    }
}
