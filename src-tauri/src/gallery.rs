//! Side gallery: a panel along the right edge of the screen with the recent captures.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const LABEL: &str = "gallery";
/// Panel width in logical pixels.
const WIDTH: f64 = 300.0;
/// Gap between the panel and the screen edges, in logical pixels.
const MARGIN: f64 = 8.0;

/// Opens the gallery, or closes it if it is already open.
pub fn toggle(app: &AppHandle) {
    if close(app) {
        return;
    }
    let app = app.clone();
    // Same as the capture overlays: build windows off the main thread.
    std::thread::spawn(move || {
        if let Err(err) = open(&app) {
            log::error!("could not open the gallery: {err}");
        }
    });
}

/// Closes the gallery. Returns whether it was open.
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
    // Open on the screen under the cursor, against its right edge, leaving the
    // menu bar, Dock and taskbar clear.
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|c| app.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or("nenhum monitor encontrado")?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();

    // The window stays hidden until the page has drawn the list (gallery.ts shows it).
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("gallery.html".into()))
        .title("Histórico do Kadro")
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(true);

    // macOS: Tauri's logical coordinates are the screen's points, so build the
    // window with its final frame (resizing later moves it, see capture.rs).
    #[cfg(target_os = "macos")]
    {
        let x = (area.position.x as f64 + area.size.width as f64) / scale - WIDTH - MARGIN;
        let y = area.position.y as f64 / scale + MARGIN;
        let height = area.size.height as f64 / scale - 2.0 * MARGIN;
        builder
            .position(x, y)
            .inner_size(WIDTH, height)
            .build()
            .map_err(|e| e.to_string())?;
    }

    // Windows: place in physical pixels, which stays exact with mixed scaling,
    // and size again after moving onto a monitor with another scale.
    #[cfg(not(target_os = "macos"))]
    {
        use tauri::{PhysicalPosition, PhysicalSize};
        let width = (WIDTH * scale).round() as u32;
        let margin = (MARGIN * scale).round() as i32;
        let size = PhysicalSize::new(width, area.size.height.saturating_sub(2 * margin as u32));
        let pos = PhysicalPosition::new(
            area.position.x + area.size.width as i32 - width as i32 - margin,
            area.position.y + margin,
        );
        let window = builder.build().map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
        window.set_position(pos).map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn close_gallery(app: AppHandle) {
    close(&app);
}
