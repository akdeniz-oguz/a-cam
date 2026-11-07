//! The virtual camera's single video stream.
//!
//! When the Frame Server asks for a sample (`RequestSample`) we produce a frame and put it
//! on the event queue. Frames come from the application through shared memory; when none
//! are available the stream falls back to a test pattern.

use crate::delegate::{impl_attributes, impl_event_generator};
use crate::format;
use std::sync::Mutex;
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFMediaEventQueue, IMFMediaSource, IMFMediaStream_Impl, IMFMediaStream2,
    IMFMediaStream2_Impl, IMFStreamDescriptor, MEMediaSample, MEStreamStarted, MEStreamStopped,
    MF_E_INVALIDREQUEST, MF_E_SHUTDOWN, MF_STREAM_STATE, MF_STREAM_STATE_PAUSED,
    MF_STREAM_STATE_RUNNING, MF_STREAM_STATE_STOPPED, MFCreateEventQueue,
};
use windows::core::{Ref, Result, implement};

/// Running state of the stream, plus its frame counter.
struct State {
    running: bool,
    shutdown: bool,
    frames: u64,
    /// Bridge that reads phone frames; opens once the application starts streaming.
    bridge: Option<a_cam_shared::Reader>,
    /// Scratch buffer for the frame, kept so each request does not allocate again.
    pixels: Vec<u8>,
    /// Whether phone frames or the test pattern are showing; logged once when it changes.
    live: bool,
    /// Whether the buffer currently holds a valid phone frame.
    have_frame: bool,
    /// How many reads in a row have been missed.
    misses: u32,
}

/// After this many consecutive misses the stream is treated as genuinely stopped and the
/// test pattern comes back (roughly two seconds).
const MISS_LIMIT: u32 = 60;

#[implement(IMFMediaStream2)]
pub struct MediaStream {
    source: IMFMediaSource,
    descriptor: IMFStreamDescriptor,
    queue: IMFMediaEventQueue,
    attributes: IMFAttributes,
    state: Mutex<State>,
}

impl MediaStream {
    pub fn new(
        source: IMFMediaSource,
        descriptor: IMFStreamDescriptor,
        attributes: IMFAttributes,
    ) -> Result<Self> {
        Ok(Self {
            source,
            descriptor,
            queue: unsafe { MFCreateEventQueue()? },
            attributes,
            state: Mutex::new(State {
                running: false,
                shutdown: false,
                frames: 0,
                bridge: None,
                pixels: vec![0u8; a_cam_shared::PIXEL_BYTES],
                live: false,
                have_frame: false,
                misses: 0,
            }),
        })
    }
}

impl MediaStream_Impl {
    fn queue(&self) -> &IMFMediaEventQueue {
        &self.this.queue
    }

    fn attributes(&self) -> &IMFAttributes {
        &self.this.attributes
    }

    /// Called when the source stops or shuts down.
    pub fn shutdown(&self) {
        if let Ok(mut state) = self.this.state.lock() {
            state.shutdown = true;
            state.running = false;
        }
        unsafe {
            let _ = self.this.queue.Shutdown();
        }
    }

    pub fn set_running(&self, running: bool) -> Result<()> {
        {
            let mut state = self.this.state.lock().expect("stream state");
            if state.shutdown {
                return Err(MF_E_SHUTDOWN.into());
            }
            state.running = running;
        }
        let event = if running {
            MEStreamStarted
        } else {
            MEStreamStopped
        };
        unsafe {
            self.this.queue.QueueEventParamVar(
                event.0 as u32,
                &windows::core::GUID::zeroed(),
                windows::core::HRESULT(0),
                std::ptr::null(),
            )
        }
    }
}

impl IMFMediaStream_Impl for MediaStream_Impl {
    fn GetMediaSource(&self) -> Result<IMFMediaSource> {
        let state = self.this.state.lock().expect("stream state");
        if state.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(self.this.source.clone())
    }

    fn GetStreamDescriptor(&self) -> Result<IMFStreamDescriptor> {
        let state = self.this.state.lock().expect("stream state");
        if state.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(self.this.descriptor.clone())
    }

