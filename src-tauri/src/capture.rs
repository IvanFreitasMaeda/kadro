//! Screen capture flow: grab every monitor, open one overlay window per monitor
//! showing the frozen screenshot, then crop the user's selection and copy or save it.

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    imageops, ExtendedColorType, ImageEncoder, ImageFormat, RgbaImage,
};
use serde::Deserialize;
use tauri::{
    http,
    ipc::{InvokeBody, Request},
    AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, UriSchemeContext,
    UriSchemeResponder, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
use tauri_plugin_dialog::DialogExt;

use crate::config;

const OVERLAY_PREFIX: &str = "overlay-";

#[derive(Default)]
pub struct CaptureState {
    /// One screenshot per monitor, indexed like the overlay window labels.
    shots: Mutex<Vec<RgbaImage>>,
    /// True while overlays are open, so a second hotkey press is ignored.
    active: AtomicBool,
    /// Kept alive for the app's lifetime: on Linux the clipboard content is owned
    /// by the process and disappears when the `Clipboard` is dropped.
    clipboard: Mutex<Option<arboard::Clipboard>>,
}

/// Selection in screenshot pixels, sent by the overlay in the `x-kadro-rect` header.
#[derive(Debug, Deserialize)]
pub struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// Starts a capture. `full` preselects the whole monitor under the cursor.
pub fn start(app: &AppHandle, full: bool) {
    let state = app.state::<CaptureState>();
    if state.active.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    // Capturing and building windows off the main thread keeps the event loop
    // responsive and avoids window-creation deadlocks on Windows.
    std::thread::spawn(move || {
        if let Err(err) = open_overlays(&app, full) {
            log::error!("capture failed: {err}");
            close_all(&app);
        }
    });
}

fn open_overlays(app: &AppHandle, full: bool) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if !crate::permissions::ensure_screen_capture(app) {
        app.state::<CaptureState>()
            .active
            .store(false, Ordering::SeqCst);
        return Ok(());
    }

    let cursor = app.cursor_position().ok();
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let screens = app.available_monitors().map_err(|e| e.to_string())?;

    let mut shots = Vec::with_capacity(monitors.len());
    let mut placements = Vec::with_capacity(monitors.len());
    for (i, monitor) in monitors.iter().enumerate() {
        let image = monitor.capture_image().map_err(|e| e.to_string())?;
        placements.push(placement(monitor, &screens, i, &image));
        shots.push(image);
    }
    *app.state::<CaptureState>().shots.lock().unwrap() = shots;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();

    for (i, (pos, size)) in placements.into_iter().enumerate() {
        let under_cursor = cursor.is_some_and(|c| {
            c.x >= pos.x as f64
                && c.y >= pos.y as f64
                && c.x < (pos.x + size.width as i32) as f64
                && c.y < (pos.y + size.height as i32) as f64
        });
        let init = format!(
            "window.__KADRO__ = {{ monitor: {i}, nonce: {nonce}, full: {} }};",
            full && under_cursor
        );
        let builder = WebviewWindowBuilder::new(
            app,
            format!("{OVERLAY_PREFIX}{i}"),
            WebviewUrl::App("overlay.html".into()),
        )
        .title("Kadro")
        .initialization_script(&init)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .shadow(false)
        .visible(false)
        .focused(true);

        // macOS: xcap reports the display frame in points with a top-left origin, which
        // is exactly what Tauri's logical coordinates are. Creating the window with its
        // final frame avoids resizing it afterwards: Cocoa keeps the bottom-left corner
        // fixed on resize, which pushed the overlay off the top of the screen.
        #[cfg(target_os = "macos")]
        let window = {
            let m = &monitors[i];
            builder
                .position(m.x().unwrap_or(0) as f64, m.y().unwrap_or(0) as f64)
                .inner_size(
                    m.width().unwrap_or(size.width) as f64,
                    m.height().unwrap_or(size.height) as f64,
                )
                .build()
                .map_err(|e| e.to_string())?
        };

        // Windows/Linux: place in physical pixels, which stays exact across monitors
        // with different scaling. Size again after moving, because moving onto a
        // monitor with another scale makes Windows rescale the window.
        #[cfg(not(target_os = "macos"))]
        {
            let window = builder.build().map_err(|e| e.to_string())?;
            window.set_size(size).map_err(|e| e.to_string())?;
            window.set_position(pos).map_err(|e| e.to_string())?;
            window.set_size(size).map_err(|e| e.to_string())?;
        }

        #[cfg(target_os = "macos")]
        raise_above_menu_bar(&window);
    }
    Ok(())
}

