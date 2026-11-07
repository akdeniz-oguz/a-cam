//! Registers the virtual camera with Windows and keeps it alive.
//!
//! `MFCreateVirtualCamera` does not let us hand frames over directly: it takes a CLSID and
//! loads that COM object inside its own Frame Server service. The code producing frames
//! therefore lives in a separate binary (`a_cam_source.dll`); this module only opens and
//! closes the camera.
//!
//! The camera is visible system-wide for as long as the `IMFVirtualCamera` object lives.
//! Dropping it, or closing the application, removes it (`Lifetime_Session`).

use anyhow::{Context as _, Result};
use windows::Win32::Media::MediaFoundation::{
    IMFVirtualCamera, MF_VERSION, MFCreateVirtualCamera, MFSTARTUP_FULL, MFShutdown, MFStartup,
    MFVirtualCameraAccess_CurrentUser, MFVirtualCameraLifetime_Session,
    MFVirtualCameraType_SoftwareCameraSource,
};
use windows::core::HSTRING;

/// CLSID of the source DLL.
///
/// NOTE: must match `CLSID_ACAM_SOURCE` in `source/src/lib.rs` exactly. The two binaries
/// cannot share a constant, so changing one means changing the other.
const SOURCE_CLSID: &str = "{6D0F2B7A-1C84-4F6E-9B35-27A4C1D8E500}";

/// The name the camera shows up under in Windows.
pub const FRIENDLY_NAME: &str = "a-Cam (phone)";

/// Keeps the virtual camera registered with the system for as long as it is held.
pub struct VirtualCamera {
    camera: IMFVirtualCamera,
}

impl VirtualCamera {
    /// Creates the virtual camera and starts it.
    pub fn start() -> Result<Self> {
        unsafe {
            MFStartup(MF_VERSION, MFSTARTUP_FULL).context("MFStartup failed")?;
            let camera = MFCreateVirtualCamera(
                MFVirtualCameraType_SoftwareCameraSource,
                MFVirtualCameraLifetime_Session,
                // CurrentUser needs no elevation; only registering the DLL does.
                MFVirtualCameraAccess_CurrentUser,
                &HSTRING::from(FRIENDLY_NAME),
                &HSTRING::from(SOURCE_CLSID),
                None,
            )
            .context(
                "MFCreateVirtualCamera failed. Is a_cam_source.dll registered? \
                 (Windows 11 is required.)",
            )?;
            camera.Start(None).context(
                "IMFVirtualCamera::Start failed. Access denied here usually means the Frame \
                 Server services cannot read the DLL; it must live outside your user profile.",
            )?;
            Ok(Self { camera })
        }
    }
}

impl Drop for VirtualCamera {
    fn drop(&mut self) {
        unsafe {
            let _ = self.camera.Stop();
            let _ = self.camera.Remove();
            let _ = MFShutdown();
        }
    }
}
