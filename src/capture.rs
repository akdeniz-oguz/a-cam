//! The capture thread: starts the stream, decodes H.264 and offers the latest frame.
//!
//! The user interface never does any of this work; it only calls `take_frame()` and uploads
//! the ready RGBA frame to a GPU texture. Only the newest frame is kept, so if the interface
//! cannot keep up the older frames are dropped and latency does not build up.

use crate::adb::Tools;
use crate::stream::{Config, Logger, Stream};
use openh264::decoder::Decoder;
use openh264::formats::YUVSource as _;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

/// A frame ready to be uploaded for display.
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Writes a decoded I420 frame as the NV12 the virtual camera expects, scaled to the fixed
/// output size.
///
/// I420 and NV12 differ only in whether the U and V planes are separate or interleaved, so
/// the conversion is cheap. Scaling uses nearest neighbour: the source is already close to
/// the target size, and this runs on every frame, so staying cheap matters.
fn write_nv12(yuv: &openh264::decoder::DecodedYUV<'_>, view: &Transform, target: &mut [u8]) {
    let (src_w, src_h) = yuv.dimensions();
    let (dst_w, dst_h) = (a_cam_shared::WIDTH as usize, a_cam_shared::HEIGHT as usize);
    if src_w < 2 || src_h < 2 || target.len() != a_cam_shared::PIXEL_BYTES {
        return;
    }
    let (sample_x, sample_y, sideways) = view.sampling(src_w, src_h, dst_w, dst_h);
    let (y_stride, u_stride, v_stride) = yuv.strides();
    let (luma, chroma) = target.split_at_mut(dst_w * dst_h);
    let (y_plane, u_plane, v_plane) = (yuv.y(), yuv.u(), yuv.v());

    for row in 0..dst_h {
        let out = &mut luma[row * dst_w..][..dst_w];
        for (column, pixel) in out.iter_mut().enumerate() {
            // When rotated, a destination row corresponds to a source column.
            let (sx, sy) = if sideways {
                (sample_x[row], sample_y[column])
            } else {
                (sample_x[column], sample_y[row])
            };
            *pixel = y_plane[sy * y_stride + sx];
        }
    }
    // The colour planes are half resolution on both axes, so step through the full
    // resolution tables two at a time and halve the source position.
    for row in 0..dst_h / 2 {
        let out = &mut chroma[row * dst_w..][..dst_w];
        for column in 0..dst_w / 2 {
            let (sx, sy) = if sideways {
                (sample_x[row * 2], sample_y[column * 2])
            } else {
                (sample_x[column * 2], sample_y[row * 2])
            };
            let (cx, cy) = (sx / 2, sy / 2);
            out[column * 2] = u_plane[cy * u_stride + cx];
            out[column * 2 + 1] = v_plane[cy * v_stride + cx];
        }
    }
}

/// View settings applied to the virtual camera output.
///
/// The interface applies these as texture coordinates on the GPU and never touches the
/// frame data, so the virtual camera has to apply them separately. The interface writes
/// here whenever a value changes; the capture thread reads it for every frame.
#[derive(Clone, Copy)]
pub struct Transform {
    pub rotation: usize,
    pub mirror: bool,
    /// Zoom factor, where 1.0 means none.
    pub zoom: f32,
    /// Crop in the orientation shown on screen, normalised: (left, top, right, bottom).
    pub crop: (f32, f32, f32, f32),
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            rotation: 0,
            mirror: false,
            zoom: 1.0,
            crop: (0.0, 0.0, 1.0, 1.0),
        }
    }
}

impl Transform {
    /// Lookup tables mapping a destination pixel to a source pixel.
    ///
    /// Because the transforms are multiples of 90° plus a flip, the mapping separates by
    /// axis: source X depends on one destination axis and source Y on the other. Two tables
    /// are therefore enough, and the inner loop has no floating point left in it.
    fn sampling(
        &self,
        src_w: usize,
        src_h: usize,
        dst_w: usize,
        dst_h: usize,
    ) -> (Vec<usize>, Vec<usize>, bool) {
        let (left, top, right, bottom) = self.crop;
        // Zoom is applied around the centre of the cropped area.
        let span = 1.0 / self.zoom.max(1.0);
        let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
        let (half_w, half_h) = ((right - left) * span / 2.0, (bottom - top) * span / 2.0);
        let (u0, u1) = (cx - half_w, cx + half_w);
        let (v0, v1) = (cy - half_h, cy + half_h);

        // Normalised positions within the visible area.
        let along = |count: usize, start: f32, end: f32| -> Vec<f32> {
            (0..count)
                .map(|i| start + (end - start) * (i as f32 + 0.5) / count as f32)
                .collect()
        };
        let us = along(dst_w, u0, u1);
        let vs = along(dst_h, v0, v1);

        let to_x = |f: f32| ((f * src_w as f32) as usize).min(src_w - 1);
        let to_y = |f: f32| ((f * src_h as f32) as usize).min(src_h - 1);

        // Map the on-screen orientation back to the source, as the interface does.
        let sideways = self.rotation % 2 == 1;
        let (mut xs, ys): (Vec<usize>, Vec<usize>) = match self.rotation % 4 {
            1 => (
                vs.iter().map(|&f| to_x(f)).collect(),
                us.iter().map(|&f| to_y(1.0 - f)).collect(),
            ),
            2 => (
                us.iter().map(|&f| to_x(1.0 - f)).collect(),
                vs.iter().map(|&f| to_y(1.0 - f)).collect(),
            ),
            3 => (
                vs.iter().map(|&f| to_x(1.0 - f)).collect(),
                us.iter().map(|&f| to_y(f)).collect(),
            ),
            _ => (
                us.iter().map(|&f| to_x(f)).collect(),
                vs.iter().map(|&f| to_y(f)).collect(),
            ),
        };
        if self.mirror {
            for x in &mut xs {
                *x = src_w - 1 - *x;
            }
        }
        (xs, ys, sideways)
    }
}

