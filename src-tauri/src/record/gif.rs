//! Animated GIF encoder for recordings, fed frame by frame while recording.
//!
//! Screen recordings change little from one frame to the next, so each GIF frame only
//! holds the rectangle that changed, with the unchanged pixels inside it transparent.
//! That keeps files small and avoids colour flicker in still areas.

use std::{
    fs::File,
    io::BufWriter,
    path::Path,
    sync::mpsc::{Receiver, SyncSender},
    thread::JoinHandle,
    time::Duration,
};

use image::{imageops, RgbaImage};

/// NeuQuant speed (1 = best, 30 = fastest). 10 is the `gif` crate's default.
const SPEED: i32 = 10;

pub enum Msg {
    /// An RGBA frame and when it was captured, counted from the first frame.
    Frame(RgbaImage, Duration),
    /// Recording stopped at this time: write the last frame and close the file.
    End(Duration),
}

/// The encoder's input and its thread, which returns how many frames it wrote.
pub type Encoder = (SyncSender<Msg>, JoinHandle<Result<u32, String>>);

/// Starts the encoder thread. Frames sent to the returned channel are scaled to `size`.
pub fn spawn(path: &Path, size: (u32, u32)) -> Result<Encoder, String> {
    let file = File::create(path).map_err(|e| format!("não foi possível criar o GIF: {e}"))?;
    // A small queue: if encoding falls behind, the capture side drops frames instead
    // of piling them up in memory.
    let (tx, rx) = std::sync::mpsc::sync_channel(3);
    let handle = std::thread::spawn(move || run(rx, BufWriter::new(file), size));
    Ok((tx, handle))
}

fn run(
    rx: Receiver<Msg>,
    out: BufWriter<File>,
    (width, height): (u32, u32),
) -> Result<u32, String> {
    let err = |e: gif::EncodingError| format!("erro ao gravar o GIF: {e}");
    let mut encoder = gif::Encoder::new(out, width as u16, height as u16, &[]).map_err(err)?;
    encoder.set_repeat(gif::Repeat::Infinite).map_err(err)?;

    let mut previous: Option<RgbaImage> = None;
    // The last frame is written once the next one arrives, since that sets its delay.
    let mut pending: Option<(gif::Frame<'static>, Duration)> = None;
    let mut written = 0;
    let mut end = None;

    for msg in rx {
        let (img, time) = match msg {
            Msg::Frame(img, time) => (img, time),
            Msg::End(time) => {
                end = Some(time);
                break;
            }
        };
        let img = if img.dimensions() == (width, height) {
            img
        } else {
            imageops::resize(&img, width, height, imageops::FilterType::Triangle)
        };
        let Some(frame) = delta_frame(previous.as_ref(), &img) else {
            continue; // nothing changed: the previous frame just lasts longer
        };
        if let Some((mut last, start)) = pending.take() {
            last.delay = delay(start, time);
            encoder.write_frame(&last).map_err(err)?;
            written += 1;
        }
        pending = Some((frame, time));
        previous = Some(img);
    }

    if let Some((mut last, start)) = pending {
        last.delay = delay(start, end.unwrap_or(start)).max(10);
        encoder.write_frame(&last).map_err(err)?;
        written += 1;
    }
    encoder.into_inner().map_err(err)?;
    Ok(written)
}

/// Frame delay in hundredths of a second. Rounding both ends keeps the total in sync
/// with the clock instead of drifting frame after frame. Browsers slow down delays
/// under 2, so that is the minimum.
fn delay(start: Duration, end: Duration) -> u16 {
    let cs = |d: Duration| (d.as_millis() as u64 + 5) / 10;
    cs(end).saturating_sub(cs(start)).clamp(2, u16::MAX as u64) as u16
}

/// The part of `img` that differs from `previous`, as a GIF frame placed over it.
/// Returns `None` when nothing changed.
fn delta_frame(previous: Option<&RgbaImage>, img: &RgbaImage) -> Option<gif::Frame<'static>> {
    let (w, h) = img.dimensions();
    let Some(prev) = previous else {
        let mut pixels = img.as_raw().clone();
        opaque(&mut pixels);
        return Some(frame_at(0, 0, w, h, &mut pixels));
    };
    let (x0, y0, x1, y1) = changed_box(prev, img)?;
    let mut pixels = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
    for y in y0..y1 {
        for x in x0..x1 {
            let p = img.get_pixel(x, y).0;
            if prev.get_pixel(x, y).0 == p {
                // Unchanged: transparent, so the previous frame shows through.
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                pixels.extend_from_slice(&[p[0], p[1], p[2], 255]);
            }
        }
    }
    Some(frame_at(x0, y0, x1 - x0, y1 - y0, &mut pixels))
}

