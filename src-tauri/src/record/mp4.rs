//! MP4 encoding for Linux, where the OS has no video encoder an app can count on:
//! OpenH264 (Cisco's, built in) encodes H.264 tuned for screen content, and a small
//! muxer writes the MP4. Runs on its own thread; frames that arrive while it is busy
//! are dropped, and the previous frame simply lasts longer.

use std::{
    fs::File,
    io::BufWriter,
    path::{Path, PathBuf},
    sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError},
    thread::JoinHandle,
    time::Duration,
};

use image::{imageops, ImageBuffer, Rgba};
use mp4::{AvcConfig, MediaConfig, Mp4Config, Mp4Sample, Mp4Writer, TrackConfig, TrackType};
use openh264::{
    encoder::{
        BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod,
        RateControlMode, UsageType,
    },
    formats::{BgraSliceU8, YUVBuffer},
    OpenH264API, Timestamp,
};

/// The track's clock: the usual 90 kHz for video.
const TIMESCALE: u32 = 90_000;

pub enum Msg {
    /// Tightly packed BGRA pixels, captured this long after the first frame.
    Frame {
        width: u32,
        height: u32,
        bgra: Vec<u8>,
        time: Duration,
    },
    /// No more frames; the recording lasted this long.
    End(Duration),
}

pub struct Mp4Encoder {
    tx: SyncSender<Msg>,
    handle: JoinHandle<Result<(), String>>,
}

impl Mp4Encoder {
    pub fn spawn(path: &Path, size: (u32, u32), fps: u32) -> Result<Self, String> {
        // Check the file can be written before recording anything.
        File::create(path).map_err(|e| format!("não foi possível criar o arquivo: {e}"))?;
        let (tx, rx) = sync_channel(3);
        let path = path.to_path_buf();
        let handle = std::thread::Builder::new()
            .name("mp4-encoder".into())
            .spawn(move || run(rx, path, size, fps))
            .map_err(|e| e.to_string())?;
        Ok(Self { tx, handle })
    }

    pub fn sender(&self) -> SyncSender<Msg> {
        self.tx.clone()
    }

    /// Sends a frame unless the encoder is behind.
    pub fn offer(tx: &SyncSender<Msg>, msg: Msg) {
        if let Err(TrySendError::Disconnected(_)) = tx.try_send(msg) {
            log::warn!("mp4 encoder is gone");
        }
    }

    /// Finishes the file.
    pub fn finish(self, duration: Duration) -> Result<(), String> {
        let _ = self.tx.send(Msg::End(duration));
        drop(self.tx);
        self.handle
            .join()
            .map_err(|_| "o codificador de vídeo parou".to_string())?
    }
}

fn ticks(time: Duration) -> u64 {
    (time.as_micros() * TIMESCALE as u128 / 1_000_000) as u64
}

/// An encoded frame waiting for the next one, which tells how long it lasts.
struct Pending {
    start: u64,
    sync: bool,
    bytes: Vec<u8>,
}

struct Muxer {
    path: PathBuf,
    size: (u32, u32),
    writer: Option<Mp4Writer<BufWriter<File>>>,
    pending: Option<Pending>,
    samples: u32,
}

impl Muxer {
    fn push(&mut self, sps: &[u8], pps: &[u8], sample: Pending) -> Result<(), String> {
        if self.writer.is_none() {
            let file = File::create(&self.path).map_err(|e| e.to_string())?;
            let config = Mp4Config {
                major_brand: "isom".parse().unwrap(),
                minor_version: 512,
                compatible_brands: ["isom", "iso2", "avc1", "mp41"]
                    .iter()
                    .map(|b| b.parse().unwrap())
                    .collect(),
                timescale: 1000,
            };
            let mut writer =
                Mp4Writer::write_start(BufWriter::new(file), &config).map_err(|e| e.to_string())?;
            writer
                .add_track(&TrackConfig {
                    track_type: TrackType::Video,
                    timescale: TIMESCALE,
                    language: "und".into(),
                    media_conf: MediaConfig::AvcConfig(AvcConfig {
                        width: self.size.0 as u16,
                        height: self.size.1 as u16,
                        seq_param_set: sps.to_vec(),
                        pic_param_set: pps.to_vec(),
                    }),
                })
                .map_err(|e| e.to_string())?;
            self.writer = Some(writer);
        }
        if let Some(previous) = self.pending.take() {
            let duration = sample.start.saturating_sub(previous.start).max(1);
            self.write(previous, duration)?;
        }
        self.pending = Some(sample);
        Ok(())
    }

