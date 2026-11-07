<p align="center">
  <img src="assets/icon.png" width="120" alt="a-Cam">
</p>

<h1 align="center">a-Cam</h1>

<p align="center">
  Use the camera of a USB-connected Android phone on your desktop —
  and optionally as a <b>system webcam</b>.
</p>

<p align="center">
  <a href="../../releases/latest"><b>⬇ Download for Windows</b></a> ·
  <a href="#using-it">Usage</a> ·
  <a href="#building">Building</a> ·
  <a href="LICENSE">MIT</a>
</p>

---

> # ⚠️ The phone needs Developer options and USB debugging
>
> **There is no way around this, and it is a deliberate trade-off.**
>
> a-Cam drives the phone over **adb**: it copies scrcpy's small server to the phone, runs it
> with Android's own `app_process`, and reads the video back. That is exactly what lets it
> work **without installing any app on the phone** — but adb only answers when USB debugging
> is on, and that switch lives behind Developer options.
>
> So you pick one or the other: no app on the phone, or no developer mode. a-Cam chooses the
> first.
>
> The one way to avoid both is Android's own webcam mode (`DeviceAsWebcam`, Android 14 QPR1
> and later), where the phone presents itself as a plain USB webcam. It needs no app and no
> developer mode — but the manufacturer has to enable it, which in practice mostly means
> Pixel devices. If your phone has it, you do not need a-Cam at all.

---

- **No APK** is installed on the phone
- **No OBS** and no third-party virtual camera driver
- Written in Rust; the preview is drawn on the GPU

## Download

Pre-built archives are on the [releases page](../../releases/latest). Unpack
`a-Cam-windows-x64.zip` anywhere and run `a-cam.exe` — scrcpy and adb are already inside, so
there is nothing else to install.

To use the phone as a webcam in other applications, run once in an **administrator**
PowerShell:

```powershell
.\install-vcam.ps1
```

## How it works

```
┌──────── Android phone ──────────┐      ┌──────────────── Desktop ──────────────────┐
│ Camera                          │      │  a-cam.exe                                │
│   ↓                             │ USB  │    adb forward → H.264 decode (openh264)  │
│ scrcpy-server (app_process)     ├─────►│      ↓                    ↓               │
│ hardware H.264 encoder          │      │   GPU texture         NV12 conversion     │
└─────────────────────────────────┘      │   (egui preview)          ↓               │
                                         │                     shared memory         │
                                         └───────────────────────────┬───────────────┘
                                                                     │
                                         ┌───────────────────────────▼───────────────┐
                                         │  Windows Frame Server (svchost)           │
                                         │    a_cam_source.dll → "a-Cam (phone)"     │
                                         └───────────────────────────┬───────────────┘
                                                                     ▼
                                                      Zoom / Meet / Teams / Discord
```

**Why two binaries?** `MFCreateVirtualCamera` does not take frames from our process: it is
given a CLSID and loads that COM object inside its own service. The code feeding the camera
therefore has to be a separate DLL. The two processes talk through a file-backed memory
mapping under `C:\Users\Public`; they run in different sessions, where named shared memory
would require a privilege a normal user does not have.

## Layout

```
a-cam/
├── src/              a-cam.exe — interface, adb, decoding, preview
│   ├── main.rs       egui interface, crop / zoom / rotation
│   ├── adb.rs        device discovery, camera listing
│   ├── stream.rs     starts scrcpy-server, reads H.264 packets
│   ├── capture.rs    decoding, RGBA (screen) and NV12 (virtual camera)
│   └── vcam.rs       registers the virtual camera with the system
├── source/           a_cam_source.dll — the COM media source Windows loads
├── shared/           frame bridge shared by both sides
├── assets/           application icon
├── tools/
│   ├── install-vcam.ps1   installs and registers the DLL (administrator)
│   └── package.ps1        produces a distributable folder
└── scrcpy/           contents of the scrcpy Windows zip (tools/fetch-scrcpy.ps1)
```

scrcpy is not vendored into this repository: it is a separate project, its releases are
around 90 MB of binaries, and a copy in git would go stale. `tools/fetch-scrcpy.ps1`
downloads an official release, and the published archives bundle one.

## Requirements

