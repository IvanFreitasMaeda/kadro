mod autostart;
mod capture;
mod clipboard;
mod config;
mod fileclip;
mod gallery;
mod history;
#[cfg(target_os = "linux")]
mod linux;
mod ocr;
#[cfg(target_os = "macos")]
mod permissions;
mod record;
mod settings;
mod shortcuts;
mod tray;
mod updater;

use std::sync::Mutex;

use tauri::{Manager, RunEvent};

pub fn run() {
    #[cfg(target_os = "linux")]
    linux::prepare_env();

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Launching Kadro again while it runs starts a capture instead of a second
            // copy, or the action named on the command line (`kadro --record`…).
            let kind = shortcuts::kind_from_args(&args).unwrap_or(shortcuts::Kind::Area);
            shortcuts::run(app, kind);
        }))
        .plugin(shortcuts::plugin())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(autostart::plugin())
        .manage(capture::CaptureState::default())
        .manage(record::RecordState::default())
        .register_asynchronous_uri_scheme_protocol("kadro", capture::serve_shot)
        .invoke_handler(tauri::generate_handler![
            capture::copy_selection,
            capture::save_selection,
            capture::save_selection_as,
            capture::close_capture,
            capture::recognize_text,
            capture::copy_text,
            gallery::close_gallery,
            clipboard::popup::close_clipboard,
            clipboard::clip_list,
            clipboard::clip_use,
            clipboard::clip_pin,
            clipboard::clip_delete,
            clipboard::clip_clear,
            clipboard::set_clip_retention,
            history::history_list,
            history::history_copy,
            history::history_save,
            history::history_save_as,
            history::history_open,
            history::history_reveal,
            history::history_delete,
            history::history_clear,
            history::set_history_limit,
            record::start_recording,
            record::stop_recording,
            record::recording_elapsed,
            record::draw::set_drawing,
            record::draw::drawing,
            record::draw::fit_bar,
            settings::get_settings,
            settings::set_shortcut,
            settings::pause_shortcuts,
            settings::choose_save_dir,
            settings::open_support_page,
            autostart::set_autostart,
        ])
        .setup(|app| {
            // Menu bar app: no Dock icon on macOS.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle();
            app.manage(config::ConfigState(Mutex::new(config::load(handle))));
            #[cfg(target_os = "macos")]
            permissions::reset_after_update(handle);
            autostart::init(handle);
            #[cfg(target_os = "linux")]
            if linux::is_wayland() {
                linux::shortcuts::register_app(&app.config().identifier);
            }
            tray::create(handle)?;
            shortcuts::register_all(handle);
            clipboard::start(handle);
            updater::start(handle);
            // Started with an action (`kadro --area` from a system shortcut while
            // Kadro wasn't running): do it once the app is up.
            let args: Vec<String> = std::env::args().collect();
            if let Some(kind) = shortcuts::kind_from_args(&args) {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    let app = handle.clone();
                    let _ = handle.run_on_main_thread(move || shortcuts::run(&app, kind));
                });
            }
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
