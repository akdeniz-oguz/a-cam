//! Embeds the application icon into the executable.
//!
//! The icon is kept as a PNG in `assets/` because that is what the window needs at runtime.
//! Windows wants an `.ico` for the executable itself, so we convert it here at build time
//! rather than keeping two copies of the same artwork in the repository.

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.png");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

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
