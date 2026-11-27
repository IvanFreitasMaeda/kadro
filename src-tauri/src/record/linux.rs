//! Linux backend.
//!
//! - X11: the selected area is read straight from the screen with the MIT-SHM
//!   extension (no copy through the X socket), `fps` times a second, with the mouse
//!   pointer drawn in from XFixes.
//! - Wayland: apps can't read the screen, so the desktop's screen cast portal hands
//!   over a PipeWire stream of the monitor (the desktop asks the user which screen to
//!   share the first time, and remembers it). Each frame is cropped to the area.
//!
//! Frames go to the GIF encoder through [`Feed`], or to the MP4 encoder in `mp4.rs`.

use std::{
    os::fd::OwnedFd,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::SyncSender,
        Arc, Mutex,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use tauri::{AppHandle, Manager};

use super::{
    mp4::{Mp4Encoder, Msg},
    Backend, Feed, Format, Target,
};
use crate::config;

/// A rectangle in a source's pixels.
#[derive(Debug, Clone, Copy)]
struct Area {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

/// Where cropped frames go.
struct Sink {
    feed: Arc<Feed>,
    mp4: Option<SyncSender<Msg>>,
    first: Mutex<Option<Instant>>,
    scratch: Mutex<Vec<u8>>,
}

impl Sink {
    /// Takes the `area` of a frame whose pixels are BGRA (or RGBA when `rgba`), rows
    /// `stride` bytes apart.
    fn push(&self, data: &[u8], stride: usize, area: Area, rgba: bool) {
        let time = {
            let mut first = self.first.lock().unwrap();
            first.get_or_insert_with(Instant::now).elapsed()
        };
        let row = area.w as usize * 4;
        let last = (area.y + area.h).saturating_sub(1) as usize * stride + area.x as usize * 4 + row;
        if area.w == 0 || area.h == 0 || data.len() < last {
            return;
        }
        let mut packed = self.scratch.lock().unwrap();
        packed.clear();
        packed.reserve(row * area.h as usize);
        for y in area.y..area.y + area.h {
            let start = y as usize * stride + area.x as usize * 4;
            let src = &data[start..start + row];
            if rgba {
                for px in src.chunks_exact(4) {
                    packed.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            } else {
                packed.extend_from_slice(src);
            }
        }
        // X11 and most compositors leave the alpha byte undefined (BGRx).
        for px in packed.chunks_exact_mut(4) {
            px[3] = 255;
        }
        self.feed.push_bgra(area.w, area.h, row, &packed, time);
        if let Some(tx) = &self.mp4 {
            Mp4Encoder::offer(
                tx,
                Msg::Frame {
                    width: area.w,
                    height: area.h,
                    bgra: packed.clone(),
                    time,
                },
            );
        }
    }
}

struct LinuxBackend {
    stop: Arc<AtomicBool>,
    capture: Option<JoinHandle<()>>,
    pipewire: Option<pipewire::channel::Sender<()>>,
    mp4: Option<Mp4Encoder>,
    sink: Arc<Sink>,
}

impl Backend for LinuxBackend {
    fn stop(mut self: Box<Self>) -> Result<(), String> {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(tx) = self.pipewire.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.capture.take() {
            let _ = handle.join();
        }
        let duration = self
            .sink
            .first
            .lock()
            .unwrap()
            .map(|t| t.elapsed())
            .unwrap_or_default();
        match self.mp4.take() {
            Some(mp4) => mp4.finish(duration),
            None => Ok(()),
        }
    }
}

pub fn start(app: &AppHandle, target: &Target, feed: Arc<Feed>) -> Result<Box<dyn Backend>, String> {
    let mp4 = match target.format {
        Format::Mp4 => Some(Mp4Encoder::spawn(&target.path, target.size, target.fps)?),
        Format::Gif => None,
    };
    let sink = Arc::new(Sink {
        feed,
        mp4: mp4.as_ref().map(Mp4Encoder::sender),
        first: Mutex::new(None),
        scratch: Mutex::new(Vec::new()),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let mut backend = LinuxBackend {
        stop: stop.clone(),
        capture: None,
        pipewire: None,
        mp4,
        sink: sink.clone(),
    };
    let started = if crate::linux::is_wayland() {
        wayland::start(app, target, sink).map(|tx| backend.pipewire = Some(tx))
    } else {
        x11::start(target, sink, stop).map(|handle| backend.capture = Some(handle))
    };
    if let Err(err) = started {
        let _ = Box::new(backend).stop();
        let _ = std::fs::remove_file(&target.path);
        return Err(err);
    }
    Ok(Box::new(backend))
}

/// MP4 keeps even sizes: drop the odd pixel. GIF frames are scaled later anyway.
fn even(target: &Target, w: u32, h: u32) -> (u32, u32) {
    match target.format {
        Format::Mp4 => ((w & !1).max(2), (h & !1).max(2)),
        Format::Gif => (w.max(1), h.max(1)),
    }
}

mod x11 {
    use super::*;
    use xcb::{shm, x, xfixes};

    struct Shm {
        addr: *mut libc::c_void,
        size: usize,
        seg: shm::Seg,
    }

    impl Drop for Shm {
        fn drop(&mut self) {
            unsafe {
                libc::shmdt(self.addr);
            }
        }
    }

    // The segment is only touched by the capture thread.
    unsafe impl Send for Shm {}

    fn attach(conn: &xcb::Connection, size: usize) -> Option<Shm> {
        unsafe {
            let id = libc::shmget(libc::IPC_PRIVATE, size, libc::IPC_CREAT | 0o600);
            if id < 0 {
                return None;
            }
            let addr = libc::shmat(id, std::ptr::null(), 0);
            // Freed by the kernel once both sides detach.
            libc::shmctl(id, libc::IPC_RMID, std::ptr::null_mut());
            if addr as isize == -1 {
                return None;
            }
            let seg: shm::Seg = conn.generate_id();
            let attached = conn.send_and_check_request(&shm::Attach {
                shmseg: seg,
                shmid: id as u32,
                read_only: false,
            });
            let shm = Shm { addr, size, seg };
            attached.ok().map(|_| shm)
        }
    }

    pub fn start(
        target: &Target,
        sink: Arc<Sink>,
        stop: Arc<AtomicBool>,
    ) -> Result<JoinHandle<()>, String> {
        let (conn, screen) = xcb::Connection::connect_with_extensions(
            None,
            &[],
            &[xcb::Extension::Shm, xcb::Extension::XFixes],
        )
        .map_err(|e| format!("sem conexão com o X11: {e}"))?;
        let root = conn
            .get_setup()
            .roots()
            .nth(screen as usize)
            .ok_or("tela do X11 não encontrada")?
            .root();

        // The area on the root window: the monitor's origin plus the selection, both
        // in X pixels (screenshot pixels are X pixels on X11).
        let m = &target.monitor;
        let r = &target.rect;
        let s = m.px_per_unit.max(0.01);
        let (w, h) = even(target, (r.width as f64 / s) as u32, (r.height as f64 / s) as u32);
        let area = (
            (m.x + (r.x as f64 / s) as i32) as i16,
            (m.y + (r.y as f64 / s) as i32) as i16,
            w as u16,
            h as u16,
        );
        let has_xfixes = conn
            .wait_for_reply(conn.send_request(&xfixes::QueryVersion {
                client_major_version: 4,
                client_minor_version: 0,
            }))
            .is_ok();
        let segment = conn
            .active_extensions()
            .any(|e| e == xcb::Extension::Shm)
            .then(|| attach(&conn, w as usize * h as usize * 4))
            .flatten();
        if segment.is_none() {
            log::info!("recording without MIT-SHM (slower)");
        }
        let interval = Duration::from_secs_f64(1.0 / target.fps.max(1) as f64);

        std::thread::Builder::new()
            .name("x11-record".into())
            .spawn(move || {
                let mut frame = vec![0u8; w as usize * h as usize * 4];
                let start = Instant::now();
                let mut n: u32 = 0;
                while !stop.load(Ordering::SeqCst) {
                    if grab(&conn, root, area, segment.as_ref(), &mut frame) {
                        if has_xfixes {
                            draw_cursor(&conn, area, &mut frame);
                        }
                        let a = Area { x: 0, y: 0, w, h };
                        sink.push(&frame, w as usize * 4, a, false);
                    }
                    // Keep to the clock; skip frames when a grab ran late.
                    n += 1;
                    let mut next = start + interval * n;
                    let now = Instant::now();
                    while next < now {
                        n += 1;
                        next = start + interval * n;
                    }
                    std::thread::sleep(next - now);
                }
                if let Some(seg) = &segment {
                    conn.send_request(&shm::Detach { shmseg: seg.seg });
                    let _ = conn.flush();
                }
            })
            .map_err(|e| e.to_string())
    }

    fn grab(
        conn: &xcb::Connection,
        root: x::Window,
        (x, y, w, h): (i16, i16, u16, u16),
        segment: Option<&Shm>,
        out: &mut [u8],
    ) -> bool {
        match segment {
            Some(seg) => {
                let cookie = conn.send_request(&shm::GetImage {
                    drawable: x::Drawable::Window(root),
                    x,
                    y,
                    width: w,
                    height: h,
                    plane_mask: u32::MAX,
                    format: x::ImageFormat::ZPixmap as u8,
                    shmseg: seg.seg,
                    offset: 0,
                });
                if conn.wait_for_reply(cookie).is_err() {
                    return false;
                }
                let data = unsafe { std::slice::from_raw_parts(seg.addr as *const u8, seg.size) };
                out.copy_from_slice(&data[..out.len()]);
                true
            }
            None => {
                let cookie = conn.send_request(&x::GetImage {
                    format: x::ImageFormat::ZPixmap,
                    drawable: x::Drawable::Window(root),
                    x,
                    y,
                    width: w,
                    height: h,
                    plane_mask: u32::MAX,
                });
                match conn.wait_for_reply(cookie) {
                    Ok(reply) if reply.data().len() >= out.len() => {
                        out.copy_from_slice(&reply.data()[..out.len()]);
                        true
                    }
                    _ => false,
                }
            }
        }
    }

    /// Blends the mouse pointer (premultiplied ARGB from XFixes) into the frame.
    fn draw_cursor(conn: &xcb::Connection, (ax, ay, w, h): (i16, i16, u16, u16), frame: &mut [u8]) {
        let Ok(cursor) = conn.wait_for_reply(conn.send_request(&xfixes::GetCursorImage {})) else {
            return;
        };
        let left = cursor.x() as i32 - cursor.xhot() as i32 - ax as i32;
        let top = cursor.y() as i32 - cursor.yhot() as i32 - ay as i32;
        let (cw, ch) = (cursor.width() as i32, cursor.height() as i32);
        let pixels = cursor.cursor_image();
        for cy in 0..ch {
            let fy = top + cy;
            if fy < 0 || fy >= h as i32 {
                continue;
            }
            for cx in 0..cw {
                let fx = left + cx;
                if fx < 0 || fx >= w as i32 {
                    continue;
                }
                let argb = pixels[(cy * cw + cx) as usize];
                let a = argb >> 24;
                if a == 0 {
                    continue;
                }
                let i = (fy as usize * w as usize + fx as usize) * 4;
                let inv = 255 - a;
                for (c, shift) in [(0, 0), (1, 8), (2, 16)] {
                    let src = (argb >> shift) & 0xff;
                    frame[i + c] = (src + frame[i + c] as u32 * inv / 255).min(255) as u8;
                }
            }
        }
    }
}

mod wayland {
    use super::*;
    use ashpd::desktop::{
        screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream},
        PersistMode,
    };
    use pipewire::{
        context::ContextRc,
        main_loop::MainLoopRc,
        properties::properties,
        spa::{
            param::{
                format::{FormatProperties, MediaSubtype, MediaType},
                format_utils,
                video::{VideoFormat, VideoInfoRaw},
                ParamType,
            },
            pod::{self, serialize::PodSerializer, Pod},
            utils::{Direction, Fraction, Rectangle, SpaTypes},
        },
        stream::{StreamFlags, StreamRc},
    };

    /// Asks the portal for the monitor's stream. Reuses the screen the user shared last
    /// time (restore token) and asks again only if that isn't the monitor recorded now.
    fn open(app: &AppHandle, target: &Target) -> Result<(Stream, OwnedFd), String> {
        let token = app
            .state::<config::ConfigState>()
            .0
            .lock()
            .unwrap()
            .screencast_token
            .clone();
        let m = &target.monitor;
        let monitors = xcap::Monitor::all().map(|l| l.len()).unwrap_or(1);
        let mut token = token;
        for _ in 0..2 {
            let (stream, fd, new_token) = pollster::block_on(request(token.as_deref()))?;
            let matches = match stream.position() {
                Some((x, y)) => (x, y) == (m.x, m.y) || monitors <= 1,
                None => true,
            };
            if matches || token.is_none() {
                let state = app.state::<config::ConfigState>();
                let mut config = state.0.lock().unwrap();
                if config.screencast_token != new_token {
                    config.screencast_token = new_token;
                    let _ = config::store(app, &config);
                }
                return Ok((stream, fd));
            }
            // The remembered screen is another monitor: ask which one.
            token = None;
        }
        Err("a tela compartilhada não é o monitor da gravação".into())
    }

    async fn request(token: Option<&str>) -> Result<(Stream, OwnedFd, Option<String>), String> {
        let err = |e: ashpd::Error| match e {
            ashpd::Error::Response(_) => "A gravação foi cancelada na janela do sistema.".to_string(),
            other => format!("portal de gravação de tela: {other}"),
        };
        let proxy = Screencast::new().await.map_err(err)?;
        let cursor = proxy
            .available_cursor_modes()
            .await
            .ok()
            .filter(|modes| modes.contains(CursorMode::EMBEDDED))
            .map(|_| CursorMode::EMBEDDED);
        let session = proxy.create_session(Default::default()).await.map_err(err)?;
        proxy
            .select_sources(
                &session,
                SelectSourcesOptions::default()
                    .set_sources(SourceType::MONITOR)
                    .set_multiple(false)
                    .set_cursor_mode(cursor)
                    .set_persist_mode(PersistMode::ExplicitlyRevoked)
                    .set_restore_token(token),
            )
            .await
            .map_err(err)?
            .response()
            .map_err(err)?;
        let streams = proxy
            .start(&session, None, Default::default())
            .await
            .map_err(err)?
            .response()
            .map_err(err)?;
        let stream = streams
            .streams()
            .first()
            .cloned()
            .ok_or("nenhuma tela foi escolhida")?;
        let fd = proxy
            .open_pipe_wire_remote(&session, Default::default())
            .await
            .map_err(err)?;
        // The session stays open as long as the D-Bus connection does (the app's).
        std::mem::forget(session);
        Ok((stream, fd, streams.restore_token().map(str::to_string)))
    }

    pub fn start(
        app: &AppHandle,
        target: &Target,
        sink: Arc<Sink>,
    ) -> Result<pipewire::channel::Sender<()>, String> {
        let (stream, fd) = open(app, target)?;
        let node = stream.pipe_wire_node_id();

        // The selection in screenshot pixels; frames may come at another size.
        let m = &target.monitor;
        let shot_w = (m.width as f64 * m.px_per_unit).round().max(1.0);
        let shot_h = (m.height as f64 * m.px_per_unit).round().max(1.0);
        let r = target.rect;
        let format = target.format;
        let (stop_tx, stop_rx) = pipewire::channel::channel::<()>();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let interval = Duration::from_secs_f64(1.0 / target.fps.max(1) as f64);

        std::thread::Builder::new()
            .name("pipewire-record".into())
            .spawn(move || {
                let result = (|| -> Result<(), String> {
                    pipewire::init();
                    let main_loop = MainLoopRc::new(None).map_err(|e| e.to_string())?;
                    let context = ContextRc::new(&main_loop, None).map_err(|e| e.to_string())?;
                    let core = context.connect_fd_rc(fd, None).map_err(|e| e.to_string())?;
                    let pw_stream = StreamRc::new(
                        core.clone(),
                        "Kadro",
                        properties! {
                            *pipewire::keys::MEDIA_TYPE => "Video",
                            *pipewire::keys::MEDIA_CATEGORY => "Capture",
                            *pipewire::keys::MEDIA_ROLE => "Screen",
                        },
                    )
                    .map_err(|e| e.to_string())?;

                    struct Data {
                        format: VideoInfoRaw,
                        last: Option<Instant>,
                    }
                    let _listener = pw_stream
                        .add_local_listener_with_user_data(Data {
                            format: Default::default(),
                            last: None,
                        })
                        .param_changed(|_, data, id, param| {
                            let Some(param) = param else { return };
                            if id != ParamType::Format.as_raw() {
                                return;
                            }
                            if let Ok((MediaType::Video, MediaSubtype::Raw)) =
                                format_utils::parse_format(param)
                            {
                                let _ = data.format.parse(param);
                            }
                        })
                        .process(move |stream, data| {
                            let Some(mut buffer) = stream.dequeue_buffer() else {
                                return;
                            };
                            // Frames come at the screen's refresh rate: keep `fps`.
                            let now = Instant::now();
                            if data.last.is_some_and(|t| now - t < interval.mul_f32(0.9)) {
                                return;
                            }
                            let size = data.format.size();
                            let rgba = match data.format.format() {
                                VideoFormat::BGRx | VideoFormat::BGRA => false,
                                VideoFormat::RGBx | VideoFormat::RGBA => true,
                                _ => return,
                            };
                            let datas = buffer.datas_mut();
                            let Some(plane) = datas.first_mut() else { return };
                            let chunk = plane.chunk();
                            let (offset, stride) = (chunk.offset() as usize, chunk.stride());
                            let stride = if stride > 0 {
                                stride as usize
                            } else {
                                size.width as usize * 4
                            };
                            let Some(bytes) = plane.data() else { return };
                            if bytes.len() <= offset {
                                return;
                            }
                            let bytes = &bytes[offset..];
                            // The selection, scaled to the frame.
                            let sx = size.width as f64 / shot_w;
                            let sy = size.height as f64 / shot_h;
                            let x = ((r.x as f64 * sx) as u32).min(size.width.saturating_sub(2));
                            let y = ((r.y as f64 * sy) as u32).min(size.height.saturating_sub(2));
                            let w = ((r.width as f64 * sx) as u32).min(size.width - x);
                            let h = ((r.height as f64 * sy) as u32).min(size.height - y);
                            let (w, h) = match format {
                                Format::Mp4 => ((w & !1).max(2), (h & !1).max(2)),
                                Format::Gif => (w.max(1), h.max(1)),
                            };
                            data.last = Some(now);
                            sink.push(bytes, stride, Area { x, y, w, h }, rgba);
                        })
                        .register()
                        .map_err(|e| e.to_string())?;

                    let obj = pod::object!(
                        SpaTypes::ObjectParamFormat,
                        ParamType::EnumFormat,
                        pod::property!(FormatProperties::MediaType, Id, MediaType::Video),
                        pod::property!(FormatProperties::MediaSubtype, Id, MediaSubtype::Raw),
                        pod::property!(
                            FormatProperties::VideoFormat,
                            Choice,
                            Enum,
                            Id,
                            VideoFormat::BGRx,
                            VideoFormat::BGRx,
                            VideoFormat::BGRA,
                            VideoFormat::RGBx,
                            VideoFormat::RGBA
                        ),
                        pod::property!(
                            FormatProperties::VideoSize,
                            Choice,
                            Range,
                            Rectangle,
                            Rectangle { width: 1920, height: 1080 },
                            Rectangle { width: 1, height: 1 },
                            Rectangle { width: 8192, height: 8192 }
                        ),
                        pod::property!(
                            FormatProperties::VideoFramerate,
                            Choice,
                            Range,
                            Fraction,
                            Fraction { num: 30, denom: 1 },
                            Fraction { num: 0, denom: 1 },
                            Fraction { num: 1000, denom: 1 }
                        ),
                    );
                    let values = PodSerializer::serialize(
                        std::io::Cursor::new(Vec::new()),
                        &pod::Value::Object(obj),
                    )
                    .map_err(|e| format!("{e:?}"))?
                    .0
                    .into_inner();
                    let mut params = [Pod::from_bytes(&values).ok_or("pod")?];
                    pw_stream
                        .connect(
                            Direction::Input,
                            Some(node),
                            StreamFlags::AUTOCONNECT | StreamFlags::MAP_BUFFERS,
                            &mut params,
                        )
                        .map_err(|e| e.to_string())?;

                    let quit = main_loop.clone();
                    let _stop = stop_rx.attach(main_loop.loop_(), move |()| quit.quit());
                    let _ = ready_tx.send(Ok(()));
                    main_loop.run();
                    let _ = pw_stream.disconnect();
                    Ok(())
                })();
                if let Err(err) = result {
                    let _ = ready_tx.send(Err(err));
                }
            })
            .map_err(|e| e.to_string())?;

        match ready_rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(stop_tx),
            Ok(Err(err)) => Err(err),
            Err(_) => Err("o PipeWire não respondeu".into()),
        }
    }
}
