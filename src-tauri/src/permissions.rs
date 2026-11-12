//! macOS Screen Recording permission.
//!
//! Without it, macOS hands back black (or wallpaper-only) screenshots instead of an
//! error. So we check before capturing and, when it is missing, send the user to the
//! right settings pane instead of showing a useless black overlay.

use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_opener::OpenerExt;

use crate::config;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Returns true when Kadro may capture the screen. Otherwise asks for the permission
/// and opens System Settings; macOS applies a new grant only after Kadro restarts.
pub fn ensure_screen_capture(app: &AppHandle) -> bool {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return true;
    }
    // Shows the system prompt the first time and adds Kadro to the settings list.
    unsafe { CGRequestScreenCaptureAccess() };
    if let Err(err) = app.opener().open_url(SETTINGS_URL, None::<&str>) {
        log::error!("could not open System Settings: {err}");
    }
    // macOS only applies the grant after a restart, so offer it right here.
    let handle = app.clone();
    app.dialog()
        .message(
            "Ative o Kadro em Ajustes do Sistema → Privacidade e Segurança → Gravação de Tela. \
             Depois clique em Reiniciar: o macOS só aplica a permissão quando o Kadro abre de novo.\n\n\
             Se o Kadro já aparecer ativado e mesmo assim não funcionar, remova-o da lista com o \
             botão − e tente de novo.",
        )
        .title("Permissão de Gravação de Tela")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Reiniciar o Kadro".into(),
            "Depois".into(),
        ))
        .show(move |restart| {
            if restart {
                handle.restart();
            }
        });
    false
}

/// On the first launch of a new version, clears a Screen Recording grant left by the
/// previous one.
///
/// Kadro has no Apple signature, so macOS ties the grant to the exact build. After an
/// update the old grant still shows as enabled in System Settings, but it no longer
/// matches, and switching it off and on does not help: only a reset does. Running the
/// reset from the new version itself (not from the old one before it quits) makes sure
/// it happens. A grant that still works is left alone.
pub fn reset_after_update(app: &AppHandle) {
    let current = app.package_info().version.to_string();
    let state = app.state::<config::ConfigState>();
    let mut cfg = state.0.lock().unwrap();
    if cfg.last_version.as_deref() == Some(current.as_str()) {
        return;
    }
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        match std::process::Command::new("/usr/bin/tccutil")
            .args(["reset", "ScreenCapture", &app.config().identifier])
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => log::warn!("tccutil reset exited with {status}"),
            Err(err) => log::warn!("tccutil reset failed: {err}"),
        }
    }
    cfg.last_version = Some(current);
    if let Err(err) = config::store(app, &cfg) {
        log::warn!("could not save config: {err}");
    }
}