| | |
|---|---|
| Phone | **Android 12+** (camera capture arrived in that release), with **Developer options and USB debugging enabled** — see the notice above |
| Windows | **Windows 11** for the virtual camera; the preview also works on Windows 10 |
| scrcpy | 2.2+ Windows zip, extracted into `scrcpy/` |

## Building

Rust's GNU target and MinGW are needed. Visual Studio is not.

```powershell
winget install Rustlang.Rustup
winget install BrechtSanders.WinLibs.POSIX.MSVCRT
rustup default stable-x86_64-pc-windows-gnu

.\tools\fetch-scrcpy.ps1
cargo build --release
```

> MinGW provides two things: `dlltool`, which Rust's GNU target uses when linking against
> system DLLs, and `gcc`/`g++`, because the H.264 decoder is written in C++.

For a distributable folder:

```powershell
.\tools\package.ps1
```

## Using it

1. Connect the phone over USB and accept the **USB debugging** prompt.
2. Run `a-cam.exe`. Cameras are listed automatically.
3. Press **Start Stream**.

| Control | What it does |
|---|---|
| **CAMERA** | Back or front camera |
| **Quality** | Resolution the phone streams (720p / 1080p / 1440p) |
| **Rotation**, **Mirror** | Applied immediately |
| **ZOOM** | 100%–400%, digital |
| **Crop** (in the preview header) | Crop mode: drag the edges and corners, **Reset** clears it |

Rotation, mirroring, zoom and crop all apply to the preview *and* the virtual camera.

### Virtual camera

One-off setup, in an **administrator PowerShell**:

```powershell
.\tools\install-vcam.ps1
```

After that it is automatic: pressing **Start Stream** makes the camera appear as
**a-Cam (phone)**, and **Stop** removes it.

If the DLL is not registered the stream still works; there is simply no webcam output, and
a warning appears in the LOG panel.

To remove it: `.\tools\install-vcam.ps1 -Uninstall`

> The DLL is installed into `C:\Program Files\a-cam\`. It cannot run from a user profile
> folder, because the Windows Frame Server service cannot read those.

## Known limitations

- The **Windows Camera app** cannot open the virtual camera (`0xA00F429E`). It expects photo
  stream and camera profile capabilities that this source does not advertise. Browsers,
  Zoom, Meet, Teams and Discord all work.
- The virtual camera output is a fixed **1280×720 at 30 fps**. Higher phone resolutions are
  scaled to that size.
- While the application is not streaming, the virtual camera shows **colour bars**.
- Zoom is digital. The phone's hardware zoom (`camera_zoom`) is not wired up yet.

## Troubleshooting

| Symptom | Fix |
|---|---|
| "No phone found" | Pull down the phone's USB notification and set the connection to File transfer (MTP) or Photo transfer (PTP) rather than charging only. Some cables carry power but no data. |
| "Waiting for USB authorization" | Accept the prompt on the phone |
| "The phone did not return a camera list" | Android 12 or newer is required |
| Picture upside down | Set **Rotation** to 180° |
| Virtual camera missing from the list | Did `install-vcam.ps1` run as administrator? Is the stream running? |
| Colour bars in the virtual camera | The stream is off; press **Start Stream** |

Detailed errors appear in the application's **LOG** panel. If the virtual camera side
misbehaves, the DLL writes to `C:\Users\Public\a-cam-source-<pid>.log`.

## Roadmap

- [ ] The phone's hardware zoom, which is sharper than digital
- [ ] Windows Camera app support (sensor profiles and a photo stream)
- [ ] Selectable virtual camera resolution
- [ ] Automatic reconnection when the cable is disturbed
- [ ] Remembered settings

## Contributing

Issues and pull requests are welcome — see [CONTRIBUTING.md](CONTRIBUTING.md), which also
covers how to debug the virtual camera DLL, since it runs inside a Windows service where
the usual tools do not reach.

## Licence

a-Cam is [MIT licensed](LICENSE). It builds on scrcpy (Apache-2.0), openh264 (BSD-2-Clause),
egui and windows-rs; [THIRD-PARTY.md](THIRD-PARTY.md) lists them, including a note about
H.264 patent licensing that matters if you intend to distribute commercially.
