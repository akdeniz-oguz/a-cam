//! The frame bridge between `a-cam.exe` and the virtual camera DLL.
//!
//! The two processes live in different sessions: the application in the user's session, the
//! DLL inside the Windows Frame Server service (Local Service). Named shared memory would
//! then need the `Global\` namespace, and creating an object there requires a privilege a
//! normal user does not have. So the bridge is a **file-backed** memory mapping instead: a
//! file under `C:\Users\Public`, which both sides can reach, mapped straight into memory.
//!
//! Synchronisation is lock-free. The writer bumps the sequence number to an odd value,
//! writes the pixels, then completes it to an even value. The reader compares the sequence
//! before and after its copy; if they match and the value is even, the frame is intact.
//! Without a lock, neither side can block the other if it crashes.

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_BEGIN, FILE_GENERIC_READ,
    FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, SetEndOfFile,
    SetFilePointerEx,
};
use windows::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, FILE_MAP_READ, MapViewOfFile, PAGE_READONLY,
    PAGE_READWRITE, UnmapViewOfFile,
};
use windows::core::{HSTRING, Result};

/// The camera's fixed resolution. The DLL advertises this size in its media type, so the
/// application scales every frame to exactly these dimensions before writing it.
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;

/// NV12: a full-resolution Y plane followed by a half-resolution interleaved UV plane.
pub const PIXEL_BYTES: usize = (WIDTH as usize * HEIGHT as usize) * 3 / 2;
const HEADER_BYTES: usize = 32;
const TOTAL_BYTES: usize = HEADER_BYTES + PIXEL_BYTES;

const MAGIC: u32 = 0x4143_414D; // "ACAM"
const VERSION: u32 = 1;

/// The bridge file. Both processes can reach it; the DLL writes its logs here too.
pub const PATH: &str = r"C:\Users\Public\a-cam-frame.bin";

/// Shared base for the mapped memory.
struct Mapping {
    file: HANDLE,
    section: HANDLE,
    base: *mut u8,
}

impl Mapping {
    fn open(writable: bool) -> Result<Self> {
        unsafe {
            let path = HSTRING::from(PATH);
            let access = if writable {
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0
            } else {
                FILE_GENERIC_READ.0
            };
            let file = CreateFileW(
                &path,
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                if writable {
                    CREATE_ALWAYS
                } else {
                    OPEN_EXISTING
                },
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?;
            if writable {
                // The file has to be full size before it can be mapped.
                SetFilePointerEx(file, TOTAL_BYTES as i64, None, FILE_BEGIN)?;
                SetEndOfFile(file)?;
            }
            let section = CreateFileMappingW(
                file,
                None,
                if writable {
                    PAGE_READWRITE
                } else {
                    PAGE_READONLY
                },
                0,
                TOTAL_BYTES as u32,
                None,
            );
            let section = match section {
                Ok(section) => section,
                Err(err) => {
                    let _ = CloseHandle(file);
                    return Err(err);
                }
            };
            let view = MapViewOfFile(
                section,
                if writable {
                    FILE_MAP_ALL_ACCESS
                } else {
                    FILE_MAP_READ
                },
                0,
                0,
                TOTAL_BYTES,
            );
            if view.Value.is_null() {
                let _ = CloseHandle(section);
                let _ = CloseHandle(file);
                return Err(windows::core::Error::from_thread());
            }
            Ok(Self {
                file,
                section,
                base: view.Value.cast(),
            })
        }
    }

    fn sequence(&self) -> u64 {
        // SAFETY: the mapping is TOTAL_BYTES long and the header occupies the first 32.
        unsafe { self.base.add(8).cast::<u64>().read_volatile() }
    }

    fn set_sequence(&self, value: u64) {
        unsafe { self.base.add(8).cast::<u64>().write_volatile(value) };
    }

    fn pixels(&self) -> *mut u8 {
        unsafe { self.base.add(HEADER_BYTES) }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            let _ = UnmapViewOfFile(windows::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.base.cast(),
            });
            let _ = CloseHandle(self.section);
            let _ = CloseHandle(self.file);
        }
    }
}

// SAFETY: the mapped pointer belongs to this process and is only used by its owner.
unsafe impl Send for Mapping {}

/// Writes frames into the shared buffer (application side).
pub struct Writer {
    mapping: Mapping,
    counter: u64,
}

impl Writer {
    pub fn create() -> Result<Self> {
        let mapping = Mapping::open(true)?;
        unsafe {
            mapping.base.cast::<u32>().write_volatile(MAGIC);
            mapping.base.add(4).cast::<u32>().write_volatile(VERSION);
        }
        mapping.set_sequence(0);
        Ok(Self {
            mapping,
            counter: 0,
        })
    }

    /// Writes one NV12 frame. `pixels` must be exactly [`PIXEL_BYTES`] long.
    pub fn write(&mut self, pixels: &[u8]) {
        if pixels.len() != PIXEL_BYTES {
            return;
        }
        // An odd sequence means "being written": the reader discards what it copies then.
        self.counter += 1;
        self.mapping.set_sequence(self.counter * 2 - 1);
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), self.mapping.pixels(), PIXEL_BYTES);
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::Release);
        self.mapping.set_sequence(self.counter * 2);
    }
}

/// Reads frames from the shared buffer (DLL side).
pub struct Reader {
    mapping: Mapping,
    last: u64,
}

impl Reader {
    pub fn open() -> Result<Self> {
        let mapping = Mapping::open(false)?;
        let magic = unsafe { mapping.base.cast::<u32>().read_volatile() };
        if magic != MAGIC {
            return Err(windows::core::Error::empty());
        }
        Ok(Self { mapping, last: 0 })
    }

    /// Copies an intact frame into `target`. Returns false while a write is in progress, or
    /// when the frame is not newer than the last one and `allow_repeat` is not set.
    pub fn read_into(&mut self, target: &mut [u8], allow_repeat: bool) -> bool {
        if target.len() != PIXEL_BYTES {
            return false;
        }
        let before = self.mapping.sequence();
        if before == 0 || before % 2 == 1 {
            return false; // no frame yet, or one is being written
        }
        if before == self.last && !allow_repeat {
            return false; // same frame as last time
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        unsafe {
            std::ptr::copy_nonoverlapping(self.mapping.pixels(), target.as_mut_ptr(), PIXEL_BYTES);
        }
        std::sync::atomic::fence(std::sync::atomic::Ordering::Acquire);
        // If the writer stepped in mid-copy the frame may be torn, so drop it.
        if self.mapping.sequence() != before {
            return false;
        }
        self.last = before;
        true
    }
}
