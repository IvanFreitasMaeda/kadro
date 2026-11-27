//! In-app updates from GitHub Releases (`latest.json`, signed with the updater key).
//!
//! Kadro checks quietly a little after it starts and then once a day, and the tray menu
//! has "Verificar atualizações…" for a manual check. Nothing is downloaded without
//! asking: a dialog offers to update and restart.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{record, tray};

const FIRST_CHECK: Duration = Duration::from_secs(15);
const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Set while a check or a download runs, so two never overlap.
static BUSY: AtomicBool = AtomicBool::new(false);

/// Linux: whether this copy can update itself. An AppImage replaces its own file;
/// the .deb and .rpm reinstall with `pkexec`. A package built by the distribution or
/// the AUR (no dpkg/rpm, or not from our bundle) is updated by the package manager.
#[cfg(target_os = "linux")]
fn can_update() -> bool {
    use tauri::utils::{config::BundleType, platform::bundle_type};
    let has = |tool: &str| {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(tool).is_file())
        })
    };
    let owns = |cmd: &[&str]| {
        std::process::Command::new(cmd[0])
            .args(&cmd[1..])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    match bundle_type() {
        Some(BundleType::AppImage) => std::env::var_os("APPIMAGE").is_some(),
        // The package database must own Kadro: the AUR package repackages the .deb,
        // and an Arch machine may still have dpkg around.
        Some(BundleType::Deb) => has("pkexec") && owns(&["dpkg-query", "-W", "kadro"]),
        Some(BundleType::Rpm) => has("pkexec") && owns(&["rpm", "-q", "kadro"]),
        _ => false,
    }
}

#[cfg(not(target_os = "linux"))]
fn can_update() -> bool {
    true
}

/// Starts the background checks.
pub fn start(app: &AppHandle) {
    if !can_update() {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        thread::sleep(FIRST_CHECK);
        loop {
            run(&app, false);
            thread::sleep(CHECK_EVERY);
        }
    });
}

/// Manual check from the tray menu: also says when there is nothing new or it failed.
pub fn check_now(app: &AppHandle) {
    if !can_update() {
        let app = app.clone();
        thread::spawn(move || {
            info(
                &app,
                "Este Kadro foi instalado pelo gerenciador de pacotes do sistema (ou do AUR). Atualize por ele, ou baixe a versão nova em github.com/IvanFreitasMaeda/kadro/releases.",
            )
        });
        return;
    }
    let app = app.clone();
    thread::spawn(move || run(&app, true));
}

fn run(app: &AppHandle, manual: bool) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    tray::set_updating(app, Some("Procurando atualizações…"));
    let found = tauri::async_runtime::block_on(async {
        app.updater()
            .map_err(|e| e.to_string())?
            .check()
            .await
            .map_err(|e| e.to_string())
    });
    tray::set_updating(app, None);
    match found {
        Ok(Some(update)) => offer(app, update),
        Ok(None) if manual => info(
            app,
            &format!(
                "Você já está na versão mais recente do Kadro ({}).",
                app.package_info().version
            ),
        ),
        Ok(None) => {}
        Err(err) => {
            log::warn!("update check failed: {err}");
            if manual {
                error(
                    app,
                    "Não deu para verificar atualizações agora. Confira a internet e tente de novo.",
                );
            }
        }
    }
    BUSY.store(false, Ordering::SeqCst);
}

fn offer(app: &AppHandle, update: Update) {
    let mut text = format!(
        "O Kadro {} está disponível (você tem a {}).",
        update.version, update.current_version
    );
    if let Some(notes) = update
        .body
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        text.push_str("\n\n");
        text.push_str(&shorten(notes, 600));
    }
    if cfg!(target_os = "macos") {
        text.push_str(
            "\n\nDepois de atualizar, o macOS pede de novo a permissão de Gravação de Tela.",
        );
    }
    let yes = app
        .dialog()
        .message(text)
        .title("Atualização do Kadro")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Atualizar e reiniciar".into(),
            "Agora não".into(),
        ))
        .blocking_show();
    if !yes {
        return;
    }
    if record::is_busy(app) {
        info(
            app,
            "Termine a gravação antes de atualizar. Depois use \"Verificar atualizações…\" no menu do Kadro.",
        );
        return;
    }

    tray::set_updating(app, Some("Baixando atualização…"));
    let installed = tauri::async_runtime::block_on(update.download_and_install(|_, _| {}, || {}));
    tray::set_updating(app, None);
    match installed {
        Ok(()) => {
            // On Windows the installer has already closed Kadro and reopens it itself.
            app.restart();
        }
        Err(err) => {
            log::error!("update install failed: {err}");
            error(
                app,
                "Não deu para instalar a atualização. Tente de novo mais tarde ou baixe a versão nova em github.com/IvanFreitasMaeda/kadro/releases.",
            );
        }
    }
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

fn info(app: &AppHandle, text: &str) {
    app.dialog()
        .message(text)
        .title("Kadro")
        .kind(MessageDialogKind::Info)
        .blocking_show();
}

fn error(app: &AppHandle, text: &str) {
    app.dialog()
        .message(text)
        .title("Kadro")
        .kind(MessageDialogKind::Error)
        .blocking_show();
}
