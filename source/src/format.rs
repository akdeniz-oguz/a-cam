//! The format the camera advertises, plus the fallback test pattern.
//!
//! NV12 was chosen because it is the format camera applications expect and the Frame Server
//! accepts it without an extra conversion. It is a full-resolution Y plane followed by a
//! half-resolution interleaved U/V plane.

use windows::Win32::Media::MediaFoundation::{
    IMFMediaType, IMFSample, MF_MT_ALL_SAMPLES_INDEPENDENT, MF_MT_AVG_BITRATE,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE,
    MF_MT_MAJOR_TYPE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MFCreateMediaType,
    MFCreateMemoryBuffer, MFCreateSample, MFGetSystemTime, MFMediaType_Video,
    MFSampleExtension_CleanPoint, MFVideoFormat_NV12, MFVideoInterlace_Progressive,
};
use windows::core::Result;

// The dimensions come from the shared crate: the application writes frames at exactly
// this size.
pub use a_cam_shared::{HEIGHT, PIXEL_BYTES as FRAME_BYTES, WIDTH};

pub const FPS: u32 = 30;
/// Frame duration in 100 ns units, which is Media Foundation's time base.
pub const FRAME_DURATION: i64 = 10_000_000 / FPS as i64;

/// Packs two 32-bit values into the single 64-bit attribute Media Foundation expects.
fn pack(high: u32, low: u32) -> u64 {
    (u64::from(high) << 32) | u64::from(low)
}

pub fn media_type() -> Result<IMFMediaType> {
    unsafe {
        let media_type = MFCreateMediaType()?;
        media_type.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        media_type.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        media_type.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        media_type.SetUINT64(&MF_MT_FRAME_SIZE, pack(WIDTH, HEIGHT))?;
        media_type.SetUINT64(&MF_MT_FRAME_RATE, pack(FPS, 1))?;
        media_type.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        // For NV12 the stride is the byte width of the Y plane.
        media_type.SetUINT32(&MF_MT_DEFAULT_STRIDE, WIDTH)?;
        // These two were missing compared with a working reference implementation: every
        // frame stands on its own (no inter-frame coding) and the average bitrate is stated.
        media_type.SetUINT32(&MF_MT_ALL_SAMPLES_INDEPENDENT, 1)?;
        media_type.SetUINT32(&MF_MT_AVG_BITRATE, FRAME_BYTES as u32 * 8 * FPS)?;
        Ok(media_type)
    }
}

/// Fills `target` with a frame of moving colour bars.
///
/// This is what the camera shows when the application is not streaming, so consumers always
/// get a valid picture instead of dropping the connection.
pub fn fill_test_pattern(target: &mut [u8], index: u64) {
    // The eight standard colour bars as NV12 (Y, U, V) triples.
    const BARS: [(u8, u8, u8); 8] = [
        (235, 128, 128), // white
        (210, 16, 146),  // yellow
        (170, 166, 16),  // cyan
        (145, 54, 34),   // green
        (106, 202, 222), // magenta
        (81, 90, 240),   // red
        (41, 240, 110),  // blue
        (16, 128, 128),  // black
    ];
    // Shift by one bar twice a second so it is obvious at a glance that the feed is live.
    let shift = (index / 15) as usize;
    if target.len() != FRAME_BYTES {
        return;
    }
    let (luma, chroma) = target.split_at_mut((WIDTH * HEIGHT) as usize);
    let bar_width = WIDTH as usize / BARS.len();

    for y in 0..HEIGHT as usize {
        let row = &mut luma[y * WIDTH as usize..][..WIDTH as usize];
        for (x, value) in row.iter_mut().enumerate() {
            *value = BARS[(x / bar_width + shift) % BARS.len()].0;
        }
    }
    // The UV plane is half resolution, with U and V interleaved.
    for y in 0..(HEIGHT as usize / 2) {
        let row = &mut chroma[y * WIDTH as usize..][..WIDTH as usize];
        for x in 0..(WIDTH as usize / 2) {
            let bar = BARS[((x * 2) / bar_width + shift) % BARS.len()];
            row[x * 2] = bar.1;
            row[x * 2 + 1] = bar.2;
        }
    }
}

/// Wraps ready NV12 pixels in a Media Foundation sample.
pub fn sample_from_nv12(pixels: &[u8]) -> Result<IMFSample> {
    unsafe {
        let buffer = MFCreateMemoryBuffer(FRAME_BYTES as u32)?;
        let mut target = std::ptr::null_mut();
        buffer.Lock(&mut target, None, None)?;
        std::ptr::copy_nonoverlapping(pixels.as_ptr(), target, FRAME_BYTES);
        buffer.Unlock()?;
        buffer.SetCurrentLength(FRAME_BYTES as u32)?;

        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        // The timestamp comes from the system clock. It used to start at zero and count up,
        // which for a live camera makes every frame look ancient: the pipeline accepted the
        // samples and then dropped them all, so nothing ever reached a consumer.
        sample.SetSampleTime(MFGetSystemTime())?;
        sample.SetSampleDuration(FRAME_DURATION)?;
        // No compression, so every frame is a key frame.
        sample.SetUINT32(&MFSampleExtension_CleanPoint, 1)?;
        Ok(sample)
    }
}