    fn write(&mut self, sample: Pending, duration: u64) -> Result<(), String> {
        let writer = self.writer.as_mut().ok_or("mp4")?;
        writer
            .write_sample(
                1,
                &Mp4Sample {
                    start_time: sample.start,
                    duration: duration.min(u32::MAX as u64) as u32,
                    rendering_offset: 0,
                    is_sync: sample.sync,
                    bytes: sample.bytes.into(),
                },
            )
            .map_err(|e| e.to_string())?;
        self.samples += 1;
        Ok(())
    }

    fn end(&mut self, end: u64, frame: u64) -> Result<u32, String> {
        if let Some(last) = self.pending.take() {
            let duration = end.saturating_sub(last.start).max(frame);
            self.write(last, duration)?;
        }
        if let Some(writer) = self.writer.as_mut() {
            writer.write_end().map_err(|e| e.to_string())?;
        }
        Ok(self.samples)
    }
}

/// Splits Annex B output into NAL units (without their start codes).
fn nal_units(data: &[u8]) -> Vec<&[u8]> {
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut units = Vec::with_capacity(starts.len());
    for (n, &start) in starts.iter().enumerate() {
        let mut end = starts.get(n + 1).map_or(data.len(), |&next| next - 3);
        // A 4-byte start code leaves one more zero before the next unit.
        while end > start && data[end - 1] == 0 && n + 1 < starts.len() {
            end -= 1;
        }
        if end > start {
            units.push(&data[start..end]);
        }
    }
    units
}

fn run(rx: Receiver<Msg>, path: PathBuf, size: (u32, u32), fps: u32) -> Result<(), String> {
    let (w, h) = size;
    // Screen recordings are mostly still with sharp edges: aim high on quality, the
    // still parts cost almost nothing.
    let bitrate = (w as u64 * h as u64 * fps as u64 / 6).clamp(1_000_000, 24_000_000) as u32;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 4))
        .unwrap_or(2) as u16;
    let config = EncoderConfig::new()
        .usage_type(UsageType::ScreenContentRealTime)
        .rate_control_mode(RateControlMode::Quality)
        .bitrate(BitRate::from_bps(bitrate))
        .max_frame_rate(FrameRate::from_hz(fps as f32))
        .complexity(Complexity::Low)
        .intra_frame_period(IntraFramePeriod::from_num_frames(fps * 4))
        .skip_frames(false)
        .num_threads(threads);
    let mut encoder =
        Encoder::with_api_config(OpenH264API::from_source(), config).map_err(|e| e.to_string())?;

    let mut muxer = Muxer {
        path: path.clone(),
        size,
        writer: None,
        pending: None,
        samples: 0,
    };
    let mut sps = Vec::new();
    let mut pps = Vec::new();
    let mut yuv: Option<YUVBuffer> = None;
    let mut end = None;
    let mut last_time = Duration::ZERO;

    for msg in rx {
        let (width, height, bgra, time) = match msg {
            Msg::Frame {
                width,
                height,
                bgra,
                time,
            } => (width, height, bgra, time),
            Msg::End(duration) => {
                end = Some(duration);
                break;
            }
        };
        if time < last_time && muxer.pending.is_some() {
            continue;
        }
        last_time = time;
        let bgra = fit(bgra, width, height, size);
        let yuv = yuv.get_or_insert_with(|| YUVBuffer::new(w as usize, h as usize));
        yuv.read_bgra8(BgraSliceU8::new(&bgra, (w as usize, h as usize)));
        let stream = encoder
            .encode_at(&*yuv, Timestamp::from_millis(time.as_millis() as u64))
            .map_err(|e| e.to_string())?;
        let frame_type = stream.frame_type();
        if matches!(frame_type, FrameType::Skip | FrameType::Invalid) {
            continue;
        }
        let annex_b = stream.to_vec();
        let mut sample = Vec::with_capacity(annex_b.len() + 16);
        for nal in nal_units(&annex_b) {
            match nal[0] & 0x1f {
                7 => sps = nal.to_vec(),
                8 => pps = nal.to_vec(),
                _ => {
                    sample.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                    sample.extend_from_slice(nal);
                }
            }
        }
        if sample.is_empty() || sps.is_empty() || pps.is_empty() {
            continue;
        }
        muxer.push(
            &sps,
            &pps,
            Pending {
                start: ticks(time),
                sync: matches!(frame_type, FrameType::IDR | FrameType::I),
                bytes: sample,
            },
        )?;
    }

    let frame = (TIMESCALE / fps.max(1)) as u64;
    let end = end.map(ticks).unwrap_or(0);
    let samples = muxer.end(end, frame)?;
    if samples == 0 {
        let _ = std::fs::remove_file(&path);
        return Err("Nenhum quadro foi gravado.".into());
    }
    Ok(())
}

