# Contributing

Thanks for taking a look. Issues and pull requests are both welcome.

## Getting set up

```powershell
winget install Rustlang.Rustup
winget install BrechtSanders.WinLibs.POSIX.MSVCRT
rustup default stable-x86_64-pc-windows-gnu

git clone https://github.com/<you>/a-cam
cd a-cam
.\tools\fetch-scrcpy.ps1      # downloads scrcpy into scrcpy\
cargo build --release
```

MinGW is needed for two reasons: `dlltool`, which Rust's GNU target uses when linking
against system DLLs, and `gcc`/`g++`, because the H.264 decoder is written in C++.

`build.rs` does more than usual, which is worth knowing before debugging a build: it builds
`source/` through a nested `cargo` invocation and embeds the resulting DLL, embeds five
files out of `scrcpy/`, and arranges for libstdc++ to be linked statically. The product is a
single self-contained `a-cam.exe`; there is no packaging step. Build without `scrcpy/` and
everything still compiles, but the binary falls back to looking for a `scrcpy` folder on
disk and the build says so in a warning.

## Before opening a pull request

CI runs these, so it saves a round trip to run them first:

```powershell
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace --release
```

## Working on the virtual camera

The camera source is a separate DLL that Windows loads inside its own Frame Server service,
so the usual debugging tools do not reach it. Two things make that workable:

- After changing anything under `source/`, rebuild and reinstall the DLL from an
  **administrator** PowerShell:

  ```powershell
  cargo build --release
  .\target\release\a-cam.exe --install-webcam
  ```

  Windows loads it from `C:\Program Files\a-Cam\`; it cannot run from a user profile
  folder, because the Frame Server cannot read those. `--install-webcam` and
  `--remove-webcam` are the elevated helper the application re-launches itself with, so
  they do the work directly and expect to be elevated already. Running the application
  normally also re-installs the DLL when the installed copy differs in size from the
  embedded one, but an identically sized rebuild would slip past that check — hence the
  explicit flag while developing.
- The DLL writes to `C:\Users\Public\a-cam-source-<pid>.log`. A file whose name contains
  `svchost` is the copy running inside the Frame Server — that is the interesting one.

Windows also keeps its own trace, which is often more informative than a guess:

```powershell
Get-WinEvent -LogName 'Microsoft-Windows-MF-FrameServer/Camera_FrameServer' -MaxEvents 50 |
    Sort-Object TimeCreated | Format-List TimeCreated, Message
```

## A note on scope

a-Cam deliberately installs nothing on the phone. That is why Developer options and USB
debugging are required: everything goes through adb. Changes that would need an app on the
phone are out of scope.
