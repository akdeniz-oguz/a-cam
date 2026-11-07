//! The a-Cam virtual camera source (an in-process COM server).
//!
//! This DLL does not run in our process but inside the Windows Frame Server service:
//! `MFCreateVirtualCamera` takes a CLSID and loads that COM object into its own process.
//! That is why the code feeding the camera has to be a separate binary.
//!
//! The entry points below are the COM contract Windows expects from an in-process server:
//! a class factory, plus registration and unregistration.

mod delegate;
mod format;
mod log;
mod registry;
mod source;
mod stream;

use windows::Win32::Foundation::{
    CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_POINTER, HMODULE, S_FALSE, S_OK,
};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, implement};

/// Identity of our virtual camera source; Windows finds the DLL through this CLSID.
pub const CLSID_ACAM_SOURCE: GUID = GUID::from_u128(0x6d0f2b7a_1c84_4f6e_9b35_27a4c1d8e500);

/// The name the camera shows up under in Windows.
pub const CAMERA_NAME: &str = "a-Cam (phone)";

/// The DLL's own path, written to the registry. Filled in by DllMain.
static mut MODULE: HMODULE = HMODULE(std::ptr::null_mut());

#[unsafe(no_mangle)]
pub extern "system" fn DllMain(
    module: HMODULE,
    reason: u32,
    _reserved: *mut core::ffi::c_void,
) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        // SAFETY: DllMain runs on one thread before anything else of ours does.
        unsafe { MODULE = module };
    }
    1
}

fn module_path() -> String {
    let mut buffer = [0u16; 1024];
    // SAFETY: MODULE is written once in DllMain and is read-only afterwards.
    let module = unsafe { MODULE };
    let length = unsafe { GetModuleFileNameW(Some(module), &mut buffer) } as usize;
    String::from_utf16_lossy(&buffer[..length])
}

/// The factory COM asks for in order to create our object.
#[implement(IClassFactory)]
struct SourceFactory;

impl IClassFactory_Impl for SourceFactory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<'_, IUnknown>,
        interface_id: *const GUID,
        out: *mut *mut core::ffi::c_void,
    ) -> windows::core::Result<()> {
        if out.is_null() {
            return Err(E_POINTER.into());
        }
        // SAFETY: the COM caller is required to pass a valid pointer.
        unsafe { *out = std::ptr::null_mut() };
        if !outer.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        log::line("CreateInstance");
        let object = match source::create() {
            Ok(object) => object,
            Err(err) => {
                log::line(&format!("source::create -> ERROR {err}"));
                return Err(err);
            }
        };
        // SAFETY: the COM caller is required to pass valid pointers.
        unsafe { object.query(&*interface_id, out).ok() }
    }

    fn LockServer(&self, _lock: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
}

#[unsafe(no_mangle)]
/// Hands COM the class factory for our source.
///
/// # Safety
///
/// Called by COM, which guarantees `class_id` and `interface_id` point to valid GUIDs and
/// that `out` is a writable pointer slot.
pub unsafe extern "system" fn DllGetClassObject(
    class_id: *const GUID,
    interface_id: *const GUID,
    out: *mut *mut core::ffi::c_void,
) -> HRESULT {
    if out.is_null() || class_id.is_null() || interface_id.is_null() {
        return E_POINTER;
    }
    // SAFETY: the COM caller is required to pass valid pointers.
    unsafe {
        *out = std::ptr::null_mut();
        if *class_id != CLSID_ACAM_SOURCE {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = SourceFactory.into();
        factory.query(&*interface_id, out)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    // Keep the DLL loaded; the Frame Server may create another source at any time.
    S_FALSE
}

#[unsafe(no_mangle)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    match registry::register(&CLSID_ACAM_SOURCE, CAMERA_NAME, &module_path()) {
        Ok(()) => S_OK,
        Err(err) => err.into(),
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    match registry::unregister(&CLSID_ACAM_SOURCE) {
        Ok(()) => S_OK,
        Err(err) => err.into(),
    }
}
