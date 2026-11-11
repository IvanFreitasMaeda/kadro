//! macOS backend: ScreenCaptureKit captures the display, AVAssetWriter encodes H.264
//! with the hardware encoder. Kadro's own windows (the control bar, the menu bar
//! icon) are left out of the recording.

use std::{
    sync::{mpsc, Arc, Mutex},
    time::{Duration, Instant},
};

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, ProtocolObject},
    AllocAnyThread, DefinedClass,
};
use objc2_av_foundation::{
    AVAssetWriter, AVAssetWriterInput, AVAssetWriterStatus, AVFileTypeMPEG4, AVMediaTypeVideo,
    AVVideoAverageBitRateKey, AVVideoCodecKey, AVVideoCodecTypeH264,
    AVVideoCompressionPropertiesKey, AVVideoExpectedSourceFrameRateKey, AVVideoHeightKey,
    AVVideoMaxKeyFrameIntervalKey, AVVideoProfileLevelH264HighAutoLevel, AVVideoProfileLevelKey,
    AVVideoWidthKey,
};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    kCVPixelFormatType_32BGRA, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSError, NSNumber, NSObject, NSObjectProtocol, NSString, NSURL,
};
use objc2_screen_capture_kit::{
    SCContentFilter, SCFrameStatus, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamFrameInfoStatus, SCStreamOutput, SCStreamOutputType,
};

use super::{Backend, Feed, Format, Target};

const TIMEOUT: Duration = Duration::from_secs(10);

struct Writer {
    writer: Retained<AVAssetWriter>,
    input: Retained<AVAssetWriterInput>,
}

#[derive(Default)]
struct Progress {
    /// Presentation time of the first frame.
    first: Option<CMTime>,
    /// Presentation time of the last frame and when it arrived.
    last: Option<(CMTime, Instant)>,
    /// The writer refused a frame (logged once).
    append_failed: bool,
}

struct OutputVars {
    writer: Option<Writer>,
    feed: Arc<Feed>,
    progress: Mutex<Progress>,
}

define_class!(
    // SCStreamOutput that writes each frame to the file and/or hands it to the feed.
    #[unsafe(super(NSObject))]
    #[name = "KadroStreamOutput"]
    #[ivars = OutputVars]
    struct StreamOutput;

    unsafe impl NSObjectProtocol for StreamOutput {}

    unsafe impl SCStreamOutput for StreamOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_did_output(
            &self,
            _stream: &SCStream,
            sample: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind == SCStreamOutputType::Screen {
                self.on_frame(sample);
            }
        }
    }
);

/// Reads the frame status ScreenCaptureKit attaches to each sample.
fn frame_complete(sample: &CMSampleBuffer) -> bool {
    unsafe {
        let Some(attachments) = sample.sample_attachments_array(false) else {
            return true;
        };
        // CFArray and CFDictionary are toll-free bridged with NSArray/NSDictionary.
        let array = &*(&*attachments as *const _ as *const AnyObject);
        let count: usize = msg_send![array, count];
        if count == 0 {
            return true;
        }
        let info: *mut AnyObject = msg_send![array, objectAtIndex: 0usize];
        let Some(info) = info.as_ref() else {
            return true;
        };
        let status: *mut AnyObject = msg_send![info, objectForKey: SCStreamFrameInfoStatus];
        match status.as_ref() {
            Some(status) => {
                let value: isize = msg_send![status, integerValue];
                value == SCFrameStatus::Complete.0
            }
            None => true,
        }
    }
}

fn seconds(t: CMTime) -> f64 {
    if t.timescale == 0 {
        0.0
    } else {
        t.value as f64 / t.timescale as f64
    }
}

impl StreamOutput {
    fn new(vars: OutputVars) -> Retained<Self> {
        let this = Self::alloc().set_ivars(vars);
        unsafe { msg_send![super(this), init] }
    }

    fn on_frame(&self, sample: &CMSampleBuffer) {
        let vars = self.ivars();
        // Only complete frames carry new pixels; the others are status updates
        // (idle screen, etc.) and must not reach the writer.
        if !frame_complete(sample) {
            return;
        }
        let Some(image) = (unsafe { sample.image_buffer() }) else {
            return;
        };
        let pts = unsafe { sample.presentation_time_stamp() };

        let first = {
            let mut progress = vars.progress.lock().unwrap();
            let first = *progress.first.get_or_insert_with(|| {
                if let Some(w) = &vars.writer {
                    unsafe { w.writer.startSessionAtSourceTime(pts) };
                }
                pts
            });
            if let Some(w) = &vars.writer {
                let appended = unsafe {
                    w.input.isReadyForMoreMediaData() && w.input.appendSampleBuffer(sample)
                };
                if !appended && !progress.append_failed {
                    progress.append_failed = true;
                    log::warn!("AVAssetWriter dropped a frame: {:?}", unsafe {
                        w.writer.error()
                    });
                }
            }
            progress.last = Some((pts, Instant::now()));
            first
        };

        let time = Duration::from_secs_f64((seconds(pts) - seconds(first)).max(0.0));
        if !vars.feed.wants_pixels() {
            vars.feed.tick(time);
            return;
        }
        unsafe {
            CVPixelBufferLockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
            let base = CVPixelBufferGetBaseAddress(&image);
            if !base.is_null() {
                let stride = CVPixelBufferGetBytesPerRow(&image);
                let width = CVPixelBufferGetWidth(&image);
                let height = CVPixelBufferGetHeight(&image);
                let data = std::slice::from_raw_parts(base.cast::<u8>(), stride * height);
                vars.feed
                    .push_bgra(width as u32, height as u32, stride, data, time);
            }
            CVPixelBufferUnlockBaseAddress(&image, CVPixelBufferLockFlags::ReadOnly);
        }
    }
}

