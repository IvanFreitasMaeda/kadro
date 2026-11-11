mod capture;
mod config;
mod gallery;
mod history;
#[cfg(target_os = "macos")]
mod permissions;
mod settings;
mod shortcuts;
mod tray;

use std::sync::Mutex;

use tauri::{Manager, RunEvent};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Launching Kadro again while it runs starts a capture instead of a second copy.
            capture::start(app, false);
        }))
        .plugin(shortcuts::plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(capture::CaptureState::default())
        .register_asynchronous_uri_scheme_protocol("kadro", capture::serve_shot)
        .invoke_handler(tauri::generate_handler![
            capture::copy_selection,
            capture::save_selection,
            capture::save_selection_as,
            capture::close_capture,
            gallery::close_gallery,
            history::history_list,
            history::history_copy,
            history::history_save,
            history::history_save_as,
            history::history_open,
            history::history_delete,
            history::history_clear,
            history::set_history_limit,
            settings::get_settings,
            settings::set_shortcut,
            settings::pause_shortcuts,
            settings::choose_save_dir,
        ])
        .setup(|app| {
            // Menu bar app: no Dock icon on macOS.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            app.manage(config::ConfigState(Mutex::new(config::load(handle))));
            tray::create(handle)?;
            shortcuts::register_all(handle);
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Kadro")
        .run(|_app, event| {
            // Kadro lives in the tray: closing the last overlay must not quit it.
            if let RunEvent::ExitRequested {
                api, code: None, ..
            } = event
            {
                api.prevent_exit();
            }
        });
}
