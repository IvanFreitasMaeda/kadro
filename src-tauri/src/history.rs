//! Recent captures for the side gallery, kept in the app data folder.
//!
//! Each capture is stored as two files named after its id, `<millis>-<width>x<height>`:
//! a small JPEG thumbnail, plus the full image as PNG or, for a recording, a `.rec`
//! file pointing at the video in the captures folder (videos are too big to keep a
//! second copy). The folder listing is the index, so there is nothing else to keep
//! in sync.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use chrono::{DateTime, Local, TimeZone};
use image::{codecs::jpeg::JpegEncoder, imageops, DynamicImage, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::{capture, config, fileclip, record::Format};

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
    /// "image", "mp4" or "gif".
    kind: &'static str,
    /// Length of a recording, in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    duration: Option<u64>,
    /// The recording's file was moved or deleted.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    missing: bool,
}

/// Contents of a `.rec` file.
#[derive(Serialize, Deserialize)]
struct Recording {
    path: PathBuf,
    format: Format,
    duration_ms: u64,
}

/// What a history item's full content is.
enum Content {
    Image(PathBuf),
    Video(Recording),
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
        kind: "image",
        duration: None,
        missing: false,
    })
}

fn png_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.png"))
}

fn thumb_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.jpg"))
}

fn rec_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.rec"))
}

fn read_rec(path: &Path) -> Option<Recording> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// The capture's full image or video, after checking the id.
fn content(app: &AppHandle, id: &str) -> Result<(Item, Content), String> {
    let item = parse_id(id).ok_or("captura inválida")?;
    let dir = dir(app)?;
    let png = png_path(&dir, id);
    if png.exists() {
        return Ok((item, Content::Image(png)));
    }
    let rec = read_rec(&rec_path(&dir, id)).ok_or("Essa captura não está mais no histórico.")?;
    if !rec.path.exists() {
        return Err("O arquivo da gravação foi movido ou apagado.".into());
    }
    Ok((item, Content::Video(rec)))
}

impl Content {
    fn path(&self) -> &Path {
        match self {
            Content::Image(path) => path,
            Content::Video(rec) => &rec.path,
        }
    }

    fn ext(&self) -> &'static str {
        match self {
            Content::Image(_) => "png",
            Content::Video(rec) => rec.format.ext(),
        }
    }

    fn filter_name(&self) -> &'static str {
        match self {
            Content::Image(_) => "Imagem PNG",
            Content::Video(Recording {
                format: Format::Mp4,
                ..
            }) => "Vídeo MP4",
            Content::Video(_) => "GIF animado",
        }
    }
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
            let name = name.to_str()?;
            if let Some(id) = name.strip_suffix(".png") {
                return parse_id(id);
            }
            let mut item = parse_id(name.strip_suffix(".rec")?)?;
            let rec = read_rec(&e.path())?;
            item.kind = rec.format.ext();
            item.duration = Some(rec.duration_ms);
            item.missing = !rec.path.exists();
            Some(item)
        })
        .collect();
    items.sort_by_key(|item| std::cmp::Reverse(item.time));
    items
}

/// Removes an item from the history. A recording's video stays in the captures folder.
fn remove(dir: &Path, id: &str) {
    let _ = fs::remove_file(png_path(dir, id));
    let _ = fs::remove_file(rec_path(dir, id));
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
    let png = capture::encode_png(img, image::codecs::png::CompressionType::Default)?;
    write_item(app, img, (img.width(), img.height()), limit, |dir, id| {
        fs::write(png_path(dir, id), png).map_err(|e| e.to_string())
    })
}

/// Writes the thumbnail, then the item's main file with `write`, under a new id.
fn write_item(
    app: &AppHandle,
    img: &RgbaImage,
    (width, height): (u32, u32),
    limit: usize,
    write: impl FnOnce(&Path, &str) -> Result<(), String>,
) -> Result<(), String> {
    let thumb = fit_thumb(img);
    let mut jpeg = Vec::new();
    JpegEncoder::new_with_quality(&mut jpeg, THUMB_QUALITY)
        .encode_image(&thumb)
        .map_err(|e| e.to_string())?;

    let _guard = LOCK.lock().unwrap();
    let dir = dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut time = Local::now().timestamp_millis();
    let id = loop {
        let id = format!("{time}-{width}x{height}");
        if !thumb_path(&dir, &id).exists() {
            break id;
        }
        time += 1;
    };
    // Thumbnail first: the listing looks for the main file, so an item only shows
    // up once both files are there.
    fs::write(thumb_path(&dir, &id), jpeg).map_err(|e| e.to_string())?;
    write(&dir, &id)?;
    prune(&dir, limit);
    Ok(())
}

