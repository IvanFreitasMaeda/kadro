//! Clipboard history, like Windows' Win+V. Kept apart from the captures gallery: its
//! own popup, shortcut, folder and retention settings.
//!
//! Watching is cheap: a thread reads the clipboard's change counter twice a second
//! (one integer, the clipboard itself is not opened) and only reads the contents when
//! that number moves. Items live in `<app data>/clipboard/`: `index.json` with the
//! list, plus `<id>.png` and a `<id>.jpg` thumbnail per image, and `<id>.txt` for a
//! text too long to keep whole in the index.

pub mod popup;

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    pub fn change_count() -> i64 {
        0
    }
    pub fn should_skip() -> bool {
        false
    }
    pub fn remember_foreground() {}
    pub fn activate_previous() {}
    pub fn send_paste() {}
    pub fn can_paste(_prompt: bool) -> bool {
        false
    }
}

pub use popup::toggle;

use std::{
    fs,
    hash::{Hash, Hasher},
    io::Cursor,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

use image::{codecs::jpeg::JpegEncoder, imageops, ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::config;

pub const DEFAULT_LIMIT: usize = 100;
pub const DEFAULT_DAYS: u32 = 30;
/// Event sent to the popup when the list changes.
const CHANGED: &str = "clipboard-changed";
/// How often the change counter is read.
const POLL: Duration = Duration::from_millis(500);
/// Some apps put several formats on the clipboard one after the other: give them a
/// moment before reading.
const SETTLE: Duration = Duration::from_millis(120);
/// How much of a text is kept in the index, for the preview and the search.
const PREVIEW_CHARS: usize = 2000;
/// Larger texts are left out (a whole log file, say): they would bloat the folder.
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
/// Larger images are left out (about 8K by 8K).
const MAX_IMAGE_PIXELS: usize = 64_000_000;
/// Thumbnails fit in this box: the popup's width on a 2x display.
const THUMB_W: u32 = 680;
const THUMB_H: u32 = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Link,
    Image,
    Files,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// Unix time in milliseconds when first copied, as text; also the file name.
    id: String,
    /// Unix time in milliseconds of the last copy.
    time: i64,
    kind: Kind,
    #[serde(default)]
    pinned: bool,
    /// Content hash, to spot the same thing copied again.
    hash: String,
    /// Text and links: the start of the text. Files: unused.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    text: String,
    /// The text is longer than `text`; the whole of it is in `<id>.txt`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    long: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    width: u32,
    #[serde(default, skip_serializing_if = "is_zero")]
    height: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    paths: Vec<PathBuf>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Default)]
pub struct ClipState {
    entries: Mutex<Vec<Entry>>,
}

/// Held while Kadro itself writes to the clipboard, so the watcher doesn't record
/// that write as something new.
static WRITING: Mutex<()> = Mutex::new(());
/// Change counter right after Kadro's own last write.
static OWN_COUNT: AtomicI64 = AtomicI64::new(i64::MIN);
/// The Accessibility prompt is shown at most once per launch.
static PROMPTED: AtomicBool = AtomicBool::new(false);

fn dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|d| d.join("clipboard"))
        .map_err(|e| e.to_string())
}

/// Ids come from the webview and end up in file names: digits only.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() < 20 && id.bytes().all(|b| b.is_ascii_digit())
}

pub fn limit(app: &AppHandle) -> usize {
    let state = app.state::<config::ConfigState>();
    let config = state.0.lock().unwrap();
    config.clip_limit.unwrap_or(DEFAULT_LIMIT)
}

pub fn days(app: &AppHandle) -> u32 {
    let state = app.state::<config::ConfigState>();
    let config = state.0.lock().unwrap();
    config.clip_days.unwrap_or(DEFAULT_DAYS)
}

pub fn autopaste(app: &AppHandle) -> bool {
    let state = app.state::<config::ConfigState>();
    let config = state.0.lock().unwrap();
    config.clip_autopaste.unwrap_or(true)
}