    fn RequestSample(&self, token: Ref<'_, windows::core::IUnknown>) -> Result<()> {
        let outcome = self.deliver_sample(token);
        // Log the outcome too: we could see the requests arriving but not whether our
        // answers were accepted, and the stream used to stop after 19 requests.
        if let Err(err) = &outcome {
            crate::log::line(&format!("stream: RequestSample -> ERROR {err}"));
        }
        outcome
    }
}

impl MediaStream_Impl {
    fn deliver_sample(&self, token: Ref<'_, windows::core::IUnknown>) -> Result<()> {
        let sample = {
            let mut state = self.this.state.lock().expect("stream state");
            if state.shutdown {
                return Err(MF_E_SHUTDOWN.into());
            }
            // This used to reject requests made before the stream was started. The Frame
            // Server can ask before SetStreamState or Start, and that rejection stopped the
            // flow entirely. A live camera can always produce a frame.
            state.running = true;
            state.frames += 1;
            let index = state.frames;

            // The bridge appears when the application starts streaming, so open it once and
            // keep it rather than retrying on every request.
            if state.bridge.is_none() {
                state.bridge = a_cam_shared::Reader::open().ok();
            }
            // Send a phone frame when there is one, otherwise the test pattern, so the
            // camera still produces a valid picture and consumers do not drop out.
            let mut buffer = std::mem::take(&mut state.pixels);
            let fresh = state
                .bridge
                .as_mut()
                .is_some_and(|reader| reader.read_into(&mut buffer, true));

            // On a missed read, which happens when the application is mid-write, repeat the
            // previous frame. Falling back to the test pattern here looked like a blink
            // every few seconds.
            let live = if fresh {
                state.have_frame = true;
                state.misses = 0;
                true
            } else if state.have_frame && state.misses < MISS_LIMIT {
                state.misses += 1;
                true
            } else {
                state.have_frame = false;
                format::fill_test_pattern(&mut buffer, index);
                false
            };
            if live != state.live {
                state.live = live;
                crate::log::line(if live {
                    "stream: switched to phone frames"
                } else {
                    "stream: no phone frames, showing test pattern"
                });
            }
            let sample = format::sample_from_nv12(&buffer);
            state.pixels = buffer;
            sample?
        };
        let index = self
            .this
            .state
            .lock()
            .map(|state| state.frames)
            .unwrap_or(0);
        if let Some(token) = token.as_ref() {
            unsafe {
                sample.SetUnknown(
                    &windows::Win32::Media::MediaFoundation::MFSampleExtension_Token,
                    token,
                )?;
            }
        }
        let queued = unsafe {
            self.this.queue.QueueEventParamUnk(
                MEMediaSample.0 as u32,
                &windows::core::GUID::zeroed(),
                windows::core::HRESULT(0),
                &sample,
            )
        };
        if index <= 3 {
            crate::log::line(&format!(
                "stream: sample {index} queued (token={}) -> {}",
                token.as_ref().is_some(),
                if queued.is_ok() { "ok" } else { "ERROR" }
            ));
        }
        queued
    }
}

impl IMFMediaStream2_Impl for MediaStream_Impl {
    fn SetStreamState(&self, state: MF_STREAM_STATE) -> Result<()> {
        crate::log::line(&format!("stream: SetStreamState({})", state.0));
        match state {
            MF_STREAM_STATE_RUNNING => self.set_running(true),
            MF_STREAM_STATE_STOPPED | MF_STREAM_STATE_PAUSED => self.set_running(false),
            _ => Err(MF_E_INVALIDREQUEST.into()),
        }
    }

    fn GetStreamState(&self) -> Result<MF_STREAM_STATE> {
        let state = self.this.state.lock().expect("stream state");
        if state.shutdown {
            return Err(MF_E_SHUTDOWN.into());
        }
        Ok(if state.running {
            MF_STREAM_STATE_RUNNING
        } else {
            MF_STREAM_STATE_STOPPED
        })
    }
}

impl_event_generator!(MediaStream_Impl, queue);
impl_attributes!(MediaStream_Impl, attributes);
