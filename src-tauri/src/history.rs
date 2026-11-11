//! Recent captures for the side gallery, kept in the app data folder.
//!
//! Each capture is stored as two files named after its id, `<millis>-<width>x<height>`:
//! the full image as PNG and a small JPEG thumbnail. The folder listing is the index,
//! so there is nothing else to keep in sync.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::{DateTime, Local, TimeZone};
use image::{codecs::jpeg::JpegEncoder, imageops, DynamicImage, ImageFormat, RgbaImage};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::{capture, config};

pub const DEFAULT_LIMIT: usize = 30;
/// Thumbnails fit in this box, enough for the gallery width on a 2x display.
const THUMB_MAX: u32 = 560;
const THUMB_QUALITY: u8 = 80;
/// Event sent to the gallery when the list changes.
pub const CHANGED: &str = "history-changed";

/// Serializes writes so pruning never races a capture being added.
static LOCK: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
pub struct Item {
    id: String,
    /// Unix time in milliseconds.
    time: i64,
    width: u32,
    height: u32,
}

fn dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("history"))
        .map_err(|e| e.to_string())
}

pub fn limit(app: &AppHandle) -> usize {
    app.state::<config::ConfigState>()
        .0
        .lock()
        .unwrap()
        .history_limit
        .unwrap_or(DEFAULT_LIMIT)
}

/// Parses an id, rejecting anything that isn't exactly `<millis>-<w>x<h>`
/// (ids come from the webview and end up in file paths).
fn parse_id(id: &str) -> Option<Item> {
    let (time, size) = id.split_once('-')?;
    let (w, h) = size.split_once('x')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(time) && digits(w) && digits(h)) {
        return None;
    }
    Some(Item {
        id: id.to_string(),
        time: time.parse().ok()?,
        width: w.parse().ok()?,
        height: h.parse().ok()?,
    })
}

fn png_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.png"))
}

fn thumb_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.jpg"))
}

/// The capture's full image, after checking the id.
fn image_path(app: &AppHandle, id: &str) -> Result<(Item, PathBuf), String> {
    let item = parse_id(id).ok_or("captura inválida")?;
    let path = png_path(&dir(app)?, id);
    if !path.exists() {
        return Err("Essa captura não está mais no histórico.".into());
    }
    Ok((item, path))
}

/// Every capture in the history, newest first.
fn items(dir: &Path) -> Vec<Item> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<Item> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let id = name.to_str()?.strip_suffix(".png")?;
            parse_id(id)
        })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.time));
    items
}

fn remove(dir: &Path, id: &str) {
    let _ = fs::remove_file(png_path(dir, id));
    let _ = fs::remove_file(thumb_path(dir, id));
}

/// Deletes the oldest captures beyond `limit`.
fn prune(dir: &Path, limit: usize) {
    for item in items(dir).into_iter().skip(limit) {
        remove(dir, &item.id);
    }
}

/// Adds a capture in the background, so copying or saving isn't slowed down.
pub fn add(app: &AppHandle, img: RgbaImage) {
    let limit = limit(app);
    if limit == 0 {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || match store(&app, &img, limit) {
        Ok(()) => {
            let _ = app.emit(CHANGED, ());
        }
        Err(err) => log::error!("could not add to history: {err}"),
    });
}

fn store(app: &AppHandle, img: &RgbaImage, limit: usize) -> Result<(), String> {
    let thumb = fit_thumb(img);
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, THUMB_QUALITY)
        .encode_image(&thumb)
        .map_err(|e| e.to_string())?;
    let png = capture::encode_png(img, image::codecs::png::CompressionType::Default)?;

    let _guard = LOCK.lock().unwrap();
    let dir = dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut time = Local::now().timestamp_millis();
    let id = loop {
        let id = format!("{time}-{}x{}", img.width(), img.height());
        if !png_path(&dir, &id).exists() {
            break id;
        }
        time += 1;
    };
    // Thumbnail first: the listing looks for the PNG, so an item only shows up
    // once both files are there.
    fs::write(thumb_path(&dir, &id), jpeg).map_err(|e| e.to_string())?;
    fs::write(png_path(&dir, &id), png).map_err(|e| e.to_string())?;
    prune(&dir, limit);
    Ok(())
}

/// Scales the image down to fit in `THUMB_MAX`, keeping its aspect ratio.
fn fit_thumb(img: &RgbaImage) -> image::RgbImage {
    let (w, h) = (img.width(), img.height());
    let scale = (THUMB_MAX as f64 / w.max(h) as f64).min(1.0);
    let tw = ((w as f64 * scale).round() as u32).max(1);
    let th = ((h as f64 * scale).round() as u32).max(1);
    DynamicImage::ImageRgba8(imageops::thumbnail(img, tw, th)).to_rgb8()
}