/// Whether Kadro may type the paste shortcut into another app (on macOS this needs
/// the Accessibility permission).
pub fn paste_allowed() -> bool {
    platform::can_paste(false)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// Storage

fn load(app: &AppHandle) -> Vec<Entry> {
    dir(app)
        .ok()
        .and_then(|d| fs::read(d.join("index.json")).ok())
        .and_then(|bytes| serde_json::from_slice::<Vec<Entry>>(&bytes).ok())
        .map(|list| list.into_iter().filter(|e| valid_id(&e.id)).collect())
        .unwrap_or_default()
}

fn store(app: &AppHandle, entries: &[Entry]) {
    let result = (|| -> Result<(), String> {
        let dir = dir(app)?;
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let tmp = dir.join("index.json.tmp");
        fs::write(
            &tmp,
            serde_json::to_vec(entries).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::rename(&tmp, dir.join("index.json")).map_err(|e| e.to_string())
    })();
    if let Err(err) = result {
        log::warn!("could not save the clipboard history: {err}");
    }
}

fn remove_files(dir: &Path, id: &str) {
    for ext in ["png", "jpg", "txt"] {
        let _ = fs::remove_file(dir.join(format!("{id}.{ext}")));
    }
}

/// Drops what the retention settings no longer keep. Pinned items always stay.
/// Returns whether anything was removed.
fn prune(app: &AppHandle, entries: &mut Vec<Entry>) -> bool {
    let limit = limit(app);
    let days = days(app);
    let oldest = (days > 0).then(|| now_ms() - i64::from(days) * 86_400_000);
    let mut kept = 0usize;
    let mut removed = Vec::new();
    entries.retain(|e| {
        if e.pinned {
            return true;
        }
        let keep = kept < limit && oldest.map_or(true, |t| e.time >= t);
        if keep {
            kept += 1;
        } else {
            removed.push(e.id.clone());
        }
        keep
    });
    if let Ok(dir) = dir(app) {
        for id in &removed {
            remove_files(&dir, id);
        }
    }
    !removed.is_empty()
}

/// Deletes files no entry refers to (left by a crash between writing and indexing).
fn sweep(app: &AppHandle, entries: &[Entry]) {
    let Ok(dir) = dir(app) else { return };
    let Ok(read) = fs::read_dir(&dir) else { return };
    for file in read.flatten() {
        let name = file.file_name();
        let name = name.to_string_lossy();
        let Some((stem, ext)) = name.rsplit_once('.') else {
            continue;
        };
        if matches!(ext, "png" | "jpg" | "txt") && !entries.iter().any(|e| e.id == stem) {
            let _ = fs::remove_file(file.path());
        }
    }
}

fn changed(app: &AppHandle) {
    let _ = app.emit_to(popup::LABEL, CHANGED, ());
}

// ---------------------------------------------------------------------------
// Watching

enum Copied {
    Text(String),
    Image(arboard::ImageData<'static>),
    Files(Vec<PathBuf>),
}

/// Reads what was just copied. Files first (Finder also puts the file name as
/// text), then text (Office also puts a picture of the text), then images.
fn read_clipboard() -> Option<Copied> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    if let Ok(files) = clipboard.get().file_list() {
        if !files.is_empty() {
            return Some(Copied::Files(files));
        }
    }
    if let Ok(text) = clipboard.get_text() {
        if !text.trim().is_empty() {
            return Some(Copied::Text(text));
        }
    }
    clipboard.get_image().ok().map(Copied::Image)
}

fn hash_of(kind: Kind, bytes: &[u8]) -> String {
    let mut hasher = std::hash::DefaultHasher::new();
    kind.hash(&mut hasher);
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn is_link(text: &str) -> bool {
    let t = text.trim();
    !t.contains(char::is_whitespace)
        && (t.starts_with("https://") || t.starts_with("http://") || t.starts_with("www."))
}

/// Starts watching the clipboard. Call once, at launch.
pub fn start(app: &AppHandle) {
    let mut entries = load(app);
    prune(app, &mut entries);
    sweep(app, &entries);
    store(app, &entries);
    app.manage(ClipState {
        entries: Mutex::new(entries),
    });

    let app = app.clone();
    std::thread::Builder::new()
        .name("clipboard-watch".into())
        .spawn(move || watch(app))
        .ok();
}

fn watch(app: AppHandle) {
    // What is on the clipboard at launch was seen before (or by no one): skip it.
    let mut last = platform::change_count();
    let mut last_prune = Instant::now();
    loop {
        std::thread::sleep(POLL);
        if last_prune.elapsed() > Duration::from_secs(600) {
            last_prune = Instant::now();
            let state = app.state::<ClipState>();
            let mut entries = state.entries.lock().unwrap();
            if prune(&app, &mut entries) {
                store(&app, &entries);
                drop(entries);
                changed(&app);
            }
        }
        let count = {
            let _guard = WRITING.lock().unwrap();
            platform::change_count()
        };
        if count == last {
            continue;
        }
        last = count;
        if count == OWN_COUNT.load(Ordering::SeqCst) || limit(&app) == 0 {
            continue;
        }
        std::thread::sleep(SETTLE);
        // Password managers mark what they copy as concealed or transient.
        if platform::should_skip() {
            continue;
        }
        let Some(copied) = read_clipboard() else {
            continue;
        };
        if let Err(err) = add(&app, copied) {
            log::warn!("clipboard history: {err}");
        }
    }
}

fn new_id(entries: &[Entry]) -> String {
    let mut ms = now_ms();
    while entries.iter().any(|e| e.id == ms.to_string()) {
        ms += 1;
    }
    ms.to_string()
}

fn add(app: &AppHandle, copied: Copied) -> Result<(), String> {
    let (kind, hash) = match &copied {
        Copied::Text(text) => {
            if text.len() > MAX_TEXT_BYTES {
                return Err("text too large, skipped".into());
            }
            let kind = if is_link(text) {
                Kind::Link
            } else {
                Kind::Text
            };
            (kind, hash_of(kind, text.as_bytes()))
        }
        Copied::Image(img) => {
            if img.width * img.height > MAX_IMAGE_PIXELS {
                return Err("image too large, skipped".into());
            }
            (Kind::Image, hash_of(Kind::Image, &img.bytes))
        }
        Copied::Files(paths) => {
            let joined = paths
                .iter()
                .map(|p| p.to_string_lossy())
                .collect::<Vec<_>>()
                .join("\n");
            (Kind::Files, hash_of(Kind::Files, joined.as_bytes()))
        }
    };

    let state = app.state::<ClipState>();
    // Copied again: move it to the top instead of keeping two.
    if bump(app, &state, &hash) {
        changed(app);
        return Ok(());
    }

    let id = new_id(&state.entries.lock().unwrap());
    let dir = dir(app)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut entry = Entry {
        id: id.clone(),
        time: now_ms(),
        kind,
        pinned: false,
        hash,
        text: String::new(),
        long: false,
        width: 0,
        height: 0,
        paths: Vec::new(),
    };
    // Files are written before the entry is listed, outside the lock.
    match copied {
        Copied::Text(text) => {
            let preview: String = text.chars().take(PREVIEW_CHARS).collect();
            if preview.len() < text.len() {
                fs::write(dir.join(format!("{id}.txt")), &text).map_err(|e| e.to_string())?;
                entry.long = true;
            }
            entry.text = preview;
        }
        Copied::Image(img) => {
            let image =
                RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())
                    .ok_or("invalid image")?;
            entry.width = image.width();
            entry.height = image.height();
            let png =
                crate::capture::encode_png(&image, image::codecs::png::CompressionType::Fast)?;
            fs::write(dir.join(format!("{id}.png")), png).map_err(|e| e.to_string())?;
            fs::write(dir.join(format!("{id}.jpg")), thumbnail(&image)?)
                .map_err(|e| e.to_string())?;
        }
        Copied::Files(paths) => entry.paths = paths,
    }

    {
        let mut entries = state.entries.lock().unwrap();
        entries.insert(0, entry);
        prune(app, &mut entries);
        store(app, &entries);
    }
    changed(app);
    Ok(())
}

/// Moves the entry with this hash to the top. Returns whether there was one.
fn bump(app: &AppHandle, state: &ClipState, hash: &str) -> bool {
    let mut entries = state.entries.lock().unwrap();
    let Some(pos) = entries.iter().position(|e| e.hash == hash) else {
        return false;
    };
    let mut entry = entries.remove(pos);
    entry.time = now_ms();
    entries.insert(0, entry);
    store(app, &entries);
    true
}

/// A small JPEG over a white background (JPEG has no transparency).
fn thumbnail(image: &RgbaImage) -> Result<Vec<u8>, String> {
    let scale = (THUMB_W as f32 / image.width() as f32)
        .min(THUMB_H as f32 / image.height() as f32)
        .min(1.0);
    let w = ((image.width() as f32 * scale).round() as u32).max(1);
    let h = ((image.height() as f32 * scale).round() as u32).max(1);
    let small = imageops::thumbnail(image, w, h);
    let mut rgb = image::RgbImage::new(w, h);
    for (dst, src) in rgb.pixels_mut().zip(small.pixels()) {
        let a = src[3] as u32;
        for c in 0..3 {
            dst[c] = ((src[c] as u32 * a + 255 * (255 - a)) / 255) as u8;
        }
    }
    let mut out = Vec::new();
    JpegEncoder::new_with_quality(&mut Cursor::new(&mut out), 80)
        .encode_image(&rgb)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// Thumbnails for the popup, served as `kadro://localhost/c/<id>.jpg`.
pub fn serve<R: Runtime>(app: &AppHandle<R>, name: &str) -> Option<(Vec<u8>, &'static str)> {
    let id = name.strip_suffix(".jpg")?;
    if !valid_id(id) {
        return None;
    }
    fs::read(dir(app).ok()?.join(name))
        .ok()
        .map(|bytes| (bytes, "image/jpeg"))
}

// ---------------------------------------------------------------------------
// Using an item

fn find(app: &AppHandle, id: &str) -> Result<Entry, String> {
    app.state::<ClipState>()
        .entries
        .lock()
        .unwrap()
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or_else(|| "Esse item não está mais no histórico.".into())
}

fn write_clipboard(app: &AppHandle, entry: &Entry) -> Result<(), String> {
    let dir = dir(app)?;
    let err = |e: arboard::Error| format!("Não foi possível copiar: {e}");
    let _guard = WRITING.lock().unwrap();
    let mut clipboard = arboard::Clipboard::new().map_err(err)?;
    match entry.kind {
        Kind::Text | Kind::Link => {
            let text = if entry.long {
                fs::read_to_string(dir.join(format!("{}.txt", entry.id)))
                    .map_err(|e| e.to_string())?
            } else {
                entry.text.clone()
            };
            clipboard.set_text(text).map_err(err)?;
        }
        Kind::Image => {
            let bytes = fs::read(dir.join(format!("{}.png", entry.id)))
                .map_err(|_| "A imagem não foi encontrada.".to_string())?;
            let image = image::load_from_memory_with_format(&bytes, ImageFormat::Png)
                .map_err(|e| e.to_string())?
                .into_rgba8();
            clipboard
                .set_image(arboard::ImageData {
                    width: image.width() as usize,
                    height: image.height() as usize,
                    bytes: image.into_raw().into(),
                })
                .map_err(err)?;
        }
        Kind::Files => {
            let paths: Vec<&PathBuf> = entry.paths.iter().filter(|p| p.exists()).collect();
            if paths.is_empty() {
                return Err("Os arquivos foram movidos ou apagados.".into());
            }
            clipboard.set().file_list(&paths).map_err(err)?;
        }
    }
    OWN_COUNT.store(platform::change_count(), Ordering::SeqCst);
    Ok(())
}

/// Puts an item back on the clipboard, closes the popup and, when `paste` is set and
/// allowed, pastes it into the app that was in front. Returns whether it pasted.
#[tauri::command]
pub async fn clip_use(app: AppHandle, id: String, paste: bool) -> Result<bool, String> {
    let entry = find(&app, &id)?;
    write_clipboard(&app, &entry)?;
    let state = app.state::<ClipState>();
    bump(&app, &state, &entry.hash);

    let mut pasting = paste && autopaste(&app);
    if pasting && !platform::can_paste(false) {
        pasting = false;
        // macOS: ask for Accessibility once; this time it only copies.
        if !PROMPTED.swap(true, Ordering::SeqCst) {
            platform::can_paste(true);
        }
    }
    popup::close(&app);
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        platform::activate_previous();
        if pasting {
            // Give the other app a moment to take the focus back.
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(120));
                platform::send_paste();
            });
        }
        changed(&handle);
    });
    Ok(pasting)
}