/// Finds where the overlay for an xcap monitor goes, in physical pixels.
/// xcap reports positions in points on macOS and in pixels on Windows/Linux, so we
/// match against Tauri's monitor list both ways and fall back to the list order.
fn placement(
    monitor: &xcap::Monitor,
    screens: &[tauri::Monitor],
    index: usize,
    image: &RgbaImage,
) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    let (x, y) = (monitor.x().unwrap_or(0), monitor.y().unwrap_or(0));
    let matches = |s: &&tauri::Monitor| {
        let p = s.position();
        let scale = s.scale_factor();
        (p.x == x && p.y == y)
            || ((p.x as f64 / scale).round() as i32 == x
                && (p.y as f64 / scale).round() as i32 == y)
    };
    match screens.iter().find(matches).or_else(|| screens.get(index)) {
        Some(screen) => (*screen.position(), *screen.size()),
        None => (
            PhysicalPosition::new(x, y),
            PhysicalSize::new(image.width(), image.height()),
        ),
    }
}

/// Puts the overlay above the macOS menu bar and Dock (Tauri's always-on-top level is below them).
#[cfg(target_os = "macos")]
fn raise_above_menu_bar(window: &WebviewWindow) {
    let window = window.clone();
    let _ = window.clone().run_on_main_thread(move || {
        if let Ok(ns_window) = window.ns_window() {
            // NSScreenSaverWindowLevel
            const LEVEL: isize = 1000;
            unsafe {
                let ns_window = &*(ns_window as *mut objc2::runtime::AnyObject);
                let _: () = objc2::msg_send![ns_window, setLevel: LEVEL];
            }
        }
    });
}

/// Closes every overlay and frees the screenshots.
pub fn close_all(app: &AppHandle) {
    for (label, window) in app.webview_windows() {
        if label.starts_with(OVERLAY_PREFIX) {
            let _ = window.destroy();
        }
    }
    let state = app.state::<CaptureState>();
    state.shots.lock().unwrap().clear();
    state.active.store(false, Ordering::SeqCst);
}

/// Serves `kadro://localhost/<monitor>/<nonce>` as a PNG of that monitor's screenshot.
pub fn serve_shot<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let path = percent_encoding::percent_decode_str(request.uri().path())
        .decode_utf8_lossy()
        .into_owned();
    std::thread::spawn(move || {
        let index = path
            .trim_start_matches('/')
            .split('/')
            .next()
            .and_then(|s| s.parse::<usize>().ok());
        let png = index.and_then(|i| {
            let state = app.state::<CaptureState>();
            let shots = state.shots.lock().unwrap();
            shots
                .get(i)
                .and_then(|img| encode_png(img, CompressionType::Fast).ok())
        });
        let response = match png {
            Some(bytes) => http::Response::builder()
                .header(http::header::CONTENT_TYPE, "image/png")
                .header(http::header::CACHE_CONTROL, "no-store")
                .body(bytes),
            None => http::Response::builder()
                .status(http::StatusCode::NOT_FOUND)
                .body(Vec::new()),
        };
        responder.respond(response.unwrap());
    });
}

fn encode_png(img: &RgbaImage, compression: CompressionType) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    PngEncoder::new_with_quality(&mut buf, compression, FilterType::Adaptive)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(buf)
}

fn crop(app: &AppHandle, window: &WebviewWindow, rect: &Rect) -> Result<RgbaImage, String> {
    let index: usize = window
        .label()
        .strip_prefix(OVERLAY_PREFIX)
        .and_then(|s| s.parse().ok())
        .ok_or("janela inválida")?;
    let state = app.state::<CaptureState>();
    let shots = state.shots.lock().unwrap();
    let img = shots.get(index).ok_or("captura não encontrada")?;
    let x = rect.x.min(img.width().saturating_sub(1));
    let y = rect.y.min(img.height().saturating_sub(1));
    let w = rect.width.clamp(1, img.width() - x);
    let h = rect.height.clamp(1, img.height() - y);
    Ok(imageops::crop_imm(img, x, y, w, h).to_image())
}