/// Adds a finished recording, with `thumb` (its first frame) as the thumbnail.
pub fn add_recording(
    app: &AppHandle,
    thumb: &RgbaImage,
    path: &Path,
    format: Format,
    size: (u32, u32),
    duration: std::time::Duration,
) {
    let limit = limit(app);
    if limit == 0 {
        return;
    }
    let rec = Recording {
        path: path.to_path_buf(),
        format,
        duration_ms: duration.as_millis() as u64,
    };
    let result = write_item(app, thumb, size, limit, |dir, id| {
        let json = serde_json::to_vec(&rec).map_err(|e| e.to_string())?;
        fs::write(rec_path(dir, id), json).map_err(|e| e.to_string())
    });
    match result {
        Ok(()) => {
            let _ = app.emit(CHANGED, ());
        }
        Err(err) => log::error!("could not add the recording to history: {err}"),
    }
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
/// Recordings only have the thumbnail.
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

/// Copies an image to the clipboard, or a recording as a file (to paste in a chat
/// or a folder).
#[tauri::command]
pub async fn history_copy(app: AppHandle, id: String) -> Result<(), String> {
    match content(&app, &id)?.1 {
        Content::Image(path) => {
            let bytes = fs::read(&path).map_err(|e| e.to_string())?;
            let img = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .map_err(|e| e.to_string())?
                .to_rgba8();
            capture::copy_to_clipboard(&app, img)
        }
        Content::Video(rec) => fileclip::copy_file(&rec.path),
    }
}

/// Copies the capture to the configured folder and returns the new file's path.
/// Recordings are already saved there, unless the folder changed since.
#[tauri::command]
pub async fn history_save(app: AppHandle, id: String) -> Result<String, String> {
    let (item, content) = content(&app, &id)?;
    let dir = config::save_dir(&app)?;
    let src = content.path();
    if src.parent() == Some(dir.as_path()) {
        return Ok(src.display().to_string());
    }
    let dest = capture::unique_path_ext(&dir, local_time(&item), content.ext());
    fs::copy(src, &dest).map_err(|e| format!("não foi possível salvar: {e}"))?;
    Ok(dest.display().to_string())
}

/// Asks where to save the capture. Returns the path, or `None` if cancelled.
#[tauri::command]
pub async fn history_save_as(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
) -> Result<Option<String>, String> {
    let (item, content) = content(&app, &id)?;
    let ext = content.ext();
    let dir = config::save_dir(&app).unwrap_or_else(|_| config::default_save_dir());
    let chosen = app
        .dialog()
        .file()
        .set_parent(&window)
        .set_title(if ext == "png" {
            "Salvar captura"
        } else {
            "Salvar gravação"
        })
        .set_directory(dir)
        .set_file_name(capture::file_name_ext(local_time(&item), ext))
        .add_filter(content.filter_name(), &[ext])
        .blocking_save_file();
    let Some(path) = chosen.and_then(|p| p.as_path().map(Path::to_path_buf)) else {
        return Ok(None);
    };
    let path = if path.extension().is_none() {
        path.with_extension(ext)
    } else {
        path
    };
    fs::copy(content.path(), &path).map_err(|e| format!("não foi possível salvar: {e}"))?;
    Ok(Some(path.display().to_string()))
}

/// Opens the capture in the system's default image viewer or video player.
#[tauri::command]
pub fn history_open(app: AppHandle, id: String) -> Result<(), String> {
    let (_, content) = content(&app, &id)?;
    app.opener()
        .open_path(content.path().to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Shows the recording's file in Finder / Explorer.
#[tauri::command]
pub fn history_reveal(app: AppHandle, id: String) -> Result<(), String> {
    let (_, content) = content(&app, &id)?;
    app.opener()
        .reveal_item_in_dir(content.path())
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
    fn lists_recordings() {
        let dir = std::env::temp_dir().join(format!("kadro-history-rec-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let video = dir.join("video.gif");
        fs::write(&video, b"GIF89a").unwrap();
        let rec = Recording {
            path: video.clone(),
            format: Format::Gif,
            duration_ms: 4200,
        };
        fs::write(
            rec_path(&dir, "7-640x360"),
            serde_json::to_vec(&rec).unwrap(),
        )
        .unwrap();
        fs::write(thumb_path(&dir, "7-640x360"), b"").unwrap();
        fs::write(png_path(&dir, "3-10x10"), b"").unwrap();

        let list = items(&dir);
        assert_eq!(list.len(), 2);
        assert_eq!(
            (list[0].kind, list[0].duration, list[0].missing),
            ("gif", Some(4200), false)
        );
        assert_eq!(list[1].kind, "image");

        fs::remove_file(&video).unwrap();
        assert!(items(&dir)[0].missing);
        remove(&dir, "7-640x360");
        assert_eq!(items(&dir).len(), 1);
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