/// Serves `h/<id>.jpg` (thumbnail) or `h/<id>.png` (full image) for the gallery.
pub fn serve<R: Runtime>(app: &AppHandle<R>, name: &str) -> Option<(Vec<u8>, &'static str)> {
    let (id, mime) = if let Some(id) = name.strip_suffix(".jpg") {
        (id, "image/jpeg")
    } else {
        (name.strip_suffix(".png")?, "image/png")
    };
    parse_id(id)?;
    let dir = dir(app).ok()?;
    let path = if mime == "image/jpeg" {
        thumb_path(&dir, id)
    } else {
        png_path(&dir, id)
    };
    fs::read(path).ok().map(|bytes| (bytes, mime))
}

fn local_time(item: &Item) -> DateTime<Local> {
    Local
        .timestamp_millis_opt(item.time)
        .single()
        .unwrap_or_else(Local::now)
}

#[tauri::command]
pub fn history_list(app: AppHandle) -> Vec<Item> {
    dir(&app).map(|d| items(&d)).unwrap_or_default()
}

#[tauri::command]
pub async fn history_copy(app: AppHandle, id: String) -> Result<(), String> {
    let (_, path) = image_path(&app, &id)?;
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let img = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    capture::copy_to_clipboard(&app, img)
}

/// Copies the capture to the configured folder and returns the new file's path.
#[tauri::command]
pub async fn history_save(app: AppHandle, id: String) -> Result<String, String> {
    let (item, src) = image_path(&app, &id)?;
    let dest = capture::unique_path(&config::save_dir(&app)?, local_time(&item));
    fs::copy(&src, &dest).map_err(|e| format!("não foi possível salvar: {e}"))?;
    Ok(dest.display().to_string())
}

/// Asks where to save the capture. Returns the path, or `None` if cancelled.
#[tauri::command]
pub async fn history_save_as(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<Option<String>, String> {
    let (item, src) = image_path(&app, &id)?;
    let dir = config::save_dir(&app).unwrap_or_else(|_| config::default_save_dir());
    let chosen = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Salvar captura")
        .set_directory(dir)
        .set_file_name(capture::file_name(local_time(&item)))
        .add_filter("Imagem PNG", &["png"])
        .blocking_save_file();
    let Some(path) = chosen.and_then(|p| p.as_path().map(Path::to_path_buf)) else {
        return Ok(None);
    };
    let path = if path.extension().is_none() {
        path.with_extension("png")
    } else {
        path
    };
    fs::copy(&src, &path).map_err(|e| format!("não foi possível salvar: {e}"))?;
    Ok(Some(path.display().to_string()))
}

/// Opens the capture in the system's default image viewer.
#[tauri::command]
pub fn history_open(app: AppHandle, id: String) -> Result<(), String> {
    let (_, path) = image_path(&app, &id)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_delete(app: AppHandle, id: String) -> Result<(), String> {
    parse_id(&id).ok_or("captura inválida")?;
    let _guard = LOCK.lock().unwrap();
    remove(&dir(&app)?, &id);
    Ok(())
}

#[tauri::command]
pub fn history_clear(app: AppHandle) -> Result<(), String> {
    let _guard = LOCK.lock().unwrap();
    prune(&dir(&app)?, 0);
    Ok(())
}

/// Saves how many captures to keep (0 turns the history off) and drops the extra ones.
#[tauri::command]
pub fn set_history_limit(app: AppHandle, limit: usize) -> Result<(), String> {
    {
        let state = app.state::<config::ConfigState>();
        let mut config = state.0.lock().unwrap();
        config.history_limit = (limit != DEFAULT_LIMIT).then_some(limit);
        config::store(&app, &config)?;
    }
    let _guard = LOCK.lock().unwrap();
    prune(&dir(&app)?, limit);
    let _ = app.emit(CHANGED, ());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_strict() {
        let item = parse_id("1731360000000-1440x900").unwrap();
        assert_eq!(
            (item.time, item.width, item.height),
            (1731360000000, 1440, 900)
        );
        for bad in [
            "",
            "../x",
            "1-2x",
            "1-x3",
            "a-1x1",
            "1-1x1/..",
            "1-1x1.png",
            "-1x1",
        ] {
            assert!(parse_id(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn keeps_newest_within_limit() {
        let dir = std::env::temp_dir().join(format!("kadro-history-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        for t in 1..=5 {
            let id = format!("{t}-1x1");
            fs::write(png_path(&dir, &id), b"").unwrap();
            fs::write(thumb_path(&dir, &id), b"").unwrap();
        }
        prune(&dir, 2);
        let ids: Vec<_> = items(&dir).into_iter().map(|i| i.id).collect();
        assert_eq!(ids, ["5-1x1", "4-1x1"]);
        assert!(!thumb_path(&dir, "1-1x1").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn thumbnails_keep_aspect_ratio() {
        let thumb = fit_thumb(&RgbaImage::new(2880, 900));
        assert_eq!((thumb.width(), thumb.height()), (THUMB_MAX, 175));
        let small = fit_thumb(&RgbaImage::new(100, 40));
        assert_eq!((small.width(), small.height()), (100, 40));
    }
}
