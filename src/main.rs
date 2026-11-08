//! a-Cam: shows the camera of a USB-connected Android phone on the desktop.
//!
//! No APK is installed on the phone, and no OBS or third-party virtual camera driver.
//!
//! Architecture: the capture thread (adb → scrcpy-server → H.264 → RGBA) and the user
//! interface are fully separate. The interface only uploads a ready frame to a GPU texture.
//! Rotation, mirroring and zoom are texture coordinates, so they are free on the GPU.

// No console window: this is a desktop application, and everything it has to say goes to
// the LOG panel rather than stdout. Debug builds keep the console for development.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod bundled;
mod capture;
mod stream;
mod vcam;
mod vcam_install;

use eframe::egui;
use std::time::{Duration, Instant};

const BG: egui::Color32 = egui::Color32::from_rgb(0x17, 0x18, 0x1c);
const VIDEO_BG: egui::Color32 = egui::Color32::from_rgb(0x0e, 0x0f, 0x12);
const MUTED: egui::Color32 = egui::Color32::from_rgb(0x9a, 0xa1, 0xab);
const FAINT: egui::Color32 = egui::Color32::from_rgb(0x5f, 0x66, 0x70);
const GREEN: egui::Color32 = egui::Color32::from_rgb(0x3e, 0xcf, 0x75);
const AMBER: egui::Color32 = egui::Color32::from_rgb(0xf5, 0xa5, 0x24);
const RED: egui::Color32 = egui::Color32::from_rgb(0xff, 0x6b, 0x6b);
const GREY: egui::Color32 = egui::Color32::from_rgb(0x6b, 0x72, 0x80);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x3d, 0x7b, 0xfd);
const DANGER: egui::Color32 = egui::Color32::from_rgb(0xe5, 0x48, 0x4d);

const APP_NAME: &str = "a-Cam";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const QUALITIES: [(&str, u32); 3] = [("720p", 1280), ("1080p", 1920), ("1440p", 2560)];
const ROTATIONS: [&str; 4] = ["0°", "90°", "180°", "270°"];