struct MacBackend {
    stream: Retained<SCStream>,
    output: Retained<StreamOutput>,
    queue: DispatchRetained<DispatchQueue>,
}

// The stream and writer are only touched from the capture queue and, after the
// stream has stopped, from the thread that finishes the file.
unsafe impl Send for MacBackend {}

fn describe(err: &NSError) -> String {
    err.localizedDescription().to_string()
}

/// Calls an API that reports completion with an optional NSError, and waits for it.
fn wait_for(call: impl FnOnce(&block2::DynBlock<dyn Fn(*mut NSError)>)) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<Option<String>>();
    let block = RcBlock::new(move |err: *mut NSError| {
        let _ = tx.send(unsafe { err.as_ref() }.map(describe));
    });
    call(&block);
    match rx.recv_timeout(TIMEOUT) {
        Ok(None) => Ok(()),
        Ok(Some(err)) => Err(err),
        Err(_) => Err("o macOS não respondeu a tempo".into()),
    }
}

fn shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (tx, rx) = mpsc::channel::<Result<Retained<SCShareableContent>, String>>();
    let block = RcBlock::new(move |content: *mut SCShareableContent, err: *mut NSError| {
        let result = match unsafe { Retained::retain(content) } {
            Some(content) => Ok(content),
            None => Err(unsafe { err.as_ref() }
                .map(describe)
                .unwrap_or_else(|| "sem acesso à tela".into())),
        };
        let _ = tx.send(result);
    });
    unsafe {
        SCShareableContent::getShareableContentExcludingDesktopWindows_onScreenWindowsOnly_completionHandler(
            false, true, &block,
        );
    }
    rx.recv_timeout(TIMEOUT)
        .map_err(|_| "o macOS não respondeu a tempo".to_string())?
}

fn number(n: u32) -> Retained<NSNumber> {
    NSNumber::new_u32(n)
}

fn writer(target: &Target) -> Result<Writer, String> {
    let (w, h) = target.size;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&target.path.to_string_lossy()));
    let missing = || "AVFoundation incompleto".to_string();
    unsafe {
        let writer = AVAssetWriter::assetWriterWithURL_fileType_error(
            &url,
            AVFileTypeMPEG4.ok_or_else(missing)?,
        )
        .map_err(|e| describe(&e))?;

        // Screen content compresses well; about 0.12 bits per pixel per frame is sharp.
        let bitrate =
            ((w as f64 * h as f64 * target.fps as f64 * 0.12) as u32).clamp(2_000_000, 40_000_000);
        let compression_keys = [
            AVVideoAverageBitRateKey.ok_or_else(missing)?,
            AVVideoExpectedSourceFrameRateKey.ok_or_else(missing)?,
            AVVideoMaxKeyFrameIntervalKey.ok_or_else(missing)?,
            AVVideoProfileLevelKey.ok_or_else(missing)?,
        ];
        let bitrate = number(bitrate);
        let fps = number(target.fps);
        let keyframes = number(target.fps * 2);
        let compression_values: [&AnyObject; 4] = [
            bitrate.as_ref(),
            fps.as_ref(),
            keyframes.as_ref(),
            AVVideoProfileLevelH264HighAutoLevel
                .ok_or_else(missing)?
                .as_ref(),
        ];
        let compression = NSDictionary::<NSString, AnyObject>::from_slices(
            &compression_keys,
            &compression_values,
        );

        let width = number(w);
        let height = number(h);
        let keys = [
            AVVideoCodecKey.ok_or_else(missing)?,
            AVVideoWidthKey.ok_or_else(missing)?,
            AVVideoHeightKey.ok_or_else(missing)?,
            AVVideoCompressionPropertiesKey.ok_or_else(missing)?,
        ];
        let values: [&AnyObject; 4] = [
            AVVideoCodecTypeH264.ok_or_else(missing)?.as_ref(),
            width.as_ref(),
            height.as_ref(),
            compression.as_ref(),
        ];
        let settings = NSDictionary::<NSString, AnyObject>::from_slices(&keys, &values);

        let input = AVAssetWriterInput::assetWriterInputWithMediaType_outputSettings(
            AVMediaTypeVideo.ok_or_else(missing)?,
            Some(&settings),
        );
        input.setExpectsMediaDataInRealTime(true);
        if !writer.canAddInput(&input) {
            return Err("o codificador de vídeo recusou as configurações".into());
        }
        writer.addInput(&input);
        if !writer.startWriting() {
            return Err(writer
                .error()
                .map(|e| describe(&e))
                .unwrap_or_else(|| "não foi possível criar o arquivo".into()));
        }
        Ok(Writer { writer, input })
    }
}

