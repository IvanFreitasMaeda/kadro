//! macOS Screen Recording permission.
//!
//! Without it, macOS hands back black (or wallpaper-only) screenshots instead of an
//! error. So we check before capturing and, when it is missing, send the user to the
//! right settings pane instead of showing a useless black overlay.

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Returns true when Kadro may capture the screen. Otherwise asks for the permission
/// and opens System Settings; macOS applies a new grant only after Kadro restarts.
pub fn ensure_screen_capture(app: &AppHandle) -> bool {
    if unsafe { CGPreflightScreenCaptureAccess() } {
        return true;
    }
    // Shows the system prompt the first time and adds Kadro to the settings list.
    unsafe { CGRequestScreenCaptureAccess() };
    if let Err(err) = app.opener().open_url(SETTINGS_URL, None::<&str>) {
        log::error!("could not open System Settings: {err}");
    }
    false
}
