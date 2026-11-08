//! adb and scrcpy-server carried inside the executable.
//!
//! Keeping them in the binary is what makes a-Cam a single file: there is no folder to
//! keep beside it and nothing to unpack by hand. They cannot simply be run from memory
//! though — adb is a separate process and scrcpy-server is pushed to the phone as a file —
//! so on first use they are written to a per-user cache directory.
//!
//! The directory is named after the scrcpy version, so upgrading a-Cam writes a fresh set
//! rather than mixing files from two releases. scrcpy's licence is written alongside them,
//! which Apache-2.0 asks for when its binaries are redistributed.

use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};

/// Whether this build carries the tools. False when it was built without a scrcpy folder,
/// in which case a-Cam looks for one on disk instead.
pub const AVAILABLE: bool = cfg!(embedded_scrcpy);

#[cfg(embedded_scrcpy)]
mod payload {
    macro_rules! file {
        ($name:literal) => {
            (
                $name,
                include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/scrcpy/", $name)).as_slice(),
            )
        };
    }

    pub const VERSION: &str = env!("ACAM_SCRCPY_VERSION");

    pub const FILES: [(&str, &[u8]); 5] = [
        file!("adb.exe"),
        // adb's only non-system dependency, which in turn needs the USB one.
        file!("AdbWinApi.dll"),
        file!("AdbWinUsbApi.dll"),
        file!("scrcpy-server"),
        file!("LICENSE.txt"),
    ];
}

#[cfg(not(embedded_scrcpy))]
mod payload {
    pub const VERSION: &str = "";
    pub const FILES: [(&str, &[u8]); 0] = [];
}

/// The scrcpy version these files came from.
pub fn version() -> &'static str {
    payload::VERSION
}

/// Writes the tools out if they are not already there, and returns the directory.
///
/// Extraction is skipped when every file is already present at the right size, so this is
/// cheap on every run after the first.
pub fn extract() -> Result<PathBuf> {
    let base =
        dirs_local_app_data().context("could not locate the local application data folder")?;
    let dir = base
        .join("a-Cam")
        .join(format!("scrcpy-{}", payload::VERSION));

    if payload::FILES
        .iter()
        .all(|(name, bytes)| is_current(&dir.join(name), bytes))
    {
        return Ok(dir);
    }

    std::fs::create_dir_all(&dir).with_context(|| format!("could not create {}", dir.display()))?;
    for (name, bytes) in payload::FILES {
        let path = dir.join(name);
        if is_current(&path, bytes) {
            continue;
        }
        // A running adb.exe cannot be overwritten; writing beside it and renaming keeps a
        // half-written file from being left behind if that happens.
        let temporary = path.with_extension("partial");
        std::fs::write(&temporary, bytes)
            .with_context(|| format!("could not write {}", temporary.display()))?;
        std::fs::rename(&temporary, &path)
            .with_context(|| format!("could not place {}", path.display()))?;
    }
    Ok(dir)
}

fn is_current(path: &Path, bytes: &[u8]) -> bool {
    std::fs::metadata(path).is_ok_and(|data| data.len() == bytes.len() as u64)
}

fn dirs_local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}
