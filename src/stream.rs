//! Starts scrcpy-server on the phone with the camera as its source and reads H.264 packets.
//!
//! The stream format changed in scrcpy 5.x, so the metadata that varies between versions
//! (device name, codec and session headers) is switched off with `raw_stream=true`. Only
//! the two things that are identical from 2.2 through 5.x are switched back on: the
//! connection check byte and the 12-byte packet header (8 bytes pts+flags, 4 bytes size,
//! big-endian).

use crate::adb::{REMOTE_SERVER, SERVER_CLASS, Tools};
use anyhow::{Context as _, Result, bail};
use std::io::{BufRead, BufReader, ErrorKind, Read};
use std::net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

const LOCAL_PORT: u16 = 27183;
const HEADER_LEN: usize = 12;
const MAX_PACKET: u32 = 64 * 1024 * 1024;
/// Socket reads time out this often so a stop request can be noticed. With a blocking read,
/// pressing Stop on a camera that never delivered a frame froze the user interface.
const READ_TIMEOUT: Duration = Duration::from_millis(300);
/// Bit 63 is session, bit 62 configuration (SPS/PPS), bit 61 key frame.
const FLAG_CONFIG: u64 = 1 << 62;

/// Carries the phone-side server's log lines to the user interface.
pub type Logger = Arc<dyn Fn(String) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct Config {
    pub camera_id: String,
    pub max_size: u32,
    pub bit_rate: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            camera_id: "0".into(),
            max_size: 1920,
            bit_rate: 8_000_000,
        }
    }
}

/// One H.264 packet read from the stream.
pub struct Packet {
    pub data: Vec<u8>,
    /// When true this is decoder configuration (SPS/PPS) rather than a frame.
    pub is_config: bool,
}

pub struct Stream {
    tools: Tools,
    serial: String,
    server: Child,
    socket: TcpStream,
    stop: Arc<AtomicBool>,
}

impl Stream {
    pub fn start(
        tools: &Tools,
        serial: &str,
        config: &Config,
        log: Logger,
        stop: Arc<AtomicBool>,
    ) -> Result<Self> {
        // A 31-bit session id avoids clashing with another scrcpy running on the phone.
        let scid = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() & 0x7fff_ffff)
            .unwrap_or(0x1234_5678);

        tools.push_server(serial)?;
        tools.adb(
            Some(serial),
            &[
                "forward",
                &format!("tcp:{LOCAL_PORT}"),
                &format!("localabstract:scrcpy_{scid:08x}"),
            ],
        )?;

        match Self::spawn_and_connect(tools, serial, scid, config, log, &stop) {
            Ok((server, socket)) => Ok(Self {
                tools: tools.clone(),
                serial: serial.to_string(),
                server,
                socket,
                stop,
            }),
            Err(err) => {
                remove_forward(tools, serial);
                Err(err)
            }
        }
    }

    fn spawn_and_connect(
        tools: &Tools,
        serial: &str,
        scid: u32,
        config: &Config,
        log: Logger,
        stop: &AtomicBool,
    ) -> Result<(Child, TcpStream)> {
        let classpath = format!("CLASSPATH={REMOTE_SERVER}");
        let mut cmd = Command::new(&tools.adb);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut server = cmd
            .args([
                "-s",
                serial,
                "shell",
                &classpath,
                "app_process",
                "/",
                SERVER_CLASS,
            ])
            .args(server_args(scid, &tools.version, config))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("could not start scrcpy-server through adb")?;

        // The server's own log: our only source of information when something fails.
        for pipe in [
            server.stdout.take().map(PipeKind::Out),
            server.stderr.take().map(PipeKind::Err),
        ]
        .into_iter()
        .flatten()
        {
            let log = log.clone();
            std::thread::spawn(move || pump_log(pipe, log));
        }

        match connect_with_dummy_byte(&mut server, Duration::from_secs(15), stop) {
            Ok(socket) => Ok((server, socket)),
            Err(err) => {
                let _ = server.kill();
                let _ = server.wait();
                Err(err)
            }
        }
    }

    /// Reads the next packet. Returns `Ok(None)` when a stop is requested, so the thread
    /// exits within `READ_TIMEOUT` even on a camera that never delivers a frame.
    pub fn read_packet(&mut self) -> Result<Option<Packet>> {
        let mut header = [0u8; HEADER_LEN];
        if !read_fully(&mut self.socket, &mut header, &self.stop)? {
            return Ok(None);
        }
        let pts_and_flags = u64::from_be_bytes(header[0..8].try_into().expect("8 bayt"));
        let size = u32::from_be_bytes(header[8..12].try_into().expect("4 bayt"));
        if size > MAX_PACKET {
            bail!("invalid packet size: {size}");
        }
        let mut data = vec![0u8; size as usize];
        if !read_fully(&mut self.socket, &mut data, &self.stop)? {
            return Ok(None);
        }
        Ok(Some(Packet {
            data,
            is_config: pts_and_flags & FLAG_CONFIG != 0,
        }))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
        let _ = self.server.kill();
        let _ = self.server.wait();
        remove_forward(&self.tools, &self.serial);
    }
}