fn main() -> eframe::Result {
    // The elevated copy of ourselves that places and registers the camera source. It has
    // no window: it does the one job it was started for and exits.
    let arguments: Vec<String> = std::env::args().collect();
    for (flag, uninstall) in [
        (vcam_install::INSTALL_FLAG, false),
        (vcam_install::UNINSTALL_FLAG, true),
    ] {
        if arguments.iter().any(|argument| argument == flag) {
            vcam_install::run_elevated_action(uninstall);
            return Ok(());
        }
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(APP_NAME)
        .with_inner_size([1020.0, 660.0])
        .with_min_inner_size([780.0, 540.0]);
    if let Some(icon) = window_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(APP_NAME, options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

/// Decodes the embedded icon for the title bar and task bar.
///
/// The same artwork is compiled into the executable by `build.rs`; here it is decoded at
/// startup because the window wants raw RGBA pixels rather than a PNG.
fn window_icon() -> Option<egui::IconData> {
    let png = include_bytes!("../assets/icon.png");
    let image = image::load_from_memory(png).ok()?.into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

/// An uncropped picture: the whole frame.
const FULL_CROP: egui::Rect = egui::Rect {
    min: egui::Pos2 { x: 0.0, y: 0.0 },
    max: egui::Pos2 { x: 1.0, y: 1.0 },
};
/// At least this much must remain on every side after cropping.
const MIN_CROP: f32 = 0.10;

/// How close the pointer must be, in points, to grab a handle.
const HANDLE_GRAB: f32 = 14.0;

/// The crop handle being dragged: which edges move. Corner handles mark two edges; when
/// none are marked the whole frame is being moved.
#[derive(Clone, Copy, PartialEq, Eq)]
struct CropDrag {
    left: bool,
    top: bool,
    right: bool,
    bottom: bool,
}

impl CropDrag {
    const MOVE: Self = Self {
        left: false,
        top: false,
        right: false,
        bottom: false,
    };

    const fn edges(left: bool, top: bool, right: bool, bottom: bool) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    fn is_move(self) -> bool {
        self == Self::MOVE
    }

    fn cursor(self) -> egui::CursorIcon {
        match (self.left, self.top, self.right, self.bottom) {
            (true, true, _, _) | (_, _, true, true) => egui::CursorIcon::ResizeNwSe,
            (_, true, true, _) | (true, _, _, true) => egui::CursorIcon::ResizeNeSw,
            (true, _, _, _) | (_, _, true, _) => egui::CursorIcon::ResizeHorizontal,
            (_, true, _, _) | (_, _, _, true) => egui::CursorIcon::ResizeVertical,
            _ => egui::CursorIcon::Move,
        }
    }
}

/// The frame's eight handles: four corners and four edge midpoints, each with its edges.
fn crop_handles(frame: egui::Rect) -> [(egui::Pos2, CropDrag); 8] {
    let (cx, cy) = (frame.center().x, frame.center().y);
    [
        (frame.left_top(), CropDrag::edges(true, true, false, false)),
        (
            egui::pos2(cx, frame.min.y),
            CropDrag::edges(false, true, false, false),
        ),
        (frame.right_top(), CropDrag::edges(false, true, true, false)),
        (
            egui::pos2(frame.max.x, cy),
            CropDrag::edges(false, false, true, false),
        ),
        (
            frame.right_bottom(),
            CropDrag::edges(false, false, true, true),
        ),
        (
            egui::pos2(cx, frame.max.y),
            CropDrag::edges(false, false, false, true),
        ),
        (
            frame.left_bottom(),
            CropDrag::edges(true, false, false, true),
        ),
        (
            egui::pos2(frame.min.x, cy),
            CropDrag::edges(true, false, false, false),
        ),
    ]
}

/// Finds the handle under the pointer; inside the frame this is a move, outside nothing.
fn crop_hit(frame: egui::Rect, pos: egui::Pos2) -> Option<CropDrag> {
    let nearest = crop_handles(frame)
        .into_iter()
        .map(|(point, drag)| (point.distance(pos), drag))
        .min_by(|a, b| a.0.total_cmp(&b.0));
    match nearest {
        Some((distance, drag)) if distance <= HANDLE_GRAB => Some(drag),
        _ if frame.contains(pos) => Some(CropDrag::MOVE),
        _ => None,
    }
}

#[derive(PartialEq, Eq)]
enum State {
    Idle,
    Connecting,
    Streaming,
}

struct App {
    tools: Option<adb::Tools>,
    device: Option<adb::Device>,
    cameras: Vec<adb::Camera>,
    selected: usize,
    quality: usize,
    rotation: usize,
    mirror: bool,
    zoom: u32,

    /// Crop rectangle in the orientation shown on screen, normalised to 0..1.
    crop: egui::Rect,
    crop_mode: bool,
    crop_drag: Option<CropDrag>,

    /// Transform applied to the virtual camera output. The interface uses GPU texture
    /// coordinates and never touches frame data, so for the virtual camera to show the same
    /// picture the settings have to reach the capture thread as well.
    vcam_transform: std::sync::Arc<std::sync::Mutex<capture::Transform>>,

    /// While present, the virtual camera is visible system-wide. It is a COM object, so it
    /// is held and dropped on the user interface thread only.
    vcam: Option<vcam::VirtualCamera>,

    capture: Option<capture::Capture>,
    texture: Option<egui::TextureHandle>,
    source_size: [usize; 2],
    state: State,
    status: (String, egui::Color32),
    /// Frames actually painted per second, as opposed to frames decoded.
    paint_fps: f32,
    paint_counter: (Instant, u32),

    log: Vec<(String, egui::Color32)>,
    log_open: bool,
    /// Set once the "no phone" guidance has been written to the log, so the periodic
    /// device check does not repeat it every few seconds.
    phone_hint_shown: bool,
    /// Whether adb's background server was already running when we started. If we started
    /// it, we stop it on exit so nothing of ours is left behind in Task Manager.
    adb_was_running: bool,
    /// Whether the one-time elevated setup for the camera source has been done. Checked
    /// at startup and again after the setup runs.
    vcam_installed: bool,
    /// Whether the webcam setup has been offered this run, so declining is not met with
    /// the same prompt on every Start Stream.
    webcam_setup_asked: bool,
    /// Set while an elevated setup is running; the deadline bounds the wait.
    webcam_setup_until: Option<Instant>,
    webcam_setup_checked: Instant,
    busy: Option<std::sync::mpsc::Receiver<Job>>,
}

/// Result of background adb work; the interface never waits on adb.
enum Job {
    Device(Result<adb::Device, String>),
    Cameras(Result<Vec<adb::Camera>, String>),
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        let mut app = Self {
            tools: None,
            device: None,
            cameras: Vec::new(),
            selected: 0,
            quality: 1,
            rotation: 0,
            mirror: false,
            zoom: 100,
            crop: FULL_CROP,
            crop_mode: false,
            crop_drag: None,
            vcam_transform: std::sync::Arc::default(),
            vcam: None,
            capture: None,
            texture: None,
            source_size: [0, 0],
            state: State::Idle,
            status: ("Starting...".into(), GREY),
            paint_fps: 0.0,
            paint_counter: (Instant::now(), 0),
            log: Vec::new(),
            log_open: false,
            phone_hint_shown: false,
            adb_was_running: adb::server_running(),
            vcam_installed: vcam_install::is_installed(),
            webcam_setup_asked: false,
            webcam_setup_until: None,
            webcam_setup_checked: Instant::now(),
            busy: None,
        };
        match adb::Tools::find() {
            Ok(tools) => {
                app.info(format!("scrcpy-server {} ready.", tools.version));
                app.tools = Some(tools);
                app.refresh_device();
            }
            Err(err) => {
                app.error(format!("{err:#}"));
                app.status = ("scrcpy files not found".into(), RED);
            }
        }
        app
    }

    fn info(&mut self, line: String) {
        self.log.push((line, MUTED));
    }

    fn error(&mut self, line: String) {
        self.log.push((format!("ERROR: {line}"), RED));
    }

    /// Runs adb work in the background so the interface never stalls for a frame.
    fn spawn<T: Send + 'static>(
        &mut self,
        work: impl FnOnce() -> Result<T, String> + Send + 'static,
        wrap: impl FnOnce(Result<T, String>) -> Job + Send + 'static,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.busy = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(wrap(work()));
        });
    }

    fn refresh_device(&mut self) {
        let Some(tools) = self.tools.clone() else {
            return;
        };
        self.status = ("Looking for a phone...".into(), GREY);
        self.spawn(
            move || tools.find_device().map_err(|e| format!("{e:#}")),
            Job::Device,
        );
    }

    fn list_cameras(&mut self) {
        let (Some(tools), Some(device)) = (self.tools.clone(), self.device.clone()) else {
            self.refresh_device();
            return;
        };
        self.status = ("Querying cameras...".into(), GREY);
        self.spawn(
            move || {
                tools
                    .list_cameras(&device.serial)
                    .map_err(|e| format!("{e:#}"))
            },
            Job::Cameras,
        );
    }

    fn poll_jobs(&mut self) {
        let Some(rx) = &self.busy else { return };
        let Ok(job) = rx.try_recv() else { return };
        self.busy = None;
        match job {
            Job::Device(Ok(device)) => {
                if device.others > 0 {
                    self.info(format!(
                        "{} more phone(s) connected; using this one.",
                        device.others
                    ));
                }
                self.status = (format!("{} connected", device.model), GREEN);
                self.device = Some(device);
                // Fetch the camera list automatically: otherwise the dropdown looks empty
                // and it is not obvious that "List Cameras" has to be pressed first.
                if self.cameras.is_empty() {
                    self.list_cameras();
                }
            }
            Job::Device(Err(err)) if err.starts_with("No phone found") => {
                // Only surface the long explanation once, so the periodic device check does
                // not fill the log while the user is plugging the phone in.
                if !self.phone_hint_shown {
                    self.phone_hint_shown = true;
                    self.info(err.clone());
                }
                self.status = ("No phone connected".into(), AMBER);
                self.device = None;
            }
            Job::Device(Err(err)) => {
                self.status = (first_sentence(&err), AMBER);
                self.device = None;
            }
            Job::Cameras(Ok(cameras)) => {
                self.info(format!("Found {} cameras.", cameras.len()));
                self.selected = 0;
                self.cameras = cameras;
                if let Some(device) = &self.device {
                    self.status = (
                        format!("{} · {} cameras", device.model, self.cameras.len()),
                        GREEN,
                    );
                }
            }
            Job::Cameras(Err(err)) => {
                self.error(err.clone());
                self.status = ("Could not list cameras".into(), RED);
            }
        }
    }

    fn start_stream(&mut self) {
        let (Some(tools), Some(device)) = (self.tools.clone(), self.device.clone()) else {
            self.refresh_device();
            return;
        };
        let camera_id = self
            .cameras
            .get(self.selected)
            .map_or_else(|| "0".to_string(), |camera| camera.id.clone());
        let config = stream::Config {
            camera_id: camera_id.clone(),
            max_size: QUALITIES[self.quality].1,
            ..Default::default()
        };
        self.info(format!(
            "Starting camera {camera_id} at {}.",
            QUALITIES[self.quality].0
        ));
        self.publish_transform();
        self.capture = Some(capture::Capture::start(
            tools,
            device.serial,
            config,
            self.vcam_transform.clone(),
        ));
        self.start_virtual_camera();
        self.state = State::Connecting;
        self.status = ("Connecting...".into(), ACCENT);
    }

    /// Asks Windows to run the one-time setup that lets it load the camera source.
    ///
    /// Started from Start Stream rather than from a button of its own: the consent prompt
    /// Windows raises is already the place to decide, and asking twice helps nobody. It is
    /// offered once per run, so declining does not turn into nagging.
    fn request_webcam_setup(&mut self) {
        self.webcam_setup_asked = true;
        match vcam_install::request_install(false) {
            Ok(()) => {
                // The elevated copy runs on its own. Rather than blocking the interface
                // while the consent prompt is up, watch for the result from the UI loop.
                self.webcam_setup_until = Some(Instant::now() + Duration::from_secs(60));
                self.info("Waiting for webcam setup to finish...".into());
            }
            Err(err) => {
                self.info(format!("Webcam output not enabled: {err:#}"));
            }
        }
    }

    /// Picks up the result of the elevated setup without blocking the interface.
    fn poll_webcam_setup(&mut self) {
        let Some(deadline) = self.webcam_setup_until else {
            return;
        };
        let now = Instant::now();
        // A registry read every frame would be wasteful for something this slow.
        if now < self.webcam_setup_checked + Duration::from_millis(400) {
            return;
        }
        self.webcam_setup_checked = now;

        if vcam_install::is_installed() {
            self.webcam_setup_until = None;
            self.vcam_installed = true;
            self.info("Webcam output is ready.".into());
            if self.state == State::Streaming {
                self.start_virtual_camera();
            }
        } else if now >= deadline {
            self.webcam_setup_until = None;
            self.info(
                "Webcam output is off; the stream is unaffected. Restart a-Cam to be asked \
                 again."
                    .into(),
            );
        }
    }

    /// Brings the virtual camera up together with the stream.
    ///
    /// There is no separate button: the camera appears when streaming starts and goes away
    /// when it stops. If the DLL has not been registered (the one-off elevated step) the
    /// stream still runs, just without webcam output.
    fn start_virtual_camera(&mut self) {
        if self.vcam.is_some() {
            return;
        }
        // Without the one-time elevated setup there is nothing for Windows to load, so
        // ask for it now; the camera comes up as soon as it is done.
        if !self.vcam_installed {
            if vcam_install::AVAILABLE && !self.webcam_setup_asked {
                self.request_webcam_setup();
            }
            return;
        }
        match vcam::VirtualCamera::start() {
            Ok(camera) => {
                self.vcam = Some(camera);
                self.info(format!("Shared as a webcam: {}", vcam::FRIENDLY_NAME));
            }
            Err(err) => {
                self.error(format!("Webcam output unavailable: {err:#}"));
            }
        }
    }

    /// Passes the interface's view settings to the virtual camera side.
    fn publish_transform(&self) {
        if let Ok(mut view) = self.vcam_transform.lock() {
            *view = capture::Transform {
                rotation: self.rotation,
                mirror: self.mirror,
                zoom: self.zoom as f32 / 100.0,
                crop: (
                    self.crop.min.x,
                    self.crop.min.y,
                    self.crop.max.x,
                    self.crop.max.y,
                ),
            };
        }
    }

    fn stop_stream(&mut self) {
        if let Some(capture) = &mut self.capture {
            capture.stop();
        }
        self.capture = None; // dropping stops the capture thread and clears the adb forward
        self.vcam = None; // dropping removes the virtual camera from the system
        self.texture = None;
        self.state = State::Idle;
        self.status = match &self.device {
            Some(device) => (format!("{} connected", device.model), GREEN),
            None => ("Stream off".into(), GREY),
        };
    }

    /// Handles messages from the capture thread and the newest frame.
    fn pump_capture(&mut self, ctx: &egui::Context) {
        let Some(capture) = &self.capture else { return };

        let mut stopped: Option<Option<String>> = None;
        let mut streaming: Option<[usize; 2]> = None;
        let mut lines: Vec<String> = Vec::new();
        for event in capture.poll_events() {
            match event {
                capture::Event::Log(line) => lines.push(line),
                capture::Event::Streaming { width, height } => streaming = Some([width, height]),
                capture::Event::Stopped(error) => stopped = Some(error),
            }
        }
        let frame = capture.take_frame();

        for line in lines {
            self.log.push((line, FAINT));
        }
        if let Some(size) = streaming {
            self.source_size = size;
            self.state = State::Streaming;
            self.status = ("Streaming".into(), GREEN);
            self.info(format!("Streaming at {}x{}.", size[0], size[1]));
        }
        if let Some(frame) = frame {
            let size = [frame.width, frame.height];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba);
            match &mut self.texture {
                Some(texture) if texture.size() == size => {
                    texture.set(image, egui::TextureOptions::LINEAR);
                }
                _ => {
                    self.texture =
                        Some(ctx.load_texture("preview", image, egui::TextureOptions::LINEAR));
                }
            }
            self.source_size = size;
            self.paint_counter.1 += 1;
            let elapsed = self.paint_counter.0.elapsed().as_secs_f32();
            if elapsed >= 0.5 {
                self.paint_fps = self.paint_counter.1 as f32 / elapsed;
                self.paint_counter = (Instant::now(), 0);
            }
        }
        if let Some(error) = stopped {
            match error {
                Some(err) => {
                    self.error(err);
                    self.log_open = true;
                    self.stop_stream();
                    self.status = ("Stream stopped".into(), RED);
                }
                None => self.stop_stream(),
            }
        }
        if self.state != State::Idle {
            // Keep repainting while streaming; when idle egui sleeps on its own.
            ctx.request_repaint();
        }
    }

    // --- Interface pieces -------------------------------------------------------

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(APP_NAME).size(22.0).strong());
                ui.label(egui::RichText::new("Phone camera over USB").color(MUTED));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(&self.status.0).color(MUTED));
                // The status dot is painted rather than typed: the bundled font has no
                // geometric shapes, so characters like it came out as empty boxes.
                let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(dot.center(), 4.0, self.status.1);
            });
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let idle = self.state == State::Idle;
        let working = self.busy.is_some();

        ui.label(egui::RichText::new("CAMERA").color(MUTED).small().strong());
        ui.add_space(4.0);
        let label = match self.cameras.get(self.selected) {
            Some(camera) => camera.label(),
            None if working => "Looking for cameras...".to_string(),
            None => "No cameras yet".to_string(),
        };
        egui::ComboBox::from_id_salt("camera")
            .selected_text(label)
            .width(222.0)
            .show_ui(ui, |ui| {
                if self.cameras.is_empty() {
                    // An empty dropdown told the user nothing.
                    ui.label(
                        egui::RichText::new("Connect a phone, then List Cameras").color(FAINT),
                    );
                }
                for index in 0..self.cameras.len() {
                    let text = self.cameras[index].label();
                    ui.selectable_value(&mut self.selected, index, text);
                }
            });
        ui.add_space(6.0);
        if ui
            .add_enabled(idle && !working, egui::Button::new("List Cameras"))
            .clicked()
        {
            self.list_cameras();
        }

        ui.add_space(16.0);
        ui.label(
            egui::RichText::new("SETTINGS")
                .color(MUTED)
                .small()
                .strong(),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Quality").color(MUTED));
            egui::ComboBox::from_id_salt("quality")
                .selected_text(QUALITIES[self.quality].0)
                .show_ui(ui, |ui| {
                    for (index, (name, _)) in QUALITIES.iter().enumerate() {
                        ui.selectable_value(&mut self.quality, index, *name);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Rotation").color(MUTED));
            egui::ComboBox::from_id_salt("rotation")
                .selected_text(ROTATIONS[self.rotation])
                .show_ui(ui, |ui| {
                    for (index, name) in ROTATIONS.iter().enumerate() {
                        ui.selectable_value(&mut self.rotation, index, *name);
                    }
                });
        });
        ui.checkbox(&mut self.mirror, "Mirror");

        ui.add_space(16.0);
        ui.label(egui::RichText::new("ZOOM").color(MUTED).small().strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.small_button("-").clicked() {
                self.zoom = self.zoom.saturating_sub(10).max(100);
            }
            ui.add(
                egui::Slider::new(&mut self.zoom, 100..=400)
                    .suffix("%")
                    .step_by(10.0),
            );
            if ui.small_button("+").clicked() {
                self.zoom = (self.zoom + 10).min(400);
            }
        });

        ui.add_space(18.0);
        let start = egui::Button::new(egui::RichText::new("Start Stream").strong())
            .fill(ACCENT)
            .min_size(egui::vec2(222.0, 34.0));
        if ui.add_enabled(idle, start).clicked() {
            self.start_stream();
        }
        ui.add_space(6.0);
        let stop = egui::Button::new(egui::RichText::new("Stop").strong())
            .fill(DANGER)
            .min_size(egui::vec2(222.0, 34.0));
        if ui.add_enabled(!idle, stop).clicked() {
            self.stop_stream();
        }

        if self.vcam.is_some() {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("Webcam: {}", vcam::FRIENDLY_NAME))
                    .color(GREEN)
                    .small()
                    .strong(),
            );
            ui.label(
                egui::RichText::new("Pick it in Zoom, Meet, Discord...")
                    .color(FAINT)
                    .small(),
            );
        } else if self.webcam_setup_until.is_some() {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new("Setting up webcam output...")
                    .color(MUTED)
                    .small(),
            );
        }

        // The version line sits at the bottom of the card: a bottom-up layout takes the
        // remaining space and pushes the label to the lower edge.
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            let scrcpy = self
                .tools
                .as_ref()
                .map_or("—", |tools| tools.version.as_str());
            ui.label(
                egui::RichText::new(format!("{APP_NAME} v{VERSION}  ·  scrcpy {scrcpy}"))
                    .color(FAINT)
                    .small(),
            );
        });
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let live = self.state == State::Streaming;
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("PREVIEW").color(MUTED).small().strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let crop_button = egui::Button::new("Crop").fill(if self.crop_mode {
                    ACCENT
                } else {
                    egui::Color32::TRANSPARENT
                });
                if ui.add_enabled(live, crop_button).clicked() {
                    self.crop_mode = !self.crop_mode;
                    self.crop_drag = None;
                }
                let can_reset = self.crop != FULL_CROP;
                if ui
                    .add_enabled(can_reset, egui::Button::new("Reset"))
                    .clicked()
                {
                    self.crop = FULL_CROP;
                }
                if live {
                    let decode = self
                        .capture
                        .as_ref()
                        .map_or(0.0, capture::Capture::decode_fps);
                    let text = if self.crop_mode {
                        "Drag the edges to crop".to_string()
                    } else {
                        format!(
                            "{}×{}  ·  {:.0} fps  ·  decode {:.0} fps",
                            self.source_size[0], self.source_size[1], self.paint_fps, decode
                        )
                    };
                    ui.label(egui::RichText::new(text).color(FAINT).small());
                }
            });
        });
        ui.add_space(6.0);

        // Crop mode shows the whole frame so the user can see what is being cut away.
        let cropping = self.crop_mode && live;
        let sense = if cropping {
            egui::Sense::drag()
        } else {
            egui::Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), sense);
        ui.painter().rect_filled(rect, 10.0, VIDEO_BG);

        let Some(texture) = &self.texture else {
            // With no phone, say so here rather than only in the collapsed log: a wrong USB
            // mode is the most common first-run problem and the hint has to be visible.
            let (title, hints): (&str, &[&str]) = match self.state {
                State::Connecting => ("Connecting...", &[]),
                State::Streaming => ("Waiting for the first frame...", &[]),
                State::Idle if self.device.is_none() => (
                    "No phone connected",
                    &[
                        "This app talks to the phone over adb, so Developer options and",
                        "USB debugging have to be turned on. There is no way around that:",
                        "it is what lets us work without installing anything on the phone.",
                        "",
                        "Then pull down the phone's USB notification and choose File transfer",
                        "(MTP) or Photo transfer (PTP) rather than charging only. Some cables",
                        "carry power but no data.",
                    ],
                ),
                State::Idle => ("No video", &["Press Start Stream."]),
            };
            let mut y = rect.center().y - 10.0 * hints.len() as f32;
            ui.painter().text(
                egui::pos2(rect.center().x, y),
                egui::Align2::CENTER_CENTER,
                title,
                egui::FontId::proportional(16.0),
                MUTED,
            );
            for hint in hints {
                y += 22.0;
                ui.painter().text(
                    egui::pos2(rect.center().x, y),
                    egui::Align2::CENTER_CENTER,
                    hint,
                    egui::FontId::proportional(12.0),
                    FAINT,
                );
            }
            return;
        };
        let texture_id = texture.id();

        // Aspect shown on screen: the whole frame in crop mode, the cropped area otherwise.
        let sideways = self.rotation % 2 == 1;
        let (src_w, src_h) = (self.source_size[0] as f32, self.source_size[1] as f32);
        let (full_w, full_h) = if sideways {
            (src_h, src_w)
        } else {
            (src_w, src_h)
        };
        let (shown_w, shown_h) = if cropping {
            (full_w, full_h)
        } else {
            (full_w * self.crop.width(), full_h * self.crop.height())
        };
        let scale = (rect.width() / shown_w).min(rect.height() / shown_h);
        let image_rect = egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(shown_w * scale, shown_h * scale),
        );

        // Area to display: the whole frame in crop mode, otherwise crop plus zoom.
        let shown_area = if cropping {
            FULL_CROP
        } else {
            zoomed(self.crop, self.zoom)
        };
        let uv = shown_rect_to_texture(shown_area, self.rotation, self.mirror);
        draw_frame(
            ui.painter(),
            image_rect,
            texture_id,
            uv,
            self.rotation,
            self.mirror,
        );

        if cropping {
            self.handle_crop_drag(ui, &response, image_rect);
            draw_crop_overlay(ui.painter(), image_rect, self.crop);
        }
    }

    /// Handles dragging the crop frame's edges, corners and interior.
    fn handle_crop_drag(&mut self, ui: &egui::Ui, response: &egui::Response, image: egui::Rect) {
        let frame = screen_crop_rect(image, self.crop);

        if response.drag_started() {
            // The handle is chosen from where the button went down, not where the pointer
            // is now: egui only reports a drag after a few pixels of movement, which was
            // enough to miss a corner handle and fall through to moving the frame.
            let origin = ui.ctx().input(|input| input.pointer.press_origin());
            self.crop_drag = origin
                .or_else(|| response.interact_pointer_pos())
                .and_then(|pos| crop_hit(frame, pos));
        }
        if response.drag_stopped() {
            self.crop_drag = None;
        }

        // Let the cursor reflect the handle underneath it.
        if let Some(pos) = response.hover_pos() {
            let cursor = self
                .crop_drag
                .or_else(|| crop_hit(frame, pos))
                .map_or(egui::CursorIcon::Default, CropDrag::cursor);
            ui.ctx().set_cursor_icon(cursor);
        }

        let Some(drag) = self.crop_drag else { return };
        let delta = response.drag_delta();
        let (dx, dy) = (delta.x / image.width(), delta.y / image.height());
        let mut crop = self.crop;

        if drag.is_move() {
            // No edge was grabbed: move the frame, keeping it inside the picture.
            let dx = dx.clamp(-crop.min.x, 1.0 - crop.max.x);
            let dy = dy.clamp(-crop.min.y, 1.0 - crop.max.y);
            crop = crop.translate(egui::vec2(dx, dy));
        } else {
            if drag.left {
                crop.min.x = (crop.min.x + dx).clamp(0.0, crop.max.x - MIN_CROP);
            }
            if drag.right {
                crop.max.x = (crop.max.x + dx).clamp(crop.min.x + MIN_CROP, 1.0);
            }
            if drag.top {
                crop.min.y = (crop.min.y + dy).clamp(0.0, crop.max.y - MIN_CROP);
            }
            if drag.bottom {
                crop.max.y = (crop.max.y + dy).clamp(crop.min.y + MIN_CROP, 1.0);
            }
        }
        self.crop = crop;
    }

    fn log_panel(&mut self, ui: &mut egui::Ui) {
        let unseen = self.log.len();
        let title = if self.log_open {
            "LOG".to_string()
        } else {
            format!("LOG  ({unseen})")
        };
        let response = egui::CollapsingHeader::new(egui::RichText::new(title).small().strong())
            .open(Some(self.log_open))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(140.0)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for (line, color) in &self.log {
                            ui.label(
                                egui::RichText::new(line)
                                    .monospace()
                                    .size(11.0)
                                    .color(*color),
                            );
                        }
                    });
            });
        if response.header_response.clicked() {
            self.log_open = !self.log_open;
        }
    }
}

