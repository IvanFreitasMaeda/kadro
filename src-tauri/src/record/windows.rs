//! Windows backend: Windows Graphics Capture grabs the monitor and Media Foundation
//! encodes H.264 (both through the `windows-capture` crate). The control bar is
//! hidden from the capture with `WDA_EXCLUDEFROMCAPTURE`.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use image::{imageops, RgbaImage};
use windows::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST},
    UI::WindowsAndMessaging::{SetWindowDisplayAffinity, WDA_EXCLUDEFROMCAPTURE},
};
use windows_capture::{
    capture::{CaptureControl, Context, GraphicsCaptureApiHandler},
    encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    },
    frame::Frame,
    graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl},
    monitor::Monitor,
    settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    },
};

use super::{Backend, Feed, Format, Target};

/// Keeps a window (the recording bar) out of screen captures and recordings.
pub fn exclude_from_capture(window: &tauri::WebviewWindow) {
    if let Ok(hwnd) = window.hwnd() {
        let hwnd = HWND(hwnd.0 as *mut _);
        if let Err(err) = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) } {
            log::warn!("SetWindowDisplayAffinity: {err}");
        }
    }
}

struct Flags {
    /// Area to record in monitor pixels: x0, y0, x1, y1 (end exclusive).
    crop: (u32, u32, u32, u32),
    size: (u32, u32),
    fps: u32,
    format: Format,
    path: std::path::PathBuf,
    feed: Arc<Feed>,
}

struct Handler {
    crop: (u32, u32, u32, u32),
    size: (u32, u32),
    feed: Arc<Feed>,
    encoder: Option<VideoEncoder>,
    /// Timestamp (100 ns units) of the first frame.
    first: Option<i64>,
    /// The last frame sent to the encoder and its timestamp, to repeat it at the end:
    /// Windows sends no frames while the screen is still.
    last: Option<(Vec<u8>, i64, Instant)>,
    scratch: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Handler {
    type Flags = Flags;
    type Error = String;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let f = ctx.flags;
        let encoder = match f.format {
            Format::Mp4 => {
                let (w, h) = f.size;
                let bitrate = ((w as f64 * h as f64 * f.fps as f64 * 0.12) as u32)
                    .clamp(2_000_000, 40_000_000);
                Some(
                    VideoEncoder::new(
                        VideoSettingsBuilder::new(w, h)
                            .sub_type(VideoSettingsSubType::H264)
                            .bitrate(bitrate)
                            .frame_rate(f.fps),
                        AudioSettingsBuilder::default().disabled(true),
                        ContainerSettingsBuilder::default(),
                        &f.path,
                    )
                    .map_err(|e| e.to_string())?,
                )
            }
            Format::Gif => None,
        };
        Ok(Self {
            crop: f.crop,
            size: f.size,
            feed: f.feed,
            encoder,
            first: None,
            last: None,
            scratch: Vec::new(),
        })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let ts = frame.timestamp().map_err(|e| e.to_string())?.Duration;
        let first = *self.first.get_or_insert(ts);
        let time = Duration::from_nanos((ts - first).max(0) as u64 * 100);

        if self.encoder.is_none() && !self.feed.wants_pixels() {
            self.feed.tick(time);
            return Ok(());
        }

        // The monitor may have changed resolution: stay inside the frame.
        let (fw, fh) = (frame.width(), frame.height());
        let (x0, y0, x1, y1) = self.crop;
        let (x0, y0) = (x0.min(fw.saturating_sub(2)), y0.min(fh.saturating_sub(2)));
        let (x1, y1) = (x1.clamp(x0 + 2, fw), y1.clamp(y0 + 2, fh));
        let (w, h) = (x1 - x0, y1 - y0);

        let buffer = frame
            .buffer_crop(x0, y0, x1, y1)
            .map_err(|e| e.to_string())?;
        let data = buffer.as_nopadding_buffer(&mut self.scratch);
        self.feed.push_bgra(w, h, w as usize * 4, data, time);

        if let Some(encoder) = self.encoder.as_mut() {
            let pixels = fit(data, w, h, self.size);
            let flipped = bottom_up(&pixels, self.size.0, self.size.1);
            encoder
                .send_frame_buffer(&flipped, ts)
                .map_err(|e| e.to_string())?;
            self.last = Some((flipped, ts, Instant::now()));
        }
        Ok(())
    }
}

