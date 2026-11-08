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

- **One file.** `a-Cam.exe` and nothing else — no archive to unpack, no installer
- **No APK** is installed on the phone
- **No OBS** and no third-party virtual camera driver
- Written in Rust; the preview is drawn on the GPU

## Download

[**a-Cam.exe** from the releases page](../../releases/latest). Save it anywhere and run it.

adb, scrcpy's server and the Windows camera source all travel inside the executable, so
there is nothing else to download and nothing to install. On first run adb and the server
are written to `%LOCALAPPDATA%\a-Cam\`, because adb has to exist as a file to be launched
and the server has to be copied to the phone.

Webcam output needs no separate step either: press **Start Stream** and Windows asks once
for permission to place the camera source. Decline it and the preview still works, just
without the webcam.

## How it works

```
┌──────── Android phone ──────────┐      ┌──────────────── Desktop ──────────────────┐
│ Camera                          │      │  a-Cam.exe                                │
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

**Why a second binary?** `MFCreateVirtualCamera` does not take frames from our process: it
is given a CLSID and loads that COM object inside its own service. The code feeding the
camera therefore has to be a separate DLL. a-Cam carries it and places it in
`C:\Program Files\a-Cam\` the first time the webcam is used — it cannot live in a user
profile folder, because the Frame Server service cannot read those. The two processes talk
through a file-backed memory mapping under `C:\Users\Public`; they run in different
sessions, where named shared memory would require a privilege a normal user does not have.

## Layout

```
a-cam/
├── src/              a-Cam.exe — interface, adb, decoding, preview
│   ├── main.rs       egui interface, crop / zoom / rotation
│   ├── adb.rs        device discovery, camera listing
│   ├── stream.rs     starts scrcpy-server, reads H.264 packets
│   ├── capture.rs    decoding, RGBA (screen) and NV12 (virtual camera)
│   ├── bundled.rs    the embedded adb / scrcpy-server, written out on first run
│   ├── vcam.rs       registers the virtual camera with the system
│   └── vcam_install.rs  places and registers the camera source (asks for elevation)
├── source/           a_cam_source.dll — the COM media source Windows loads
├── shared/           frame bridge shared by both sides
├── assets/           application icon
├── tools/
│   └── fetch-scrcpy.ps1   downloads the scrcpy release that gets embedded
└── scrcpy/           contents of the scrcpy Windows zip (not in git)
```

`build.rs` does the bundling: it builds `source/`, embeds the DLL, embeds five files out of
`scrcpy/`, and links the C++ runtime statically so the executable has no DLL dependencies
of its own.

scrcpy is not vendored into this repository: it is a separate project, its releases are
around 90 MB of binaries, and a copy in git would go stale. `tools/fetch-scrcpy.ps1`
downloads an official release at build time; only five files out of it are embedded, since
the rest belongs to the scrcpy application itself, which a-Cam never runs.

## Requirements

| | |
|---|---|
| Phone | **Android 12+** (camera capture arrived in that release), with **Developer options and USB debugging enabled** — see the notice above |
| Windows | **Windows 11** for the virtual camera; the preview also works on Windows 10 |
| scrcpy | Only to *build*: a 2.2+ Windows zip in `scrcpy/`. Released builds already contain it |

## Building

Rust's GNU target and MinGW are needed. Visual Studio is not.

```powershell
winget install Rustlang.Rustup
winget install BrechtSanders.WinLibs.POSIX.MSVCRT
rustup default stable-x86_64-pc-windows-gnu

.\tools\fetch-scrcpy.ps1
cargo build --release
```

`target\release\a-cam.exe` is the whole distributable — around 21 MB, with no files beside
it. There is no packaging step.

> MinGW provides two things: `dlltool`, which Rust's GNU target uses when linking against
> system DLLs, and `gcc`/`g++`, because the H.264 decoder is written in C++.

Building without `scrcpy/` also works, but the result is not self-contained: it looks for a
`scrcpy` folder on disk instead, and the build prints a warning saying so.

## Using it

1. Connect the phone over USB and accept the **USB debugging** prompt.
2. Run `a-Cam.exe`. Cameras are listed automatically.
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

There is nothing to set up. **Start Stream** brings the webcam up with it, and the phone
appears as **a-Cam (phone)** in Zoom, Meet, Teams, Discord and browsers. **Stop** removes
it again.

The very first time, Windows shows one **User Account Control** prompt: the camera source
has to be placed in `C:\Program Files\a-Cam\`, which needs administrator rights once.
Afterwards it is silent. Declining is fine — the stream keeps running without webcam
output, and a-Cam does not ask again until it is restarted.

To remove the camera source again, run `a-cam.exe --remove-webcam`.

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
| "No phone found" | Turn on Developer options and USB debugging, then pull down the phone's USB notification and set the connection to File transfer (MTP) or Photo transfer (PTP) rather than charging only. Some cables carry power but no data. |
| "Waiting for USB authorization" | Accept the prompt on the phone |
| "The phone did not return a camera list" | Android 12 or newer is required |
| Picture upside down | Set **Rotation** to 180° |
| Virtual camera missing from the list | Is the stream running? If the UAC prompt was declined, restart a-Cam and accept it |
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