/// Messages from the capture thread to the user interface.
pub enum Event {
    Log(String),
    Streaming { width: usize, height: usize },
    Stopped(Option<String>),
}

struct Shared {
    /// Decoded frame rate, times ten so it fits in an integer atomic.
    decode_fps10: AtomicU32,
    latest: Mutex<Option<Frame>>,
}

pub struct Capture {
    shared: Arc<Shared>,
    /// Shared with the stream layer: the socket read exits as soon as it sees this flag.
    stop: Arc<AtomicBool>,
    events: Receiver<Event>,
    handle: Option<JoinHandle<()>>,
}

impl Capture {
    pub fn start(
        tools: Tools,
        serial: String,
        config: Config,
        transform: Arc<Mutex<Transform>>,
    ) -> Self {
        let shared = Arc::new(Shared {
            decode_fps10: AtomicU32::new(0),
            latest: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, events) = channel();
        let handle = {
            let shared = shared.clone();
            let stop = stop.clone();
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("a-cam-capture".into())
                .spawn(move || {
                    let result = run(&tools, &serial, &config, &shared, &stop, &tx, &transform);
                    let error = match result {
                        Ok(()) => None,
                        // If the user stopped us, errors during shutdown do not matter.
                        Err(_) if stop.load(Ordering::Relaxed) => None,
                        Err(err) => Some(format!("{err:#}")),
                    };
                    let _ = tx.send(Event::Stopped(error));
                })
                .expect("capture thread")
        };
        Self {
            shared,
            stop,
            events,
            handle: Some(handle),
        }
    }

    /// Takes the newest frame and empties the slot, so no frame is drawn twice.
    pub fn take_frame(&self) -> Option<Frame> {
        self.shared.latest.lock().ok()?.take()
    }

    pub fn decode_fps(&self) -> f32 {
        self.shared.decode_fps10.load(Ordering::Relaxed) as f32 / 10.0
    }

    pub fn poll_events(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.handle.take() {
            // Closing the socket makes the read fail at once, so this does not block long.
            let _ = handle.join();
        }
    }
}

fn run(
    tools: &Tools,
    serial: &str,
    config: &Config,
    shared: &Arc<Shared>,
    stop: &Arc<AtomicBool>,
    tx: &Sender<Event>,
    transform: &Arc<Mutex<Transform>>,
) -> anyhow::Result<()> {
    let log: Logger = {
        let tx = tx.clone();
        Arc::new(move |line| {
            let _ = tx.send(Event::Log(line));
        })
    };
    let mut live = Stream::start(tools, serial, config, log, stop.clone())?;
    let mut decoder = Decoder::new()?;

    // SPS/PPS not yet joined to a frame. openh264 accepts them separately, but prefixing
    // them to the IDR frame is the safe path across versions.
    let mut pending_config: Vec<u8> = Vec::new();
    let mut announced = false;
    let mut rgba: Vec<u8> = Vec::new();
    let mut counter = FpsCounter::new();

    // The virtual camera bridge. If it cannot be opened the stream still runs normally,
    // just without webcam output.
    let mut bridge = match a_cam_shared::Writer::create() {
        Ok(writer) => Some(writer),
        Err(err) => {
            let _ = tx.send(Event::Log(format!(
                "Virtual camera bridge unavailable: {err}"
            )));
            None
        }
    };
    let mut nv12 = vec![0u8; a_cam_shared::PIXEL_BYTES];

    while !stop.load(Ordering::Relaxed) {
        let Some(packet) = live.read_packet()? else {
            break;
        };
        if packet.is_config {
            pending_config.extend_from_slice(&packet.data);
            continue;
        }
        let payload = if pending_config.is_empty() {
            packet.data
        } else {
            let mut joined = std::mem::take(&mut pending_config);
            joined.extend_from_slice(&packet.data);
            joined
        };

        let Some(yuv) = decoder.decode(&payload)? else {
            continue; // the decoder has not produced a frame yet
        };
        let (width, height) = yuv.dimensions();
        if width == 0 || height == 0 {
            continue;
        }
        rgba.resize(width * height * 4, 0);
        yuv.write_rgba8(&mut rgba);

        // Hand the same frame to the virtual camera. The conversion happens here because
        // the decoder's YUV planes are only valid in this scope.
        if let Some(writer) = &mut bridge {
            let view = transform.lock().map(|view| *view).unwrap_or_default();
            write_nv12(&yuv, &view, &mut nv12);
            writer.write(&nv12);
        }

        if !announced {
            announced = true;
            let _ = tx.send(Event::Streaming { width, height });
        }
        if let Some(fps) = counter.tick() {
            shared
                .decode_fps10
                .store((fps * 10.0) as u32, Ordering::Relaxed);
        }
        if let Ok(mut slot) = shared.latest.lock() {
            *slot = Some(Frame {
                width,
                height,
                rgba: rgba.clone(),
            });
        }
    }
    Ok(())
}

struct FpsCounter {
    since: Instant,
    frames: u32,
}

impl FpsCounter {
    fn new() -> Self {
        Self {
            since: Instant::now(),
            frames: 0,
        }
    }

    /// Returns the frame rate, measured once a second.
    fn tick(&mut self) -> Option<f32> {
        self.frames += 1;
        let elapsed = self.since.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            let fps = self.frames as f32 / elapsed;
            self.since = Instant::now();
            self.frames = 0;
            return Some(fps);
        }
        None
    }
}
