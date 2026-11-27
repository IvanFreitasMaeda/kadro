//! Drawing on the screen while recording.
//!
//! A transparent window sits exactly over the recorded area. It normally lets every
//! click through to the apps below; the draw shortcut (or the pencil on the control
//! bar) makes it take the mouse so the user can draw on it. The strokes are on screen,
//! so they end up in the MP4/GIF like anything else. On macOS this window is created
//! after the stream starts, which keeps it out of the list of Kadro windows that the
//! recording leaves out (see `macos.rs`); on Windows it simply isn't excluded.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use super::{Target, BAR_LABEL};
use crate::clipboard;

pub const LABEL: &str = "draw";

/// Whether the overlay takes the mouse (draw mode) or lets it through.
static DRAWING: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Serialize)]
struct Mode {
    on: bool,
}

/// Opens the overlay over the recorded area, click-through and empty.
pub fn open(app: &AppHandle, target: &Target) -> Result<(), String> {
    close(app);
    DRAWING.store(false, Ordering::SeqCst);
    let builder = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("draw.html".into()))
        .title("Desenho do Kadro")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focused(false)
        // draw.ts shows it once the page is up, so no blank frame gets recorded.
        .visible(false);

    let m = &target.monitor;
    let r = &target.rect;

    // macOS: the rect is in screenshot pixels, the monitor in points; Tauri's logical
    // coordinates are points with a top-left origin.
    #[cfg(target_os = "macos")]
    let window = {
        let s = m.px_per_unit.max(0.01);
        builder
            .accept_first_mouse(true)
            .position(m.x as f64 + r.x as f64 / s, m.y as f64 + r.y as f64 / s)
            .inner_size(r.width as f64 / s, r.height as f64 / s)
            .build()
            .map_err(|e| e.to_string())?
    };

    // Windows: both are in physical pixels. Size again after moving, in case the move
    // onto another monitor rescaled the window.
    #[cfg(not(target_os = "macos"))]
    let window = {
        use tauri::{PhysicalPosition, PhysicalSize};
        let size = PhysicalSize::new(r.width, r.height);
        let pos = PhysicalPosition::new(m.x + r.x as i32, m.y + r.y as i32);
        let window = builder.build().map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
        window.set_position(pos).map_err(|e| e.to_string())?;
        window.set_size(size).map_err(|e| e.to_string())?;
        window
    };

    window
        .set_ignore_cursor_events(true)
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    set_level(&window, OVERLAY_LEVEL);
    Ok(())
}

pub fn close(app: &AppHandle) {
    DRAWING.store(false, Ordering::SeqCst);
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.destroy();
    }
}

/// The draw shortcut and the pencil on the bar.
pub fn toggle(app: &AppHandle) {
    set(app, !DRAWING.load(Ordering::SeqCst));
}

fn set(app: &AppHandle, on: bool) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };
    let was = DRAWING.swap(on, Ordering::SeqCst);
    if was == on {
        return;
    }
    if let Err(err) = window.set_ignore_cursor_events(!on) {
        log::warn!("draw overlay: {err}");
    }
    if on {
        // Keys (Esc, undo, tools) go to the overlay; give the focus back afterwards.
        clipboard::remember_foreground();
        let _ = window.set_focus();
        #[cfg(target_os = "windows")]
        if let Some(bar) = app.get_webview_window(BAR_LABEL) {
            super::windows::bring_to_top(&bar);
        }
    } else if window.is_focused().unwrap_or(false)
        || app
            .get_webview_window(BAR_LABEL)
            .is_some_and(|bar| bar.is_focused().unwrap_or(false))
    {
        let _ = app.run_on_main_thread(clipboard::activate_previous);
    }
    let _ = app.emit_to(LABEL, "draw-mode", Mode { on });
    let _ = app.emit_to(BAR_LABEL, "draw-mode", Mode { on });
}

/// Draw mode on or off: Esc on the overlay, the pencil on the bar (`on: None` toggles).
#[tauri::command]
pub fn set_drawing(app: AppHandle, on: Option<bool>) {
    match on {
        Some(on) => set(&app, on),
        None => toggle(&app),
    }
}

/// Whether draw mode is on, for a page that loads after it changed.
#[tauri::command]
pub fn drawing(app: AppHandle) -> bool {
    app.get_webview_window(LABEL).is_some() && DRAWING.load(Ordering::SeqCst)
}

/// Resizes the control bar to `width` logical pixels, keeping its center and bottom
/// (the drawing tools appear on it in draw mode).
#[tauri::command]
pub fn fit_bar(window: WebviewWindow, width: f64) -> Result<(), String> {
    use tauri::{PhysicalPosition, PhysicalSize};
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let new_width = (width.clamp(120.0, 1200.0) * scale).round() as u32;
    if new_width == size.width {
        return Ok(());
    }
    let x = pos.x + (size.width as i32 - new_width as i32) / 2;
    let new_size = PhysicalSize::new(new_width, size.height);
    window.set_size(new_size).map_err(|e| e.to_string())?;
    window
        .set_position(PhysicalPosition::new(x, pos.y))
        .map_err(|e| e.to_string())?;
    #[cfg(not(target_os = "macos"))]
    window.set_size(new_size).map_err(|e| e.to_string())?;
    Ok(())
}

/// Above the menu bar (24) and the Dock (20), so a full-screen recording can be drawn on
/// everywhere; below context menus (101).
#[cfg(target_os = "macos")]
const OVERLAY_LEVEL: isize = 25;
/// The control bar stays above the overlay, so it can still be clicked in draw mode.
#[cfg(target_os = "macos")]
pub const BAR_LEVEL: isize = 26;

#[cfg(target_os = "macos")]
pub fn set_level(window: &WebviewWindow, level: isize) {
    let window = window.clone();
    let _ = window.clone().run_on_main_thread(move || {
        if let Ok(ns_window) = window.ns_window() {
            unsafe {
                let ns_window = &*(ns_window as *mut objc2::runtime::AnyObject);
                let _: () = objc2::msg_send![ns_window, setLevel: level];
            }
        }
    });
}