fn write_png(img: &RgbaImage, path: &Path) -> Result<(), String> {
    let bytes = encode_png(img, CompressionType::Default)?;
    std::fs::write(path, bytes).map_err(|e| format!("não foi possível salvar: {e}"))
}

fn file_name() -> String {
    format!(
        "Kadro {}.png",
        chrono::Local::now().format("%Y-%m-%d às %H.%M.%S")
    )
}

/// Picks a file name in `dir` that doesn't exist yet.
fn unique_path(dir: &Path) -> PathBuf {
    let name = file_name();
    let mut path = dir.join(&name);
    let stem = name.trim_end_matches(".png").to_string();
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).png"));
        n += 1;
    }
    path
}

fn copy_to_clipboard(app: &AppHandle, img: RgbaImage) -> Result<(), String> {
    let state = app.state::<CaptureState>();
    let mut guard = state.clipboard.lock().unwrap();
    if guard.is_none() {
        *guard = Some(arboard::Clipboard::new().map_err(|e| e.to_string())?);
    }
    let (width, height) = (img.width() as usize, img.height() as usize);
    guard
        .as_mut()
        .unwrap()
        .set_image(arboard::ImageData {
            width,
            height,
            bytes: Cow::Owned(img.into_raw()),
        })
        .map_err(|e| format!("não foi possível copiar: {e}"))
}

/// Builds the final image from an overlay request: the selection comes in a header
/// and the annotations, if any, as a transparent PNG of the same size in the body.
fn compose(
    app: &AppHandle,
    window: &WebviewWindow,
    request: &Request<'_>,
) -> Result<RgbaImage, String> {
    let rect: Rect = request
        .headers()
        .get("x-kadro-rect")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| serde_json::from_str(v).ok())
        .ok_or("seleção inválida")?;
    let mut img = crop(app, window, &rect)?;
    if let InvokeBody::Raw(bytes) = request.body() {
        if !bytes.is_empty() {
            let ink = image::load_from_memory_with_format(bytes, ImageFormat::Png)
                .map_err(|e| format!("anotações inválidas: {e}"))?
                .to_rgba8();
            imageops::overlay(&mut img, &ink, 0, 0);
        }
    }
    Ok(img)
}

#[tauri::command]
pub async fn copy_selection(
    app: AppHandle,
    window: WebviewWindow,
    request: Request<'_>,
) -> Result<(), String> {
    let img = compose(&app, &window, &request)?;
    close_all(&app);
    copy_to_clipboard(&app, img)
}

/// Saves to the configured folder and returns the file path. The overlay closes itself
/// after showing a confirmation.
#[tauri::command]
pub async fn save_selection(
    app: AppHandle,
    window: WebviewWindow,
    request: Request<'_>,
) -> Result<String, String> {
    let img = compose(&app, &window, &request)?;
    let path = unique_path(&config::save_dir(&app)?);
    write_png(&img, &path)?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub async fn save_selection_as(
    app: AppHandle,
    window: WebviewWindow,
    request: Request<'_>,
) -> Result<(), String> {
    let img = compose(&app, &window, &request)?;
    // Close first: the overlay sits above everything, including the save dialog.
    close_all(&app);
    let dir = config::save_dir(&app).unwrap_or_else(|_| config::default_save_dir());
    let chosen = app
        .dialog()
        .file()
        .set_title("Salvar captura")
        .set_directory(dir)
        .set_file_name(file_name())
        .add_filter("Imagem PNG", &["png"])
        .blocking_save_file();
    let Some(path) = chosen.and_then(|p| p.as_path().map(Path::to_path_buf)) else {
        return Ok(());
    };
    let path = if path.extension().is_none() {
        path.with_extension("png")
    } else {
        path
    };
    write_png(&img, &path)
}

#[tauri::command]
pub async fn close_capture(app: AppHandle) {
    close_all(&app);
}