#[tauri::command]
pub fn clip_list(app: AppHandle) -> Vec<Entry> {
    app.state::<ClipState>().entries.lock().unwrap().clone()
}

#[tauri::command]
pub fn clip_pin(app: AppHandle, id: String, pinned: bool) -> Result<(), String> {
    let state = app.state::<ClipState>();
    let mut entries = state.entries.lock().unwrap();
    let entry = entries
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or("Esse item não está mais no histórico.")?;
    entry.pinned = pinned;
    if !pinned {
        prune(&app, &mut entries);
    }
    store(&app, &entries);
    Ok(())
}

#[tauri::command]
pub fn clip_delete(app: AppHandle, id: String) -> Result<(), String> {
    if !valid_id(&id) {
        return Err("item inválido".into());
    }
    let state = app.state::<ClipState>();
    let mut entries = state.entries.lock().unwrap();
    entries.retain(|e| e.id != id);
    remove_files(&dir(&app)?, &id);
    store(&app, &entries);
    Ok(())
}

/// Clears the history. Pinned items stay unless `pinned` is set.
#[tauri::command]
pub fn clip_clear(app: AppHandle, pinned: bool) -> Result<(), String> {
    clear(&app, pinned)
}

fn clear(app: &AppHandle, pinned: bool) -> Result<(), String> {
    let state = app.state::<ClipState>();
    let mut entries = state.entries.lock().unwrap();
    let dir = dir(app)?;
    entries.retain(|e| {
        let keep = e.pinned && !pinned;
        if !keep {
            remove_files(&dir, &e.id);
        }
        keep
    });
    store(app, &entries);
    drop(entries);
    changed(app);
    Ok(())
}