pub fn start(target: &Target, feed: Arc<Feed>) -> Result<Box<dyn Backend>, String> {
    let content = shareable_content()?;
    let pid = std::process::id() as libc_pid;
    unsafe {
        let display = content
            .displays()
            .iter()
            .find(|d| d.displayID() == target.monitor.id)
            .ok_or("monitor não encontrado")?;

        // Leave Kadro out: by app when macOS lists it, otherwise window by window.
        let apps: Vec<_> = content
            .applications()
            .iter()
            .filter(|a| a.processID() == pid)
            .collect();
        let filter = if apps.is_empty() {
            let windows: Vec<_> = content
                .windows()
                .iter()
                .filter(|w| w.owningApplication().is_some_and(|a| a.processID() == pid))
                .collect();
            SCContentFilter::initWithDisplay_excludingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::from_retained_slice(&windows),
            )
        } else {
            SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::from_retained_slice(&apps),
                &NSArray::new(),
            )
        };

        // The selection is in screenshot pixels; ScreenCaptureKit wants points.
        let s = target.monitor.px_per_unit.max(0.01);
        let r = &target.rect;
        let config = SCStreamConfiguration::new();
        config.setSourceRect(CGRect::new(
            CGPoint::new(r.x as f64 / s, r.y as f64 / s),
            CGSize::new(r.width as f64 / s, r.height as f64 / s),
        ));
        config.setWidth(target.size.0 as usize);
        config.setHeight(target.size.1 as usize);
        config.setMinimumFrameInterval(CMTime::new(1, target.fps as i32));
        config.setPixelFormat(kCVPixelFormatType_32BGRA);
        config.setShowsCursor(true);
        config.setQueueDepth(6);

        let writer = match target.format {
            Format::Mp4 => Some(writer(target)?),
            Format::Gif => None,
        };
        let output = StreamOutput::new(OutputVars {
            writer,
            feed,
            progress: Mutex::new(Progress::default()),
        });
        let stream = SCStream::initWithFilter_configuration_delegate(
            SCStream::alloc(),
            &filter,
            &config,
            None,
        );
        let queue = DispatchQueue::new(
            "io.github.ivanfreitasmaeda.kadro.record",
            DispatchQueueAttr::SERIAL,
        );
        stream
            .addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Screen,
                Some(&queue),
            )
            .map_err(|e| describe(&e))?;
        let backend = MacBackend {
            stream,
            output,
            queue,
        };
        if let Err(err) = wait_for(|block| {
            backend
                .stream
                .startCaptureWithCompletionHandler(Some(block))
        }) {
            backend.cancel();
            return Err(err);
        }
        Ok(Box::new(backend))
    }
}

#[allow(non_camel_case_types)]
type libc_pid = i32;

impl MacBackend {
    /// Waits until the capture queue has handled every frame it already received.
    fn drain(&self) {
        self.queue.exec_sync(|| {});
    }

    fn cancel(&self) {
        self.drain();
        if let Some(w) = &self.output.ivars().writer {
            unsafe { w.writer.cancelWriting() };
        }
    }
}

impl Backend for MacBackend {
    fn stop(self: Box<Self>) -> Result<(), String> {
        let stopped =
            wait_for(|block| unsafe { self.stream.stopCaptureWithCompletionHandler(Some(block)) });
        if let Err(err) = stopped {
            log::warn!("stopCapture: {err}");
        }
        self.drain();

        let vars = self.output.ivars();
        let Some(w) = &vars.writer else {
            return Ok(());
        };
        let last = vars.progress.lock().unwrap().last;
        let Some((last_pts, at)) = last else {
            unsafe { w.writer.cancelWriting() };
            return Err("Nenhum quadro foi gravado.".into());
        };
        unsafe {
            w.input.markAsFinished();
            // A still screen sends no frames: stretch the last one until now.
            let end = CMTime {
                value: last_pts.value
                    + (at.elapsed().as_secs_f64() * last_pts.timescale as f64) as i64,
                ..last_pts
            };
            w.writer.endSessionAtSourceTime(end);

            let (tx, rx) = mpsc::channel::<()>();
            let block = RcBlock::new(move || {
                let _ = tx.send(());
            });
            w.writer.finishWritingWithCompletionHandler(&block);
            if rx.recv_timeout(Duration::from_secs(30)).is_err() {
                return Err("o macOS não terminou de gravar o vídeo a tempo".into());
            }
            if w.writer.status() != AVAssetWriterStatus::Completed {
                return Err(w
                    .writer
                    .error()
                    .map(|e| describe(&e))
                    .unwrap_or_else(|| "o vídeo não foi finalizado".into()));
            }
        }
        Ok(())
    }
}
