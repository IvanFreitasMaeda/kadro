//! Screenshots on Wayland, through the XDG desktop portal (GNOME, KDE, wlroots and
//! Hyprland all provide it). One call returns the whole desktop; `capture.rs` cuts it
//! into one image per monitor.

use std::path::PathBuf;

use ashpd::desktop::screenshot::Screenshot;
use image::RgbaImage;

/// The whole desktop, in pixels, as the portal returns it.
pub fn desktop() -> Result<RgbaImage, String> {
    let uri = pollster::block_on(async {
        let response = Screenshot::request()
            .interactive(false)
            .modal(false)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .response()
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(response.uri().as_str().to_string())
    })?;
    let path = file_path(&uri).ok_or_else(|| format!("endereço inesperado: {uri}"))?;
    let image = image::open(&path).map(|i| i.into_rgba8());
    // GNOME saves the shot in Pictures; it was only for us.
    let _ = std::fs::remove_file(&path);
    image.map_err(|e| e.to_string())
}

fn file_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    // `file://host/path`: only the local host makes sense.
    let path = if rest.starts_with('/') {
        rest
    } else {
        &rest[rest.find('/')?..]
    };
    let decoded = percent_encoding::percent_decode_str(path).decode_utf8().ok()?;
    Some(PathBuf::from(decoded.into_owned()))
}

/// Cuts `desktop` into the monitors' areas. `monitors` are (x, y, width, height) in
/// the units the windows use (XWayland's, usually logical pixels); the portal image
/// may be bigger on a HiDPI screen, so the areas are scaled to it.
pub fn split(desktop: &RgbaImage, monitors: &[(i32, i32, u32, u32)]) -> Vec<RgbaImage> {
    let left = monitors.iter().map(|m| m.0).min().unwrap_or(0);
    let top = monitors.iter().map(|m| m.1).min().unwrap_or(0);
    let right = monitors
        .iter()
        .map(|m| m.0 + m.2 as i32)
        .max()
        .unwrap_or(desktop.width() as i32);
    let bottom = monitors
        .iter()
        .map(|m| m.1 + m.3 as i32)
        .max()
        .unwrap_or(desktop.height() as i32);
    let sx = desktop.width() as f64 / (right - left).max(1) as f64;
    let sy = desktop.height() as f64 / (bottom - top).max(1) as f64;
    monitors
        .iter()
        .map(|&(x, y, w, h)| {
            let x0 = (((x - left) as f64 * sx).round() as u32).min(desktop.width() - 1);
            let y0 = (((y - top) as f64 * sy).round() as u32).min(desktop.height() - 1);
            let w = ((w as f64 * sx).round() as u32).clamp(1, desktop.width() - x0);
            let h = ((h as f64 * sy).round() as u32).clamp(1, desktop.height() - y0);
            image::imageops::crop_imm(desktop, x0, y0, w, h).to_image()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_uris() {
        assert_eq!(
            file_path("file:///home/a/Imagens/Captura%20de%20tela.png"),
            Some(PathBuf::from("/home/a/Imagens/Captura de tela.png"))
        );
        assert_eq!(
            file_path("file://localhost/tmp/x.png"),
            Some(PathBuf::from("/tmp/x.png"))
        );
        assert_eq!(file_path("https://example.com/x.png"), None);
    }

    #[test]
    fn splits_a_hidpi_desktop() {
        // Two 1920x1080 monitors side by side, captured at 2x.
        let desktop = RgbaImage::new(7680, 2160);
        let parts = split(&desktop, &[(0, 0, 1920, 1080), (1920, 0, 1920, 1080)]);
        assert_eq!(parts[0].dimensions(), (3840, 2160));
        assert_eq!(parts[1].dimensions(), (3840, 2160));
    }
}
