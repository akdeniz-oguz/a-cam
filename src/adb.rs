//! The layer that talks to adb and scrcpy-server.
//!
//! No APK is installed on the phone: `scrcpy-server` is copied to `/data/local/tmp` and run
//! with Android's own `app_process` command. Thanks to `cleanup=true` the server deletes its
//! own file when it exits.

use anyhow::{Context as _, Result, anyhow, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Temporary server file on the phone.
pub const REMOTE_SERVER: &str = "/data/local/tmp/a-cam-server.jar";
pub const SERVER_CLASS: &str = "com.genymobile.scrcpy.Server";

/// Keeps adb from flashing a console window on every call.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Port the adb background server listens on.
const ADB_SERVER_PORT: u16 = 5037;

/// Reports whether an adb server is already running.
///
/// adb starts a background server on first use and leaves it running afterwards. We check
/// before touching adb so that on exit we only stop a server we started ourselves: killing
/// one that Android Studio or another tool owns would disrupt it.
pub fn server_running() -> bool {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
    use std::time::Duration;
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, ADB_SERVER_PORT);
    TcpStream::connect_timeout(&address.into(), Duration::from_millis(300)).is_ok()
}

fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Tools located in the scrcpy folder.
#[derive(Clone, Debug)]
pub struct Tools {
    pub adb: PathBuf,
    pub server: PathBuf,
    /// The server refuses to run unless the client reports exactly its own version.
    pub version: String,
}

impl Tools {
    /// Looks for the scrcpy files next to the exe or in the working directory.
    pub fn find() -> Result<Self> {
        // A normal build carries adb and scrcpy-server, so there is nothing to look for:
        // they are written to a per-user cache the first time they are needed.
        if crate::bundled::AVAILABLE {
            let dir = crate::bundled::extract()?;
            return Ok(Self {
                adb: dir.join(exe_name("adb")),
                server: dir.join("scrcpy-server"),
                version: crate::bundled::version().to_string(),
            });
        }

        let mut roots = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            // During development we run from target/debug, so look at the project root too.
            for up in [1usize, 2, 3] {
                let mut dir = exe.clone();
                for _ in 0..up {
                    dir.pop();
                }
                roots.push(dir.join("scrcpy"));
            }
        }
        roots.push(PathBuf::from("scrcpy"));

        let base = roots
            .iter()
            .find(|dir| dir.join("scrcpy-server").is_file())
            .ok_or_else(|| {
                anyhow!(
                    "scrcpy files not found. Extract the scrcpy Windows zip into a folder named \
                     'scrcpy'. Looked in: {}",
                    roots
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;

        let adb = base.join(exe_name("adb"));
        if !adb.is_file() {
            bail!("adb not found at {}", adb.display());
        }
        let scrcpy = base.join(exe_name("scrcpy"));
        if !scrcpy.is_file() {
            bail!(
                "{} not found; it is needed to read the server version",
                scrcpy.display()
            );
        }
        Ok(Self {
            adb,
            server: base.join("scrcpy-server"),
            version: read_version(&scrcpy)?,
        })
    }

    /// Runs `adb <args>` and returns stdout and stderr together.
    pub fn adb(&self, serial: Option<&str>, args: &[&str]) -> Result<String> {
        let mut cmd = command(&self.adb);
        if let Some(serial) = serial {
            cmd.args(["-s", serial]);
        }
        let out = cmd
            .args(args)
            .stdin(Stdio::null())
            .output()
            .with_context(|| format!("could not run adb {}", args.join(" ")))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            bail!("adb {} failed: {}", args.join(" "), text.trim());
        }
        Ok(text)
    }

    /// Stops the adb background server.
    ///
    /// Without this the server outlives the application and keeps showing up in Task
    /// Manager after the window is closed.
    pub fn kill_server(&self) {
        let _ = self.adb(None, &["kill-server"]);
    }

    /// Returns the first ready (authorised) device.
    pub fn find_device(&self) -> Result<Device> {
        let text = self.adb(None, &["devices"])?;
        let mut ready = Vec::new();
        let mut unauthorized = false;
        for line in text.lines().skip(1) {
            let mut parts = line.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some(serial), Some("device")) => ready.push(serial.to_string()),
                (Some(_), Some("unauthorized")) => unauthorized = true,
                _ => {}
            }
        }
        let serial = match ready.first() {
            Some(serial) => serial.clone(),
            None if unauthorized => bail!(
                "The phone has not authorized this computer. Accept the 'Allow USB debugging?' \
                 prompt on the phone."
            ),
            // The USB mode matters as much as the cable: in "charge only" the phone never
            // appears to adb at all, and that is the most common cause of this message.
            None => bail!(
                "No phone found. Developer options and USB debugging must be enabled on the \
                 phone: this app drives it over adb, which is what lets it work without \
                 installing anything on the phone. Then pull down the phone's USB \
                 notification and set the connection to File transfer (MTP) or Photo \
                 transfer (PTP) rather than charging only. Some cables carry power but no \
                 data."
            ),
        };
        let model = self
            .adb(Some(&serial), &["shell", "getprop", "ro.product.model"])
            .map(|out| out.trim().replace('_', " "))
            .unwrap_or_default();
        let others = ready.len().saturating_sub(1);
        Ok(Device {
            model: if model.is_empty() {
                serial.clone()
            } else {
                model
            },
            serial,
            others,
        })
    }

    /// Copies the server to the phone; called before every stream and camera listing.
    pub fn push_server(&self, serial: &str) -> Result<()> {
        self.adb(
            Some(serial),
            &["push", &self.server.to_string_lossy(), REMOTE_SERVER],
        )?;
        Ok(())
    }

    /// Lists the phone's cameras with their ids and resolutions.
    pub fn list_cameras(&self, serial: &str) -> Result<Vec<Camera>> {
        self.push_server(serial)?;
        let classpath = format!("CLASSPATH={REMOTE_SERVER}");
        // list_cameras prints the list and exits; read the output even on a non-zero code.
        let text = self
            .adb(
                Some(serial),
                &[
                    "shell",
                    &classpath,
                    "app_process",
                    "/",
                    SERVER_CLASS,
                    &self.version,
                    "log_level=info",
                    "list_cameras=true",
                ],
            )
            .unwrap_or_else(|err| err.to_string());

        let cameras = main_cameras(text.lines().filter_map(parse_camera_line).collect());
        if cameras.is_empty() {
            let tail: Vec<&str> = text.lines().rev().take(4).collect();
            bail!(
                "The phone did not return a camera list (Android 12+ is required). {}",
                tail.into_iter().rev().collect::<Vec<_>>().join(" | ")
            );
        }
        Ok(cameras)
    }
}

