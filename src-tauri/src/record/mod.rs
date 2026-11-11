//! Screen recording to MP4 or GIF.
//!
//! The user picks the area in the same overlay as a screenshot. Then a small control
//! bar shows the time and the stop button while the platform backend records:
//! ScreenCaptureKit + AVAssetWriter on macOS, Windows Graphics Capture + Media
//! Foundation on Windows. Both encode H.264 with the OS's own (hardware) encoder, so
//! nothing heavy ships with the app. GIFs are encoded by `gif.rs` from the frames.

// Linux builds (development only) have no backend, so the frame plumbing is unused.
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]

mod gif;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use std::{
    path::PathBuf,
    sync::{mpsc::SyncSender, Arc, Mutex},
    time::{Duration, Instant},
};

use image::RgbaImage;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

use crate::{capture, config, gallery, history, tray};

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// The control bar shown while recording.
pub const BAR_LABEL: &str = "recorder";
const BAR_WIDTH: f64 = 232.0;
const BAR_HEIGHT: f64 = 44.0;
/// Gap between the bar and the bottom of the screen, in logical pixels.
const BAR_MARGIN: f64 = 72.0;

const MP4_FPS: u32 = 30;
const GIF_FPS: u32 = 15;
/// Hardware H.264 encoders top out around 4K.
const MP4_MAX: (u32, u32) = (3840, 2160);
/// GIFs bigger than this get heavy fast; they are scaled down to fit.
const GIF_MAX: u32 = 1280;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    #[default]
    Mp4,
    Gif,
}

impl Format {
    pub fn ext(self) -> &'static str {
        match self {
            Format::Mp4 => "mp4",
            Format::Gif => "gif",
        }
    }
}

/// The monitor being recorded, as xcap describes it.
pub struct Monitor {
    /// CGDirectDisplayID on macOS, HMONITOR on Windows.
    #[allow(dead_code)]
    pub id: u32,
    /// Position and size in xcap units: points on macOS, pixels on Windows.
    #[allow(dead_code)]
    pub x: i32,
    #[allow(dead_code)]
    pub y: i32,
    #[allow(dead_code)]
    pub width: u32,
    #[allow(dead_code)]
    pub height: u32,
    /// Screenshot pixels per xcap unit (2 on a Retina Mac, 1 on Windows).
    #[allow(dead_code)]
    pub px_per_unit: f64,
}

/// Everything a backend needs to start recording.
pub struct Target {
    pub monitor: Monitor,
    /// The area to record, in screenshot pixels.
    #[allow(dead_code)]
    pub rect: capture::Rect,
    /// Size of the output frames.
    pub size: (u32, u32),
    pub fps: u32,
    pub format: Format,
    pub path: PathBuf,
}

/// A running platform recording.
pub trait Backend: Send {
    /// Stops capturing and, for MP4, finishes the file.
    fn stop(self: Box<Self>) -> Result<(), String>;
}

/// Where backends send pixels: the first frame becomes the thumbnail, and every frame
/// goes to the GIF encoder when recording a GIF.
pub struct Feed {
    gif: Option<SyncSender<gif::Msg>>,
    thumb: Mutex<Option<RgbaImage>>,
    /// Time of the last frame (from the first one) and when it arrived.
    last: Mutex<Option<(Duration, Instant)>>,
}

impl Feed {
    /// Whether the backend should copy the pixels of the next frame out.
    pub fn wants_pixels(&self) -> bool {
        self.gif.is_some() || self.thumb.lock().unwrap().is_none()
    }

    /// Takes a BGRA frame whose rows are `stride` bytes apart, captured `time` after
    /// the first frame.
    pub fn push_bgra(&self, width: u32, height: u32, stride: usize, data: &[u8], time: Duration) {
        *self.last.lock().unwrap() = Some((time, Instant::now()));
        if !self.wants_pixels() || width == 0 || height == 0 {
            return;
        }
        let Some(img) = bgra_to_rgba(width, height, stride, data) else {
            return;
        };
        {
            let mut thumb = self.thumb.lock().unwrap();
            if thumb.is_none() {
                *thumb = Some(img.clone());
            }
        }
        if let Some(tx) = &self.gif {
            // Drop the frame if the encoder is behind; the next one carries on.
            let _ = tx.try_send(gif::Msg::Frame(img, time));
        }
    }

    /// Records that a frame arrived without handing over its pixels.
    pub fn tick(&self, time: Duration) {
        *self.last.lock().unwrap() = Some((time, Instant::now()));
    }