/// Zooms into the centre of the cropped area; the zoom is a percentage.
fn zoomed(crop: egui::Rect, zoom: u32) -> egui::Rect {
    let span = 100.0 / zoom as f32;
    egui::Rect::from_center_size(crop.center(), crop.size() * span)
}

/// Converts a normalised on-screen point into texture coordinates.
///
/// The mesh applies mirroring by swapping texture corners horizontally and rotation by
/// shifting them. Here both are undone in reverse order, rotation first then mirroring, so
/// the user can drag the frame in the orientation they actually see.
fn shown_to_texture(point: egui::Pos2, rotation: usize, mirror: bool) -> egui::Pos2 {
    let (mut u, v) = match rotation % 4 {
        1 => (point.y, 1.0 - point.x),
        2 => (1.0 - point.x, 1.0 - point.y),
        3 => (1.0 - point.y, point.x),
        _ => (point.x, point.y),
    };
    if mirror {
        u = 1.0 - u;
    }
    egui::pos2(u, v)
}

/// Converts a rectangle in on-screen orientation into one in texture coordinates.
/// The transforms are multiples of 90 degrees plus a flip, so the result stays axis-aligned.
fn shown_rect_to_texture(rect: egui::Rect, rotation: usize, mirror: bool) -> egui::Rect {
    let a = shown_to_texture(rect.min, rotation, mirror);
    let b = shown_to_texture(rect.max, rotation, mirror);
    egui::Rect::from_min_max(
        egui::pos2(a.x.min(b.x), a.y.min(b.y)),
        egui::pos2(a.x.max(b.x), a.y.max(b.y)),
    )
}