/// Options passed to the server. Order matters: `raw_stream=true` turns off every `send_*`
/// option, so the ones we want back come after it.
fn server_args(scid: u32, version: &str, config: &Config) -> Vec<String> {
    vec![
        version.to_string(),
        format!("scid={scid:08x}"),
        "log_level=info".into(),
        "video_source=camera".into(),
        "video_codec=h264".into(),
        "audio=false".into(),
        "control=false".into(),
        // The phone listens and the computer connects through adb forward.
        "tunnel_forward=true".into(),
        // Leaves no file behind on the phone when the server exits.
        "cleanup=true".into(),
        format!("camera_id={}", config.camera_id),
        format!("max_size={}", config.max_size),
        format!("video_bit_rate={}", config.bit_rate),
        "raw_stream=true".into(),
        "send_dummy_byte=true".into(),
        "send_frame_meta=true".into(),
    ]
}

enum PipeKind {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

fn pump_log(pipe: PipeKind, log: Logger) {
    let reader: Box<dyn Read + Send> = match pipe {
        PipeKind::Out(out) => Box::new(out),
        PipeKind::Err(err) => Box::new(err),
    };
    for line in BufReader::new(reader).lines().map_while(Result::ok) {
        let line = line.trim_end().to_string();
        if !line.is_empty() {
            log(line);
        }
    }
}

/// adb forward accepts a connection even before the phone-side server is listening, so we
/// retry until the one-byte dummy byte the server sends actually arrives.
fn connect_with_dummy_byte(
    server: &mut Child,
    timeout: Duration,
    stop: &AtomicBool,
) -> Result<TcpStream> {
    let deadline = Instant::now() + timeout;
    let addr = SocketAddrV4::new(Ipv4Addr::LOCALHOST, LOCAL_PORT);
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            bail!("cancelled");
        }
        if let Some(status) = server.try_wait()? {
            bail!("scrcpy-server exited on the phone ({status}); see the [server] log lines");
        }
        if let Ok(mut socket) = TcpStream::connect_timeout(&addr.into(), Duration::from_secs(2)) {
            socket.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut byte = [0u8; 1];
            if matches!(socket.read(&mut byte), Ok(1)) {
                socket.set_read_timeout(Some(READ_TIMEOUT))?;
                socket.set_nodelay(true)?;
                return Ok(socket);
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    bail!("could not connect to the server on the phone (timed out)")
}

/// Fills the buffer. Returns `Ok(false)` when a stop is requested.
fn read_fully(socket: &mut TcpStream, buf: &mut [u8], stop: &AtomicBool) -> Result<bool> {
    let mut filled = 0;
    while filled < buf.len() {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        match socket.read(&mut buf[filled..]) {
            Ok(0) => bail!("the phone closed the connection"),
            Ok(n) => filled += n,
            // Timeouts are expected: check the stop flag and keep reading.
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {}
            Err(err) => return Err(err).context("reading from the phone"),
        }
    }
    Ok(true)
}

fn remove_forward(tools: &Tools, serial: &str) {
    let _ = tools.adb(
        Some(serial),
        &["forward", "--remove", &format!("tcp:{LOCAL_PORT}")],
    );
}
