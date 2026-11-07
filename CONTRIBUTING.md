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

- After changing anything under `source/`, reinstall the DLL from an **administrator**
  PowerShell: `.\tools\install-vcam.ps1`. Windows loads it from `C:\Program Files\a-cam\`;
  it cannot run from a user profile folder, because the Frame Server cannot read those.
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
