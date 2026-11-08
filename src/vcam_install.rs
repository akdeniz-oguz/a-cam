//! Installing the virtual camera source, from inside the application.
//!
//! The DLL travels in the executable like adb does, but unlike adb it cannot be run from a
//! cache folder. Windows loads it by the path recorded in HKEY_LOCAL_MACHINE, inside its
//! own Frame Server service, which runs as Local Service and cannot read a user profile.
//! So it has to be written somewhere under Program Files, and both that and the registry
//! key need administrator rights.
//!
//! Rather than shipping a script for the user to run, a-Cam asks Windows to restart a copy
//! of itself with elevation, which writes the file and the key and exits. That is the only
//! moment a-Cam ever needs administrator rights.

use anyhow::{Context as _, Result, bail};
use std::path::PathBuf;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegGetValueW, RegSetValueExW,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, PCWSTR};

/// The command line that asks an elevated copy of a-Cam to do the installation.
pub const INSTALL_FLAG: &str = "--install-webcam";
/// And the one that undoes it.
pub const UNINSTALL_FLAG: &str = "--remove-webcam";

/// Must match `CLSID_ACAM_SOURCE` in `source/src/lib.rs` and `SOURCE_CLSID` in `vcam.rs`.
const CLSID: &str = "{6D0F2B7A-1C84-4F6E-9B35-27A4C1D8E500}";
const DISPLAY_NAME: &str = "a-Cam (phone)";

/// Whether this build carries the source DLL. False only if it could not be built.
pub const AVAILABLE: bool = cfg!(embedded_source);

/// The source DLL, carried inside the executable.
#[cfg(embedded_source)]
const SOURCE_DLL: &[u8] = include_bytes!(env!("ACAM_SOURCE_DLL"));
#[cfg(not(embedded_source))]
const SOURCE_DLL: &[u8] = &[];

fn install_path() -> PathBuf {
    let program_files =
        std::env::var_os("ProgramFiles").map_or_else(|| r"C:\Program Files".into(), PathBuf::from);
    program_files.join("a-Cam").join("a_cam_source.dll")
}

fn clsid_key() -> String {
    format!("Software\\Classes\\CLSID\\{CLSID}")
}

/// Whether the registered DLL exists and matches the one in this build.
///
/// The size comparison is what makes an upgrade take effect: a newer a-Cam carrying a
/// changed DLL reports itself as not installed, so the user is asked once and the stale
/// file is replaced.
pub fn is_installed() -> bool {
    registered_path().is_some_and(|path| {
        std::fs::metadata(path).is_ok_and(|data| data.len() == SOURCE_DLL.len() as u64)
    })
}

fn registered_path() -> Option<PathBuf> {
    let key = HSTRING::from(format!("{}\\InprocServer32", clsid_key()));
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            &key,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let characters = (size as usize / 2).saturating_sub(1);
    Some(PathBuf::from(String::from_utf16_lossy(
        &buffer[..characters],
    )))
}

/// Asks Windows to run a copy of a-Cam with administrator rights to do the installation.
///
/// Returns once that copy has finished. The user sees one consent prompt; declining it is
/// reported as an ordinary error, not a failure of the stream.
pub fn request_install(uninstall: bool) -> Result<()> {
    let exe = std::env::current_exe().context("could not locate a-cam.exe")?;
    let flag = if uninstall {
        UNINSTALL_FLAG
    } else {
        INSTALL_FLAG
    };

    // "runas" is what raises the consent prompt; without it the write would simply be
    // denied, since Program Files and HKEY_LOCAL_MACHINE are not ours to change.
    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("runas"),
            &HSTRING::from(exe.as_os_str()),
            &HSTRING::from(flag),
            None,
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW reports failure as a value of 32 or less.
    if result.0 as usize <= 32 {
        bail!(
            "Windows did not allow the webcam setup to start. It asks for administrator \
             rights once, to place the camera source where its own service can read it."
        );
    }
    Ok(())
}

/// Writes the DLL and registers it. Only ever runs in the elevated copy.
pub fn install() -> Result<()> {
    let path = install_path();
    let folder = path.parent().expect("install path has a parent");
    std::fs::create_dir_all(folder)
        .with_context(|| format!("could not create {}", folder.display()))?;

    // Replacing a DLL that the Frame Server still has open fails, so the old registration
    // is removed first and the file is written beside the target and renamed into place.
    let _ = unregister();
    let temporary = path.with_extension("partial");
    std::fs::write(&temporary, SOURCE_DLL)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    std::fs::rename(&temporary, &path).with_context(|| {
        format!(
            "could not replace {}. Close any application using the camera and try again.",
            path.display()
        )
    })?;

    let base = clsid_key();
    write_string(&base, None, DISPLAY_NAME)?;
    let inproc = format!("{base}\\InprocServer32");
    write_string(&inproc, None, &path.to_string_lossy())?;
    // "Both": the COM object can be used from an STA or an MTA. Media sources need this.
    write_string(&inproc, Some("ThreadingModel"), "Both")
}

/// Removes the registration and the file. Only ever runs in the elevated copy.
pub fn uninstall_source() -> Result<()> {
    unregister()?;
    let path = install_path();
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("could not remove {}", path.display()))?;
    }
    if let Some(folder) = path.parent() {
        // Only if we left it empty; never take a folder that holds anything else.
        let _ = std::fs::remove_dir(folder);
    }
    Ok(())
}

fn unregister() -> Result<()> {
    let key = HSTRING::from(clsid_key());
    let status = unsafe { RegDeleteTreeW(HKEY_LOCAL_MACHINE, &key) };
    // Absent is the state we wanted anyway.
    if status != ERROR_SUCCESS && registered_path().is_some() {
        bail!(
            "could not remove the existing registration (error {})",
            status.0
        );
    }
    Ok(())
}

/// Runs the elevated half and reports the outcome.
///
/// This copy has no window of its own, so a failure has nowhere to go but a message box.
/// Success says nothing: the application that asked is already watching for it.
pub fn run_elevated_action(uninstall: bool) {
    let outcome = if uninstall {
        uninstall_source()
    } else {
        install()
    };
    if let Err(err) = outcome {
        let text = HSTRING::from(format!("{err:#}"));
        let caption = HSTRING::from("a-Cam webcam setup");
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::MessageBoxW(
                None,
                &text,
                &caption,
                windows::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
            );
        }
    }
}

fn write_string(path: &str, name: Option<&str>, value: &str) -> Result<()> {
    let path = HSTRING::from(path);
    let mut key = HKEY::default();
    unsafe {
        let status = RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            &path,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE | KEY_READ,
            None,
            &mut key,
            None,
        );
        if status != ERROR_SUCCESS {
            bail!("could not create the registry key (error {})", status.0);
        }
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = std::slice::from_raw_parts(
            wide.as_ptr().cast::<u8>(),
            std::mem::size_of_val(&wide[..]),
        );
        let named = name.map(HSTRING::from);
        let status = RegSetValueExW(
            key,
            named
                .as_ref()
                .map_or(PCWSTR::null(), |text| PCWSTR(text.as_ptr())),
            None,
            REG_SZ,
            Some(bytes),
        );
        let _ = RegCloseKey(key);
        if status != ERROR_SUCCESS {
            bail!("could not write the registry value (error {})", status.0);
        }
    }
    Ok(())
}