fn opaque(pixels: &mut [u8]) {
    for p in pixels.chunks_exact_mut(4) {
        p[3] = 255;
    }
}

fn frame_at(x: u32, y: u32, w: u32, h: u32, pixels: &mut [u8]) -> gif::Frame<'static> {
    let mut frame = gif::Frame::from_rgba_speed(w as u16, h as u16, pixels, SPEED);
    frame.left = x as u16;
    frame.top = y as u16;
    frame.dispose = gif::DisposalMethod::Keep;
    frame
}

/// Bounding box `(x0, y0, x1, y1)` (end exclusive) of the pixels that differ.
fn changed_box(a: &RgbaImage, b: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let w = a.width() as usize;
    let rows_a = a.as_raw().chunks_exact(w * 4);
    let rows_b = b.as_raw().chunks_exact(w * 4);
    let mut found: Option<(usize, usize, usize, usize)> = None;
    for (y, (ra, rb)) in rows_a.zip(rows_b).enumerate() {
        if ra == rb {
            continue;
        }
        let first = (0..w).find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4])?;
        let last = (0..w)
            .rev()
            .find(|&x| ra[x * 4..x * 4 + 4] != rb[x * 4..x * 4 + 4])?;
        found = Some(match found {
            None => (first, y, last + 1, y + 1),
            Some((x0, y0, x1, _)) => (x0.min(first), y0, x1.max(last + 1), y + 1),
        });
    }
    found.map(|(x0, y0, x1, y1)| (x0 as u32, y0 as u32, x1 as u32, y1 as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(c))
    }

    #[test]
    fn finds_the_changed_rectangle() {
        let a = solid(10, 8, [0, 0, 0, 255]);
        let mut b = a.clone();
        assert_eq!(changed_box(&a, &b), None);
        b.put_pixel(2, 3, image::Rgba([255, 0, 0, 255]));
        b.put_pixel(6, 5, image::Rgba([0, 255, 0, 255]));
        assert_eq!(changed_box(&a, &b), Some((2, 3, 7, 6)));
    }

    #[test]
    fn delays_follow_the_clock() {
        let ms = Duration::from_millis;
        assert_eq!(delay(ms(0), ms(66)), 7);
        assert_eq!(delay(ms(66), ms(133)), 6);
        assert_eq!(delay(ms(0), ms(5)), 2);
    }

    #[test]
    fn writes_a_playable_gif() {
        let path = std::env::temp_dir().join(format!("kadro-gif-{}.gif", std::process::id()));
        let (tx, handle) = spawn(&path, (20, 10)).unwrap();
        let ms = Duration::from_millis;
        tx.send(Msg::Frame(solid(40, 20, [255, 255, 255, 255]), ms(0)))
            .unwrap();
        tx.send(Msg::Frame(solid(40, 20, [255, 255, 255, 255]), ms(66)))
            .unwrap();
        let mut moved = solid(40, 20, [255, 255, 255, 255]);
        moved.put_pixel(10, 10, image::Rgba([255, 0, 0, 255]));
        tx.send(Msg::Frame(moved, ms(133))).unwrap();
        tx.send(Msg::End(ms(1000))).unwrap();
        assert_eq!(handle.join().unwrap().unwrap(), 2);

        let mut decoder = gif::DecodeOptions::new()
            .read_info(File::open(&path).unwrap())
            .unwrap();
        assert_eq!((decoder.width(), decoder.height()), (20, 10));
        let first = decoder.read_next_frame().unwrap().unwrap();
        assert_eq!((first.width, first.height, first.delay), (20, 10, 13));
        let second = decoder.read_next_frame().unwrap().unwrap();
        assert!(second.width < 20 && second.delay == 87);
        assert!(decoder.read_next_frame().unwrap().is_none());
        std::fs::remove_file(&path).unwrap();
    }
}
