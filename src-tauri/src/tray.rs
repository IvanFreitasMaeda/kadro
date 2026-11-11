//! Menu bar / system tray icon. Left click starts a capture, right click opens the menu.

use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::{capture, config, shortcuts};

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "area", "Capturar área", true, Some(shortcuts::AREA))?,
            &MenuItem::with_id(
                app,
                "full",
                "Capturar tela inteira",
                true,
                Some(shortcuts::FULL),
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(
                app,
                "open_dir",
                "Abrir pasta de capturas",
                true,
                None::<&str>,
            )?,
            &MenuItem::with_id(
                app,
                "choose_dir",
                "Escolher pasta de capturas…",
                true,
                None::<&str>,
            )?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Sair do Kadro", true, None::<&str>)?,
        ],
    )?;

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
        .tooltip("Kadro: clique para capturar")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "area" => capture::start(app, false),
            "full" => capture::start(app, true),
            "open_dir" => open_save_dir(app),
            "choose_dir" => choose_save_dir(app),
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
                capture::start(tray.app_handle(), false);
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

fn choose_save_dir(app: &AppHandle) {
    let current = config::save_dir(app).unwrap_or_else(|_| config::default_save_dir());
    let handle = app.clone();
    app.dialog()
        .file()
        .set_title("Escolha a pasta onde as capturas serão salvas")
        .set_directory(current)
        .pick_folder(move |folder| {
            if let Some(path) = folder.and_then(|f| f.as_path().map(|p| p.to_path_buf())) {
                if let Err(err) = config::set_save_dir(&handle, path) {
                    log::error!("could not save settings: {err}");
                }
            }
        });
}
