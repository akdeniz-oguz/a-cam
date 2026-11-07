//! COM registration.
//!
//! Everything goes under **HKEY_LOCAL_MACHINE**, not HKCU. The Frame Server and Frame Server
//! Monitor services that load the DLL run as Local Service / Local System and cannot see our
//! user hive, so an HKCU registration leaves the camera silently missing, or failing with an
//! access error when it starts.
//!
//! Writing to HKLM needs administrator rights, which is why registration is a one-off
//! elevated command (see README.md).

use windows::Win32::Foundation::{ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
};
use windows::core::{Error, PCWSTR, Result};

/// A temporary NUL-terminated UTF-16 buffer.
struct Wide(Vec<u16>);

impl Wide {
    fn new(text: &str) -> Self {
        Self(text.encode_utf16().chain(std::iter::once(0)).collect())
    }

    fn as_pcwstr(&self) -> PCWSTR {
        PCWSTR(self.0.as_ptr())
    }

    /// The bytes to write as a value, terminator included.
    fn as_bytes(&self) -> &[u8] {
        // SAFETY: the u16 slice is read as a byte slice of the same length.
        unsafe { std::slice::from_raw_parts(self.0.as_ptr().cast::<u8>(), self.0.len() * 2) }
    }
}

fn check(status: WIN32_ERROR) -> Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(Error::from_hresult(status.to_hresult()))
    }
}

fn clsid_key(class_id: &windows::core::GUID) -> String {
    format!("Software\\Classes\\CLSID\\{{{class_id:?}}}")
}

/// Creates a key and writes text to its default ("") value.
fn write_default(path: &str, value: &str) -> Result<()> {
    let mut key = HKEY::default();
    let path = Wide::new(path);
    unsafe {
        check(RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_pcwstr(),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        ))?;
        let data = Wide::new(value);
        let status = RegSetValueExW(key, None, None, REG_SZ, Some(data.as_bytes()));
        let _ = RegCloseKey(key);
        check(status)
    }
}

/// Writes text to a named value of a key.
fn write_value(path: &str, name: &str, value: &str) -> Result<()> {
    let mut key = HKEY::default();
    let path = Wide::new(path);
    let name = Wide::new(name);
    unsafe {
        check(RegCreateKeyExW(
            HKEY_LOCAL_MACHINE,
            path.as_pcwstr(),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        ))?;
        let data = Wide::new(value);
        let status = RegSetValueExW(key, name.as_pcwstr(), None, REG_SZ, Some(data.as_bytes()));
        let _ = RegCloseKey(key);
        check(status)
    }
}

pub fn register(class_id: &windows::core::GUID, name: &str, dll_path: &str) -> Result<()> {
    let base = clsid_key(class_id);
    write_default(&base, name)?;
    let inproc = format!("{base}\\InprocServer32");
    write_default(&inproc, dll_path)?;
    // "Both": the COM object can be used from an STA or an MTA. Media sources need this.
    write_value(&inproc, "ThreadingModel", "Both")
}

pub fn unregister(class_id: &windows::core::GUID) -> Result<()> {
    let path = Wide::new(&clsid_key(class_id));
    unsafe { check(RegDeleteTreeW(HKEY_LOCAL_MACHINE, path.as_pcwstr())) }
}
