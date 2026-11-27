//! Linux specifics.
//!
//! Kadro runs on X11 and on Wayland. On X11 everything works as on the other systems.
//! Wayland doesn't let an app read the screen, place its windows or grab keys, so
//! there Kadro:
//! - captures through the XDG desktop portals (`screenshot.rs`, and the screen cast
//!   portal with PipeWire for recordings, see `record/linux.rs`);
//! - registers its shortcuts with the global shortcuts portal (`shortcuts.rs`), or the
//!   user binds `kadro --area` and friends in the system settings;
//! - runs its own windows through XWayland (`GDK_BACKEND=x11`), so the overlays,
//!   the gallery and the recording bar can still be placed where they belong and the
//!   clipboard history can watch the clipboard. `KADRO_WAYLAND=1` turns that off.

pub mod screenshot;
pub mod shortcuts;
pub mod x11;

use std::sync::OnceLock;

/// Whether the desktop session is Wayland (even when Kadro's windows use XWayland).
pub fn is_wayland() -> bool {
    static WAYLAND: OnceLock<bool> = OnceLock::new();
    *WAYLAND.get_or_init(|| {
        std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
            || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v.eq_ignore_ascii_case("wayland"))
    })
}

/// Sets up the environment before GTK starts. Must run before any other thread.
pub fn prepare_env() {
    let unset = |name: &str| std::env::var_os(name).is_none_or(|v| v.is_empty());
    if is_wayland()
        && !unset("DISPLAY")
        && unset("GDK_BACKEND")
        && std::env::var("KADRO_WAYLAND").map_or(true, |v| v != "1")
    {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    // WebKitGTK's DMA-BUF renderer leaves windows blank with NVIDIA's driver.
    if std::path::Path::new("/proc/driver/nvidia").exists() && unset("WEBKIT_DISABLE_DMABUF_RENDERER")
    {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}