    /// The recording's length when it stops now, on the frames' clock.
    fn end_time(&self) -> Duration {
        match *self.last.lock().unwrap() {
            Some((time, at)) => time + at.elapsed(),
            None => Duration::ZERO,
        }
    }
}

fn bgra_to_rgba(width: u32, height: u32, stride: usize, data: &[u8]) -> Option<RgbaImage> {
    let row = width as usize * 4;
    if stride < row || data.len() < stride * (height as usize - 1) + row {
        return None;
    }
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        for px in data[y * stride..y * stride + row].chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    RgbaImage::from_raw(width, height, out)
}

/// Output size: the selection in pixels for MP4 (fit in 4K), the selection in logical
/// pixels for GIF (fit in `GIF_MAX`). Video encoders need even sizes.
fn output_size(format: Format, width: u32, height: u32, dpi_scale: f64) -> (u32, u32) {
    let (w, h) = (width.max(2) as f64, height.max(2) as f64);
    let scale = match format {
        Format::Mp4 => (MP4_MAX.0 as f64 / w).min(MP4_MAX.1 as f64 / h).min(1.0),
        Format::Gif => (1.0 / dpi_scale.max(1.0)).min(GIF_MAX as f64 / w.max(h)),
    };
    let even = |v: f64| ((v * scale).floor() as u32 & !1).max(2);
    (even(w), even(h))
}

struct Active {
    backend: Box<dyn Backend>,
    gif: Option<gif::Encoder>,
    feed: Arc<Feed>,
    path: PathBuf,
    format: Format,
    size: (u32, u32),
    started: Instant,
}

#[derive(Default)]
enum Phase {
    #[default]
    Idle,
    Starting,
    Recording(Box<Active>),
    Stopping,
}

#[derive(Default)]
pub struct RecordState(Mutex<Phase>);

/// True from the moment a recording starts until its file is finished.
pub fn is_busy(app: &AppHandle) -> bool {
    !matches!(*app.state::<RecordState>().0.lock().unwrap(), Phase::Idle)
}

fn is_recording(app: &AppHandle) -> bool {
    matches!(
        *app.state::<RecordState>().0.lock().unwrap(),
        Phase::Recording(_)
    )
}

/// The format picked last time, so the overlay starts with it.
pub fn last_format(app: &AppHandle) -> Format {
    app.state::<config::ConfigState>()
        .0
        .lock()
        .unwrap()
        .record_format
        .unwrap_or_default()
}

/// Shortcut, tray click and menu item: stop the recording if one is running,
/// otherwise open the overlay to start one.
pub fn toggle(app: &AppHandle) {
    if is_recording(app) {
        stop(app, false);
    } else if !is_busy(app) {
        capture::start_record(app);
    }
}

fn show_error(app: &AppHandle, text: String) {
    log::error!("recording: {text}");
    app.dialog()
        .message(text)
        .title("Kadro")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

/// Called by the overlay with the selected area: closes it and starts recording.
#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    window: WebviewWindow,
    rect: capture::Rect,
    format: Format,
) -> Result<(), String> {
    {
        let state = app.state::<RecordState>();
        let mut phase = state.0.lock().unwrap();
        if !matches!(*phase, Phase::Idle) {
            return Err("Já existe uma gravação em andamento.".into());
        }
        *phase = Phase::Starting;
    }
    let result = prepare(&app, &window, rect, format);
    capture::close_all(&app);
    let target = match result {
        Ok(target) => target,
        Err(err) => {
            *app.state::<RecordState>().0.lock().unwrap() = Phase::Idle;
            return Err(err);
        }
    };
    {
        let state = app.state::<config::ConfigState>();
        let mut config = state.0.lock().unwrap();
        if config.record_format != Some(format) {
            config.record_format = Some(format);
            let _ = config::store(&app, &config);
        }
    }
    std::thread::spawn(move || begin(&app, target));
    Ok(())
}

