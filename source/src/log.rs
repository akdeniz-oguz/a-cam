//! The DLL's own log.
//!
//! This code runs inside the Windows Frame Server service: it has no console and no
//! connection to our application. Writing to a file is the only way to see which methods
//! Windows calls and where things stop.

use std::fmt::Write as _;
use std::io::Write as _;

/// Each process writes to its own file.
///
/// A single shared file was misleading: once our user account created it, the Frame Server
/// running as Local Service could not append to it and the log silently stayed empty. With
/// the process id in the name every process creates its own file, and a missing file is
/// itself the answer — the DLL was never loaded in that process.
///
/// Several locations are tried in turn because the Frame Server runs under a restricted
/// account that may not be able to write to `C:\Windows\Temp`.
fn candidate_paths() -> [String; 3] {
    let pid = std::process::id();
    [
        format!(r"C:\Users\Public\a-cam-source-{pid}.log"),
        format!(r"C:\Windows\Temp\a-cam-source-{pid}.log"),
        format!("{}\\a-cam-source-{pid}.log", std::env::temp_dir().display()),
    ]
}

/// Appends one line to the log. Errors are swallowed: logging must never disturb the stream.
pub fn line(message: &str) {
    // The process name goes in too, so it is clear whether the DLL is running inside our
    // own executable or inside the Frame Server service (svchost).
    let process = std::env::current_exe()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "?".into());
    let mut text = String::new();
    let _ = writeln!(text, "[{} {}] {message}", std::process::id(), process);

    for path in candidate_paths() {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            && file.write_all(text.as_bytes()).is_ok()
        {
            return;
        }
    }
}