/// Makes a frame exactly `size`: trims the odd pixel off (encoders need even sizes)
/// or scales down a frame bigger than the output.
fn fit(bgra: Vec<u8>, width: u32, height: u32, size: (u32, u32)) -> Vec<u8> {
    let (w, h) = size;
    if (width, height) == size {
        return bgra;
    }
    if width >= w && height >= h && width - w <= 1 && height - h <= 1 {
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for row in bgra.chunks_exact(width as usize * 4).take(h as usize) {
            out.extend_from_slice(&row[..w as usize * 4]);
        }
        return out;
    }
    let Some(img) = ImageBuffer::<Rgba<u8>, _>::from_raw(width, height, bgra) else {
        return vec![0; (w * h * 4) as usize];
    };
    imageops::resize(&img, w, h, imageops::FilterType::Triangle).into_raw()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_annex_b() {
        let data = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 4, 5];
        let units = nal_units(&data);
        assert_eq!(units, vec![&[0x67, 1, 2][..], &[0x68, 3][..], &[0x65, 4, 5][..]]);
    }

    #[test]
    fn trims_odd_pixels() {
        let bgra = vec![7u8; 3 * 3 * 4];
        assert_eq!(fit(bgra, 3, 3, (2, 2)).len(), 2 * 2 * 4);
    }

    #[test]
    fn encodes_a_short_video() {
        let dir = std::env::temp_dir().join(format!("kadro-mp4-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.mp4");
        let encoder = Mp4Encoder::spawn(&path, (320, 240), 30).unwrap();
        let tx = encoder.sender();
        for i in 0..30u32 {
            let mut bgra = vec![0u8; 320 * 240 * 4];
            for (n, px) in bgra.chunks_exact_mut(4).enumerate() {
                let x = (n as u32 % 320 + i * 4) % 320;
                px.copy_from_slice(&[x as u8, 80, 200, 255]);
            }
            tx.send(Msg::Frame {
                width: 320,
                height: 240,
                bgra,
                time: Duration::from_millis(i as u64 * 33),
            })
            .unwrap();
        }
        drop(tx);
        encoder.finish(Duration::from_millis(1000)).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let reader =
            mp4::Mp4Reader::read_header(std::io::Cursor::new(&bytes), bytes.len() as u64).unwrap();
        let track = reader.tracks().values().next().unwrap();
        assert_eq!(track.width(), 320);
        assert_eq!(track.sample_count(), 30);
        assert!(reader.duration() >= Duration::from_millis(990));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