fn prepare(
    app: &AppHandle,
    window: &WebviewWindow,
    rect: capture::Rect,
    format: Format,
) -> Result<Target, String> {
    let index = capture::overlay_index(window)?;
    let (shot_w, shot_h) = capture::shot_size(app, index).ok_or("captura não encontrada")?;
    let monitors = xcap::Monitor::all().map_err(|e| e.to_string())?;
    let m = monitors.get(index).ok_or("monitor não encontrado")?;
    let (width, height) = (m.width().unwrap_or(shot_w), m.height().unwrap_or(shot_h));
    let px_per_unit = shot_w as f64 / width.max(1) as f64;
    // GIFs are sized in logical pixels: points on macOS, scaled pixels on Windows.
    let dpi_scale = if cfg!(target_os = "macos") {
        px_per_unit
    } else {
        m.scale_factor().unwrap_or(1.0) as f64
    };

    // Keep the selection inside the screenshot.
    let x = rect.x.min(shot_w.saturating_sub(2));
    let y = rect.y.min(shot_h.saturating_sub(2));
    let rect = capture::Rect {
        x,
        y,
        width: rect.width.clamp(2, shot_w - x),
        height: rect.height.clamp(2, shot_h - y),
    };
    let size = output_size(format, rect.width, rect.height, dpi_scale);
    let path =
        capture::unique_path_ext(&config::save_dir(app)?, chrono::Local::now(), format.ext());
    Ok(Target {
        monitor: Monitor {
            id: m.id().map_err(|e| e.to_string())?,
            x: m.x().unwrap_or(0),
            y: m.y().unwrap_or(0),
            width,
            height,
            px_per_unit,
        },
        rect,
        size,
        fps: match format {
            Format::Mp4 => MP4_FPS,
            Format::Gif => GIF_FPS,
        },
        format,
        path,
    })
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start_backend(target: &Target, feed: Arc<Feed>) -> Result<Box<dyn Backend>, String> {
    platform::start(target, feed)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn start_backend(_target: &Target, _feed: Arc<Feed>) -> Result<Box<dyn Backend>, String> {
    Err("A gravação de tela só funciona no macOS e no Windows.".into())
}

/// Runs off the main thread: opens the control bar, then starts the backend.
fn begin(app: &AppHandle, target: Target) {
    // Let the overlay disappear before the first frame.
    std::thread::sleep(Duration::from_millis(250));
    // The bar exists before capturing starts: on macOS that is what lets the
    // recording leave all of Kadro's windows out.
    if let Err(err) = open_bar(app, target.format) {
        log::error!("could not open the recording bar: {err}");
    }

    let gif = match target.format {
        Format::Gif => match gif::spawn(&target.path, target.size) {
            Ok(gif) => Some(gif),
            Err(err) => return fail(app, &target.path, err),
        },
        Format::Mp4 => None,
    };
    let feed = Arc::new(Feed {
        gif: gif.as_ref().map(|(tx, _)| tx.clone()),
        thumb: Mutex::new(None),
        last: Mutex::new(None),
    });

    match start_backend(&target, feed.clone()) {
        Ok(backend) => {
            *app.state::<RecordState>().0.lock().unwrap() = Phase::Recording(Box::new(Active {
                backend,
                gif,
                feed,
                path: target.path,
                format: target.format,
                size: target.size,
                started: Instant::now(),
            }));
            let _ = app.emit_to(BAR_LABEL, "recording-started", ());
            tray::set_recording(app, true);
        }
        Err(err) => {
            drop(feed);
            if let Some((tx, handle)) = gif {
                drop(tx);
                let _ = handle.join();
            }
            fail(
                app,
                &target.path,
                format!("Não foi possível iniciar a gravação.\n\n{err}"),
            );
        }
    }
}

fn fail(app: &AppHandle, path: &std::path::Path, err: String) {
    close_bar(app);
    let _ = std::fs::remove_file(path);
    *app.state::<RecordState>().0.lock().unwrap() = Phase::Idle;
    tray::set_recording(app, false);
    show_error(app, err);
}

/// Stops the recording. `discard` deletes the file instead of keeping it.
pub fn stop(app: &AppHandle, discard: bool) {
    let active = {
        let state = app.state::<RecordState>();
        let mut phase = state.0.lock().unwrap();
        match std::mem::replace(&mut *phase, Phase::Stopping) {
            Phase::Recording(active) => active,
            other => {
                *phase = other;
                return;
            }
        }
    };
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = app.emit_to(BAR_LABEL, "recording-stopping", ());
        let result = finish(&app, *active, discard);
        close_bar(&app);
        *app.state::<RecordState>().0.lock().unwrap() = Phase::Idle;
        tray::set_recording(&app, false);
        match result {
            Ok(Some(_)) if history::limit(&app) > 0 => gallery::show(&app),
            // History turned off: show the file instead.
            Ok(Some(path)) => {
                if let Err(err) = app.opener().reveal_item_in_dir(&path) {
                    log::error!("could not reveal {}: {err}", path.display());
                }
            }
            Ok(None) => {}
            Err(err) => show_error(&app, err),
        }
    });
}

