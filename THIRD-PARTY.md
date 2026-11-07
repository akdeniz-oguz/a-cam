# Third-party components

a-Cam itself is MIT licensed (see [LICENSE](LICENSE)). It builds on, and ships alongside,
the following projects.

## scrcpy — Apache-2.0

<https://github.com/Genymobile/scrcpy>

a-Cam does not contain scrcpy source code. It uses two files from an official scrcpy
release at runtime:

- `scrcpy-server`, pushed to the phone and run there
- `adb.exe`, to talk to the phone

Release archives published on this repository bundle an unmodified scrcpy release together
with its `LICENSE` file, as Apache-2.0 permits. If you build from source you download
scrcpy yourself (`tools/fetch-scrcpy.ps1`).

## openh264 — BSD-2-Clause

<https://github.com/cisco/openh264>

Used through the [`openh264`](https://crates.io/crates/openh264) crate, which compiles
Cisco's decoder from source.

> **Patent note.** H.264 is covered by patents. Cisco pays the MPEG LA royalties only for
> the **prebuilt binaries it publishes itself**; that cover does not extend to builds made
> from source, which is what this project does. For personal and development use this is
> not usually a concern, but if you plan to distribute a product commercially you should
> look into it. This is a pointer, not legal advice.

## egui / eframe — MIT OR Apache-2.0

<https://github.com/emilk/egui>

The user interface and GPU rendering.

## windows / windows-core — MIT OR Apache-2.0

<https://github.com/microsoft/windows-rs>

Microsoft's Rust bindings for the Win32 and Media Foundation APIs.

## Other crates

`anyhow`, `image`, `winresource` and their transitive dependencies are all MIT or
Apache-2.0 licensed. `cargo tree` lists the full set; `cargo install cargo-license` then
`cargo license` prints the licences.
