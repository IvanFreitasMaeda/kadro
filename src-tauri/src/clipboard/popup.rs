//! The clipboard history popup: a small window next to the cursor.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use super::platform;

pub const LABEL: &str = "clipboard";
/// Window size in logical pixels.
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 480.0;
/// Gap between the cursor and the window.
const OFFSET: f64 = 8.0;

/// Opens the popup, or closes it if it is already open.
pub fn toggle(app: &AppHandle) {
    if close(app) {
        platform::activate_previous();
        return;
    }
    // Remember where the user was, to go back there after picking an item.
    platform::remember_foreground();
    let app = app.clone();
    // Build windows off the main thread, like the gallery and the overlays.
    std::thread::spawn(move || {
        if let Err(err) = open(&app) {
            log::error!("could not open the clipboard history: {err}");
        }
    });
}

/// Closes the popup. Returns whether it was open.
pub fn close(app: &AppHandle) -> bool {
    match app.get_webview_window(LABEL) {
        Some(window) => {
            let _ = window.destroy();
            true
        }
        None => false,
    }
}

fn open(app: &AppHandle) -> Result<(), String> {
    let cursor = app.cursor_position().map_err(|e| e.to_string())?;
    let monitor = app
        .monitor_from_point(cursor.x, cursor.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or("nenhum monitor encontrado")?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();

    // Below and to the right of the cursor, kept inside the screen's usable area.
    #[cfg(target_os = "macos")]
    let place = |cursor: f64, start: f64, length: f64, size: f64| {
        let end = start + length;
        let mut p = cursor + OFFSET;
        if p + size > end {
            p = cursor - OFFSET - size;
        }
        p.clamp(start, (end - size).max(start))
    };

    // The window stays hidden until the page has drawn the list (clipboard.ts shows it).
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("clipboard.html".into()))
        .title("Área de transferência")
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(true);

    // macOS: logical coordinates are the screen's points (see gallery.rs).
    #[cfg(target_os = "macos")]
    {
        let x = place(
            cursor.x / scale,
            area.position.x as f64 / scale,
            area.size.width as f64 / scale,
            WIDTH,
        );
        let y = place(
            cursor.y / scale,
            area.position.y as f64 / scale,
            area.size.height as f64 / scale,
            HEIGHT,
        );
        builder
            .position(x, y)
            .inner_size(WIDTH, HEIGHT)
            .build()
            .map_err(|e| e.to_string())?;
    }

    // Windows: physical pixels stay exact with mixed scaling.
    #[cfg(not(target_os = "macos"))]
    {
        use tauri::{PhysicalPosition, PhysicalSize};
        let width = (WIDTH * scale).round();
        let height = (HEIGHT * scale).round();
        let offset = OFFSET * scale;
        let place = |cursor: f64, start: f64, length: f64, size: f64| {
            let end = start + length;
            let mut p = cursor + offset;
            if p + size > end {
                p = cursor - offset - size;
            }
            p.clamp(start, (end - size).max(start))
        };
        let x = place(
            cursor.x,
            area.position.x as f64,
            area.size.width as f64,
            width,
        );
        let y = place(
            cursor.y,
            area.position.y as f64,
            area.size.height as f64,
            height,
        );
        let size = PhysicalSize::new(width as u32, height as u32);
        let window = builder.build().map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
        window
            .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
            .map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Closes the popup. `restore` gives the focus back to the app that was in front
/// (Esc); a click elsewhere already moved it.
#[tauri::command]
pub async fn close_clipboard(app: AppHandle, restore: bool) {
    if close(&app) && restore {
        let _ = app.run_on_main_thread(platform::activate_previous);
    }
}