/// Saves the retention settings and applies them right away. A limit of 0 turns the
/// history off and clears what isn't pinned.
#[tauri::command]
pub fn set_clip_retention(app: AppHandle, limit: usize, days: u32) -> Result<(), String> {
    {
        let state = app.state::<config::ConfigState>();
        let mut config = state.0.lock().unwrap();
        config.clip_limit = (limit != DEFAULT_LIMIT).then_some(limit);
        config.clip_days = (days != DEFAULT_DAYS).then_some(days);
        config::store(&app, &config)?;
    }
    let state = app.state::<ClipState>();
    let mut entries = state.entries.lock().unwrap();
    prune(&app, &mut entries);
    store(&app, &entries);
    drop(entries);
    changed(&app);
    Ok(())
}

/// Turns pasting into the previous app on or off. On macOS, turning it on asks for
/// the Accessibility permission. Returns whether pasting is allowed now.
#[tauri::command]
pub fn set_clip_autopaste(app: AppHandle, enabled: bool) -> Result<bool, String> {
    {
        let state = app.state::<config::ConfigState>();
        let mut config = state.0.lock().unwrap();
        config.clip_autopaste = (!enabled).then_some(false);
        config::store(&app, &config)?;
    }
    Ok(enabled && platform::can_paste(true))
}
