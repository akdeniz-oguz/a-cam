//! Build-time setup: the application icon, and static linking of the C++ runtime.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.png");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    link_libstdcpp_statically();
    bundle_scrcpy();
    bundle_source_dll();
    embed_icon();
}

/// Builds the virtual camera source and points the crate at the resulting DLL, so the
/// executable can carry it and install it on demand.
///
/// The DLL is a separate workspace member that nothing depends on, so cargo has no reason
/// to build it first. It is built here instead. The nested build needs its own target
/// directory: cargo holds a lock on the one it is already using, and sharing it would
/// deadlock.
fn bundle_source_dll() {
    println!("cargo::rustc-check-cfg=cfg(embedded_source)");
    println!("cargo:rerun-if-changed=source/src");
    println!("cargo:rerun-if-changed=source/Cargo.toml");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let target_dir = out_dir.join("source-build");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    let built = Command::new(cargo)
        .args([
            "build",
            "--release",
            "--package",
            "a-cam-source",
            "--target-dir",
        ])
        .arg(&target_dir)
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    let dll = target_dir.join("release").join("a_cam_source.dll");
    if !built || !dll.is_file() {
        println!(
            "cargo:warning=a_cam_source.dll could not be built, so the webcam feature is \
             not included in this binary."
        );
        return;
    }
    println!("cargo:rustc-env=ACAM_SOURCE_DLL={}", dll.display());
    println!("cargo:rustc-cfg=embedded_source");
}

/// Files taken from a scrcpy release and carried inside the executable.
///
/// Only these are needed at runtime. The rest of a scrcpy release — its own GUI, SDL and
/// the FFmpeg libraries, about 16 MB — belongs to the scrcpy application itself, which we
/// never run.
const BUNDLED: [&str; 5] = [
    "adb.exe",
    "AdbWinApi.dll",
    "AdbWinUsbApi.dll",
    "scrcpy-server",
    "LICENSE.txt",
];

/// Lets the executable carry adb and scrcpy-server instead of needing a folder beside it.
///
/// The files are only embedded when a scrcpy folder is present at build time; without one
/// the binary still works and falls back to looking for the folder on disk. `cfg` and the
/// version are reported to the crate so it knows which of the two it was built with.
fn bundle_scrcpy() {
    println!("cargo::rustc-check-cfg=cfg(embedded_scrcpy)");

    let folder = Path::new("scrcpy");
    for file in BUNDLED {
        println!("cargo:rerun-if-changed=scrcpy/{file}");
    }
    println!("cargo:rerun-if-changed=scrcpy/version.txt");
    println!("cargo:rerun-if-changed=scrcpy/scrcpy.exe");

    let missing: Vec<&str> = BUNDLED
        .iter()
        .copied()
        .filter(|file| !folder.join(file).is_file())
        .collect();
    if !missing.is_empty() {
        println!(
            "cargo:warning=scrcpy/ is incomplete ({}), so the tools are not embedded. \
             Run tools/fetch-scrcpy.ps1 for a self-contained build.",
            missing.join(", ")
        );
        return;
    }

    // The server refuses to run unless the client reports exactly its own version, so it
    // is read here and compiled in.
    let Some(version) = scrcpy_version(folder) else {
        println!("cargo:warning=could not read the scrcpy version; tools are not embedded");
        return;
    };

    println!("cargo:rustc-env=ACAM_SCRCPY_VERSION={version}");
    println!("cargo:rustc-cfg=embedded_scrcpy");
}

/// The version of the scrcpy release sitting in `folder`.
///
/// `tools/fetch-scrcpy.ps1` leaves it in a file, which is the preferred source: it costs
/// nothing and works on a machine that cannot run a windowed binary. A folder extracted by
/// hand will not have that file, so as a fallback scrcpy.exe is asked directly. That is the
/// only reason scrcpy.exe is needed at build time; it is never run afterwards.
fn scrcpy_version(folder: &Path) -> Option<String> {
    let recorded = std::fs::read_to_string(folder.join("version.txt"))
        .ok()
        .map(|text| text.trim().trim_start_matches('v').to_string())
        .filter(|version| !version.is_empty());
    if let Some(version) = recorded {
        return Some(version);
    }

    let output = Command::new(folder.join("scrcpy.exe"))
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    text.lines()
        .find_map(|line| line.strip_prefix("scrcpy "))
        .and_then(|rest| rest.split_whitespace().next())
        .map(|version| version.trim_start_matches('v').to_string())
        .filter(|version| !version.is_empty())
}

/// Makes the executable carry the C++ runtime instead of loading it from a DLL.
///
/// The H.264 decoder is C++, so the binary pulls in libstdc++. By default the linker finds
/// the import library (`libstdc++.dll.a`) sitting next to the static archive and prefers
/// it, which leaves the executable needing `libstdc++-6.dll` — and that DLL in turn needs
/// `libgcc_s_seh-1.dll` and `libwinpthread-1.dll`, so three files have to travel with it.
///
/// Linker flags alone do not fix this: `-static` and `-static-libstdc++` are both ignored
/// here, because the library is requested as a plain `-lstdc++` by the decoder's build
/// script and rustc invokes gcc rather than g++. What does work is search order. A `-L`
/// directory is searched before the compiler's own, so a directory holding only the static
/// archive leaves the linker nothing else to find.
fn link_libstdcpp_statically() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let archive = match Command::new("gcc")
        .arg("-print-file-name=libstdc++.a")
        .output()
    {
        Ok(output) if output.status.success() => {
            PathBuf::from(String::from_utf8_lossy(&output.stdout).trim())
        }
        _ => {
            println!("cargo:warning=gcc not found; libstdc++ will be linked dynamically");
            return;
        }
    };
    // When gcc cannot find the file it echoes the name back unchanged.
    if !archive.is_file() {
        println!("cargo:warning=libstdc++.a not found; libstdc++ will be linked dynamically");
        return;
    }

    let staging = out_dir.join("static-cpp");
    if let Err(err) = std::fs::create_dir_all(&staging) {
        println!(
            "cargo:warning=could not prepare {}: {err}",
            staging.display()
        );
        return;
    }
    if let Err(err) = std::fs::copy(&archive, staging.join("libstdc++.a")) {
        println!("cargo:warning=could not copy libstdc++.a: {err}");
        return;
    }
    println!("cargo:rustc-link-search=native={}", staging.display());
    // libgcc is already static by default; saying so keeps it that way if that changes.
    println!("cargo:rustc-link-arg=-static-libgcc");
}

fn embed_icon() {
    let source = Path::new("assets/icon.png");
    if !source.exists() {
        println!("cargo:warning=assets/icon.png is missing; building without an icon");
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let ico = Path::new(&out_dir).join("icon.ico");

    match image::open(source) {
        Ok(image) => {
            // Windows icons must be square, and 256x256 is the largest size shells use.
            let square = image.resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
            if let Err(err) = square.save_with_format(&ico, image::ImageFormat::Ico) {
                println!("cargo:warning=could not write icon.ico: {err}");
                return;
            }
        }
        Err(err) => {
            println!("cargo:warning=could not read assets/icon.png: {err}");
            return;
        }
    }

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(&ico.to_string_lossy());
    if let Err(err) = resource.compile() {
        // Not fatal: the application still works, it just keeps the default icon.
        println!("cargo:warning=could not embed the icon: {err}");
    }
}