/// The crop rectangle in screen pixels.
fn screen_crop_rect(image: egui::Rect, crop: egui::Rect) -> egui::Rect {
    egui::Rect::from_min_max(
        image.min + egui::vec2(crop.min.x * image.width(), crop.min.y * image.height()),
        image.min + egui::vec2(crop.max.x * image.width(), crop.max.y * image.height()),
    )
}

/// The crop overlay: everything outside is dimmed and handles appear on the edges.
fn draw_crop_overlay(painter: &egui::Painter, image: egui::Rect, crop: egui::Rect) {
    let frame = screen_crop_rect(image, crop);
    let shade = egui::Color32::from_black_alpha(140);
    // The four regions that will be cut away: top, bottom, left, right.
    for area in [
        egui::Rect::from_min_max(image.min, egui::pos2(image.max.x, frame.min.y)),
        egui::Rect::from_min_max(egui::pos2(image.min.x, frame.max.y), image.max),
        egui::Rect::from_min_max(egui::pos2(image.min.x, frame.min.y), frame.left_bottom()),
        egui::Rect::from_min_max(frame.right_top(), egui::pos2(image.max.x, frame.max.y)),
    ] {
        if area.is_positive() {
            painter.rect_filled(area, 0.0, shade);
        }
    }
    painter.rect_stroke(
        frame,
        0.0,
        egui::Stroke::new(2.0, ACCENT),
        egui::StrokeKind::Inside,
    );
    // Rule-of-thirds guides.
    let guide = egui::Stroke::new(1.0, egui::Color32::from_white_alpha(70));
    for i in 1..3 {
        let t = i as f32 / 3.0;
        let x = frame.min.x + frame.width() * t;
        let y = frame.min.y + frame.height() * t;
        painter.line_segment(
            [egui::pos2(x, frame.min.y), egui::pos2(x, frame.max.y)],
            guide,
        );
        painter.line_segment(
            [egui::pos2(frame.min.x, y), egui::pos2(frame.max.x, y)],
            guide,
        );
    }
    // Handles are drawn from the same source as hit testing, so the two cannot drift apart.
    for (point, _) in crop_handles(frame) {
        painter.rect_filled(
            egui::Rect::from_center_size(point, egui::vec2(10.0, 10.0)),
            2.0,
            egui::Color32::WHITE,
        );
    }
}

