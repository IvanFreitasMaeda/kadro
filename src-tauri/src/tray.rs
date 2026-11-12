//! Menu bar / system tray icon. Left click starts a capture, right click opens the menu.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};
use tauri_plugin_opener::OpenerExt;

use crate::{
    capture, config, gallery, record, settings,
    shortcuts::{self, Kind},
    updater,
};

/// Tray menu items whose accelerator label follows the configured shortcuts.
pub struct TrayItems {
    area: MenuItem<Wry>,
    full: MenuItem<Wry>,
    gallery: MenuItem<Wry>,
    record: MenuItem<Wry>,
    update: MenuItem<Wry>,
}

const UPDATE_LABEL: &str = "Verificar atualizações…";

const TOOLTIP: &str = "Kadro: clique para capturar";
const TOOLTIP_RECORDING: &str = "Kadro: gravando. Clique para parar";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let area = MenuItem::with_id(
        app,
        "area",
        "Capturar área",
        true,
        Some(shortcuts::current(app, Kind::Area)),
    )?;
    let full = MenuItem::with_id(
        app,
        "full",
        "Capturar tela inteira",
        true,
        Some(shortcuts::current(app, Kind::Full)),
    )?;
    let record = MenuItem::with_id(
        app,
        "record",
        "Gravar tela",
        true,
        Some(shortcuts::current(app, Kind::Record)),
    )?;
    let gallery = MenuItem::with_id(
        app,
        "gallery",
        "Histórico de capturas",
        true,
        Some(shortcuts::current(app, Kind::Gallery)),
    )?;
    let update = MenuItem::with_id(app, "update", UPDATE_LABEL, true, None::<&str>)?;
    let version = MenuItem::with_id(
        app,
        "version",
        format!("Kadro {}", app.package_info().version),
        false,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &area,
            &full,
            &record,
            &PredefinedMenuItem::separator(app)?,
            &gallery,
            &MenuItem::with_id(
                app,
                "open_dir",
                "Abrir pasta de capturas",
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(app, "settings", "Preferências…", true, Some("CmdOrCtrl+,"))?,
            &PredefinedMenuItem::separator(app)?,
            &version,
            &update,
            &MenuItem::with_id(app, "quit", "Sair do Kadro", true, None::<&str>)?,
        ],
    )?;
    app.manage(TrayItems {
        area,
        full,
        gallery,
        record,
        update,
    });

    // macOS menu bar icons are monochrome "template" images; elsewhere use the app icon.
    #[cfg(target_os = "macos")]
    let icon = Image::from_bytes(include_bytes!("../icons/tray-template.png"))?;
    #[cfg(not(target_os = "macos"))]
    let icon = app
        .default_window_icon()
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| Image::from_bytes(include_bytes!("../icons/32x32.png")))?;

    TrayIconBuilder::with_id("kadro")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip(TOOLTIP)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "area" => capture::start(app, false),
            "full" => capture::start(app, true),
            "record" => record::toggle(app),
            "gallery" => gallery::toggle(app),
            "open_dir" => open_save_dir(app),
            "settings" => settings::open(app),
            "update" => updater::check_now(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                // While recording, the icon is the quickest way to stop.
                let app = tray.app_handle();
                if record::is_busy(app) {
                    record::stop(app, false);
                } else {
                    capture::start(app, false);
                }
            }
        })
        .build(app)?;
    Ok(())
}

fn open_save_dir(app: &AppHandle) {
    match config::save_dir(app) {
        Ok(dir) => {
            if let Err(err) = app.opener().open_path(dir.to_string_lossy(), None::<&str>) {
                log::error!("could not open {}: {err}", dir.display());
            }
        }
        Err(err) => log::error!("{err}"),
    }
}

/// Updates the shortcut labels shown in the tray menu.
pub fn refresh_accelerators(app: &AppHandle) {
    let items = app.state::<TrayItems>();
    let _ = items
        .area
        .set_accelerator(Some(shortcuts::current(app, Kind::Area)));
    let _ = items
        .full
        .set_accelerator(Some(shortcuts::current(app, Kind::Full)));
    let _ = items
        .gallery
        .set_accelerator(Some(shortcuts::current(app, Kind::Gallery)));
    let _ = items
        .record
        .set_accelerator(Some(shortcuts::current(app, Kind::Record)));
}

/// Switches the menu and tooltip between "record" and "stop".
pub fn set_recording(app: &AppHandle, recording: bool) {
    let items = app.state::<TrayItems>();
    let _ = items.record.set_text(if recording {
        "Parar gravação"
    } else {
        "Gravar tela"
    });
    if let Some(tray) = app.tray_by_id("kadro") {
        let _ = tray.set_tooltip(Some(if recording {
            TOOLTIP_RECORDING
        } else {
            TOOLTIP
        }));
    }
}

/// Shows what the updater is doing in place of "Verificar atualizações…" (`None` restores it).
pub fn set_updating(app: &AppHandle, status: Option<&str>) {
    let item = &app.state::<TrayItems>().update;
    let _ = item.set_text(status.unwrap_or(UPDATE_LABEL));
    let _ = item.set_enabled(status.is_none());
}