#[derive(Clone, Debug)]
pub struct Device {
    pub serial: String,
    pub model: String,
    /// How many other ready devices are connected.
    pub others: usize,
}

#[derive(Clone, Debug)]
pub struct Camera {
    pub id: String,
    pub facing: String,
    pub width: u32,
    pub height: u32,
}

impl Camera {
    /// Label for the dropdown, e.g. "Back Camera (ID 0) · 4640x3472".
    pub fn label(&self) -> String {
        let facing = match self.facing.as_str() {
            "back" => "Back Camera",
            "front" => "Front Camera",
            "external" => "External Camera",
            _ => "Camera",
        };
        format!("{facing} (ID {}) · {}x{}", self.id, self.width, self.height)
    }
}

/// Keeps only the first (main) camera for each facing.
///
/// Phones advertise logical and auxiliary cameras alongside the main ones (20, 21, 61, 100
/// and so on). Most of them cannot be opened for capture: the stream starts and the phone
/// immediately closes the connection, so the list is trimmed to what actually works.
fn main_cameras(cameras: Vec<Camera>) -> Vec<Camera> {
    let mut seen: Vec<String> = Vec::new();
    cameras
        .into_iter()
        .filter(|camera| {
            let first = !seen.contains(&camera.facing);
            if first {
                seen.push(camera.facing.clone());
            }
            first
        })
        .collect()
}

/// Parses one line of scrcpy-server output:
/// `    --camera-id=0    (back, 4640x3472, fps={15, 24, 30}, zoom-range=[1, 10])`
fn parse_camera_line(line: &str) -> Option<Camera> {
    let rest = line.split_once("--camera-id=")?.1;
    let (id, rest) = rest.split_once('(')?;
    let inside = rest.split_once(')').map_or(rest, |(head, _)| head);
    let mut fields = inside.split(',').map(str::trim);
    let facing = fields.next()?;
    let (width, height) = fields.next()?.split_once('x')?;
    Some(Camera {
        id: id.trim().to_string(),
        facing: facing.to_string(),
        width: width.trim().parse().ok()?,
        height: height.trim().parse().ok()?,
    })
}

/// Reads the version from `scrcpy --version`; the server demands an exact match.
fn read_version(scrcpy: &Path) -> Result<String> {
    let out = command(scrcpy)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("could not run {}", scrcpy.display()))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    text.lines()
        .find_map(|line| line.strip_prefix("scrcpy "))
        .map(|rest| {
            rest.split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('v')
        })
        .filter(|version| !version.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("could not read the scrcpy version"))
}

fn exe_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
}