/// Draws the frame as a four-corner mesh. Rotation and mirroring come from reordering the
/// texture coordinates assigned to the corners, which costs nothing on the GPU.
fn draw_frame(
    painter: &egui::Painter,
    rect: egui::Rect,
    texture: egui::TextureId,
    uv: egui::Rect,
    rotation: usize,
    mirror: bool,
) {
    let mut uvs = [
        uv.left_top(),
        uv.right_top(),
        uv.right_bottom(),
        uv.left_bottom(),
    ];
    if mirror {
        uvs.swap(0, 1);
        uvs.swap(2, 3);
    }
    uvs.rotate_right(rotation % 4);

    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let mut mesh = egui::Mesh::with_texture(texture);
    for (position, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos: position,
            uv,
            color: egui::Color32::WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}

fn first_sentence(text: &str) -> String {
    text.split_once(". ")
        .map_or_else(|| text.to_string(), |(head, _)| head.to_string())
}

impl eframe::App for App {
    // egui 0.37 replaced update(ctx) with ui(ui), and panels take a Ui, not a Context.
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Published every frame: it is cheap, and it avoids having to catch each individual
        // settings change, crop dragging included.
        self.publish_transform();
        self.poll_webcam_setup();
        self.poll_jobs();
        self.pump_capture(&ctx);

        egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(8.0);
            self.header(ui);
            ui.add_space(8.0);
        });
        egui::Panel::bottom("log").show(ui, |ui| {
            self.log_panel(ui);
        });
        egui::Panel::left("sidebar")
            .exact_size(260.0)
            .show(ui, |ui| {
                ui.add_space(10.0);
                self.sidebar(ui);
            });
        egui::CentralPanel::default().show(ui, |ui| {
            ui.add_space(10.0);
            self.preview(ui);
        });
    }

    /// Shuts everything down before the window closes.
    ///
    /// The capture thread and the virtual camera stop when their owners are dropped; adb's
    /// background server does not, so it is stopped explicitly here.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.capture = None;
        self.vcam = None;
        if !self.adb_was_running
            && let Some(tools) = &self.tools
        {
            tools.kill_server();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let [r, g, b, a] = BG.to_array();
        [
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
            f32::from(a) / 255.0,
        ]
    }
}