/// Scales BGRA pixels to `size` if needed (only for selections bigger than 4K).
fn fit(data: &[u8], w: u32, h: u32, size: (u32, u32)) -> std::borrow::Cow<'_, [u8]> {
    if (w, h) == size {
        return std::borrow::Cow::Borrowed(data);
    }
    // The channel order doesn't matter for scaling.
    match RgbaImage::from_raw(w, h, data.to_vec()) {
        Some(img) => std::borrow::Cow::Owned(
            imageops::resize(&img, size.0, size.1, imageops::FilterType::Triangle).into_raw(),
        ),
        None => std::borrow::Cow::Owned(vec![0; size.0 as usize * size.1 as usize * 4]),
    }
}

/// Media Foundation takes raw frames bottom row first.
fn bottom_up(data: &[u8], w: u32, h: u32) -> Vec<u8> {
    let row = w as usize * 4;
    let mut out = Vec::with_capacity(row * h as usize);
    for y in (0..h as usize).rev() {
        out.extend_from_slice(&data[y * row..(y + 1) * row]);
    }
    out
}

struct WinBackend {
    control: CaptureControl<Handler, String>,
}

impl Backend for WinBackend {
    fn stop(self: Box<Self>) -> Result<(), String> {
        let handler = self.control.callback();
        let stopped = self.control.stop().map_err(|e| e.to_string());
        let mut handler = handler.lock();
        if let Some(mut encoder) = handler.encoder.take() {
            // Hold the last frame until now, so the video lasts as long as the recording.
            if let Some((pixels, ts, at)) = handler.last.take() {
                let now = ts + (at.elapsed().as_nanos() / 100) as i64;
                if now > ts + 100_000 {
                    let _ = encoder.send_frame_buffer(&pixels, now);
                }
            }
            if handler.first.is_none() {
                drop(encoder);
                return Err("Nenhum quadro foi gravado.".into());
            }
            encoder.finish().map_err(|e| e.to_string())?;
        }
        stopped
    }
}

pub fn start(target: &Target, feed: Arc<Feed>) -> Result<Box<dyn Backend>, String> {
    let m = &target.monitor;
    let center = POINT {
        x: m.x + m.width as i32 / 2,
        y: m.y + m.height as i32 / 2,
    };
    let hmonitor = unsafe { MonitorFromPoint(center, MONITOR_DEFAULTTONEAREST) };
    let monitor = Monitor::from_raw_hmonitor(hmonitor.0);

    let r = &target.rect;
    // Video encoders need even sizes; the GIF is scaled later anyway.
    let (w, h) = match target.format {
        Format::Mp4 if target.size == (r.width & !1, r.height & !1) => target.size,
        _ => (r.width, r.height),
    };
    let flags = Flags {
        crop: (r.x, r.y, r.x + w, r.y + h),
        size: target.size,
        fps: target.fps,
        format: target.format,
        path: target.path.clone(),
        feed,
    };

    let supported = |check: Result<bool, _>| check.unwrap_or(false);
    let cursor = if supported(GraphicsCaptureApi::is_cursor_settings_supported()) {
        CursorCaptureSettings::WithCursor
    } else {
        CursorCaptureSettings::Default
    };
    let border = if supported(GraphicsCaptureApi::is_border_settings_supported()) {
        DrawBorderSettings::WithoutBorder
    } else {
        DrawBorderSettings::Default
    };
    let interval = if supported(GraphicsCaptureApi::is_minimum_update_interval_supported()) {
        MinimumUpdateIntervalSettings::Custom(Duration::from_secs_f64(1.0 / target.fps as f64))
    } else {
        MinimumUpdateIntervalSettings::Default
    };
    let settings = Settings::new(
        monitor,
        cursor,
        border,
        SecondaryWindowSettings::Default,
        interval,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    let control = Handler::start_free_threaded(settings).map_err(|e| e.to_string())?;
    Ok(Box::new(WinBackend { control }))
}