/// Finishes the file and adds it to the history. Returns the file, unless discarded.
fn finish(app: &AppHandle, active: Active, discard: bool) -> Result<Option<PathBuf>, String> {
    let Active {
        backend,
        gif,
        feed,
        path,
        format,
        size,
        started,
    } = active;
    let duration = started.elapsed();
    let stopped = backend.stop();
    let encoded = match gif {
        Some((tx, handle)) => {
            let _ = tx.send(gif::Msg::End(feed.end_time()));
            drop(tx);
            match handle.join() {
                Ok(Ok(0)) => Err("Nenhum quadro foi gravado.".to_string()),
                Ok(Ok(_)) => Ok(()),
                Ok(Err(err)) => Err(err),
                Err(_) => Err("o codificador de GIF parou".to_string()),
            }
        }
        None => Ok(()),
    };
    let thumb = feed.thumb.lock().unwrap().take();
    drop(feed);

    if discard {
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    }
    if let Err(err) = stopped.and(encoded) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("Não foi possível salvar a gravação.\n\n{err}"));
    }
    let Some(thumb) = thumb else {
        let _ = std::fs::remove_file(&path);
        return Err("Nada foi gravado. Tente de novo.".into());
    };
    history::add_recording(app, &thumb, &path, format, size, duration);
    Ok(Some(path))
}

// ---- Control bar ----

fn open_bar(app: &AppHandle, format: Format) -> Result<(), String> {
    close_bar(app);
    // Bottom center of the screen under the cursor (where the user just clicked).
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|c| app.monitor_from_point(c.x, c.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
        .ok_or("nenhum monitor encontrado")?;
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let init = format!("window.__KADRO_REC__ = {{ format: \"{}\" }};", format.ext());

    let builder = WebviewWindowBuilder::new(app, BAR_LABEL, WebviewUrl::App("record.html".into()))
        .title("Gravação do Kadro")
        .initialization_script(&init)
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        .visible(false);

    #[cfg(target_os = "macos")]
    let window = {
        let x = (area.position.x as f64 + area.size.width as f64 / 2.0) / scale - BAR_WIDTH / 2.0;
        let y =
            (area.position.y as f64 + area.size.height as f64) / scale - BAR_HEIGHT - BAR_MARGIN;
        builder
            .accept_first_mouse(true)
            .position(x, y)
            .inner_size(BAR_WIDTH, BAR_HEIGHT)
            .build()
            .map_err(|e| e.to_string())?
    };

    #[cfg(not(target_os = "macos"))]
    let window = {
        use tauri::{PhysicalPosition, PhysicalSize};
        let size = PhysicalSize::new(
            (BAR_WIDTH * scale).round() as u32,
            (BAR_HEIGHT * scale).round() as u32,
        );
        let pos = PhysicalPosition::new(
            area.position.x + (area.size.width as i32 - size.width as i32) / 2,
            area.position.y + area.size.height as i32
                - size.height as i32
                - (BAR_MARGIN * scale).round() as i32,
        );
        let window = builder.build().map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
        window.set_position(pos).map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
        window
    };

    #[cfg(target_os = "windows")]
    windows::exclude_from_capture(&window);

    window.show().map_err(|e| e.to_string())?;
    Ok(())
}

fn close_bar(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(BAR_LABEL) {
        let _ = window.destroy();
    }
}

#[tauri::command]
pub fn stop_recording(app: AppHandle, discard: bool) {
    stop(&app, discard);
}

/// Milliseconds since the recording started, or `None` if it hasn't (yet). The bar
/// asks once loaded, in case it missed the "recording-started" event.
#[tauri::command]
pub fn recording_elapsed(app: AppHandle) -> Option<u64> {
    match &*app.state::<RecordState>().0.lock().unwrap() {
        Phase::Recording(active) => Some(active.started.elapsed().as_millis() as u64),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mp4_keeps_pixels_and_even_sizes() {
        assert_eq!(output_size(Format::Mp4, 1001, 601, 2.0), (1000, 600));
        assert_eq!(output_size(Format::Mp4, 5120, 2880, 2.0), (3840, 2160));
        assert_eq!(output_size(Format::Mp4, 1, 1, 1.0), (2, 2));
    }

    #[test]
    fn gif_uses_logical_pixels_and_a_cap() {
        assert_eq!(output_size(Format::Gif, 1600, 1000, 2.0), (800, 500));
        assert_eq!(output_size(Format::Gif, 1920, 1080, 1.0), (1280, 720));
        assert_eq!(output_size(Format::Gif, 3840, 2160, 1.5), (1280, 720));
    }

    #[test]
    fn converts_padded_bgra() {
        let data = [1, 2, 3, 0, 9, 9, 9, 9, 4, 5, 6, 0, 9, 9, 9, 9];
        let img = bgra_to_rgba(1, 2, 8, &data).unwrap();
        assert_eq!(img.as_raw(), &[3, 2, 1, 255, 6, 5, 4, 255]);
    }
}
