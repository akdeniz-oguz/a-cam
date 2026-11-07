//! The virtual camera's media source.
//!
//! The Frame Server expects `IMFMediaSourceEx` (and therefore `IMFMediaSource` and
//! `IMFMediaEventGenerator`) plus `IMFGetService`. `IMFActivate` and `IMFAttributes` live on
//! the same object, which is the arrangement Microsoft recommends.

use crate::delegate::{impl_attributes, impl_event_generator};
use crate::format;
use crate::stream::MediaStream;
use std::sync::Mutex;
use windows::Win32::Media::KernelStreaming::{KSCAMERAPROFILE_Legacy, PINNAME_VIDEO_CAPTURE};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFActivate_Impl, IMFAttributes, IMFGetService, IMFGetService_Impl,
    IMFMediaEventQueue, IMFMediaSource, IMFMediaSource_Impl, IMFMediaSourceEx,
    IMFMediaSourceEx_Impl, IMFMediaStream2, IMFPresentationDescriptor, MENewStream,
    MESourceStarted, MESourceStopped, MEUpdatedStream, MF_DEVICEMFT_SENSORPROFILE_COLLECTION,
    MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES, MF_DEVICESTREAM_FRAMESERVER_SHARED,
    MF_DEVICESTREAM_STREAM_CATEGORY, MF_DEVICESTREAM_STREAM_ID,
    MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, MF_E_SHUTDOWN, MF_E_UNSUPPORTED_SERVICE,
    MFCreateAttributes, MFCreateEventQueue, MFCreatePresentationDescriptor, MFCreateSensorProfile,
    MFCreateSensorProfileCollection, MFCreateStreamDescriptor, MFFrameSourceTypes_Color,
    MFMEDIASOURCE_IS_LIVE,
};
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::core::{GUID, HRESULT, IUnknown, Interface, Ref, Result, implement};
use windows::core::{PCWSTR, w};

struct State {
    shutdown: bool,
    started: bool,
}

#[implement(IMFMediaSourceEx, IMFGetService, IMFActivate)]
pub struct MediaSource {
    attributes: IMFAttributes,
    stream_attributes: IMFAttributes,
    queue: IMFMediaEventQueue,
    descriptor: IMFPresentationDescriptor,
    /// Set up after the source COM object exists, since the two reference each other.
    stream: Mutex<Option<IMFMediaStream2>>,
    state: Mutex<State>,
}

/// Builds the source and its stream, returning the COM interface.
pub fn create() -> Result<IUnknown> {
    unsafe {
        let mut attributes = None;
        MFCreateAttributes(&mut attributes, 4)?;
        let attributes = attributes.expect("attributes");

        let mut stream_attributes = None;
        MFCreateAttributes(&mut stream_attributes, 4)?;
        let stream_attributes = stream_attributes.expect("stream attributes");

        // The Frame Server reads these from the stream. Without them, starting the camera
        // fails with 0xC00D36E6 (attribute not found).
        // Mandatory: the stream category and the stream id.
        stream_attributes.SetGUID(&MF_DEVICESTREAM_STREAM_CATEGORY, &PINNAME_VIDEO_CAPTURE)?;
        stream_attributes.SetUINT32(&MF_DEVICESTREAM_STREAM_ID, 0)?;
        // Recommended: that this is colour video and can be opened in shared mode.
        stream_attributes.SetUINT32(
            &MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES,
            MFFrameSourceTypes_Color.0 as u32,
        )?;
        stream_attributes.SetUINT32(&MF_DEVICESTREAM_FRAMESERVER_SHARED, 1)?;
        attributes.SetUINT32(
            &MF_DEVICESTREAM_ATTRIBUTE_FRAMESOURCE_TYPES,
            MFFrameSourceTypes_Color.0 as u32,
        )?;
        attributes.SetString(&MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME, w!("a-Cam (phone)"))?;

        // The Frame Server asks the source for a sensor profile collection. Per Microsoft's
        // documentation the Legacy profile is MANDATORY, so that profile-unaware apps can
        // still open the camera. Without it Windows built the source, read the descriptors,
        // and then gave up without ever calling Start.
        let profiles = MFCreateSensorProfileCollection()?;
        let legacy = MFCreateSensorProfile(&KSCAMERAPROFILE_Legacy, 0, PCWSTR::null())?;
        // Filter string: any resolution, up to 30 fps, any subtype.
        legacy.AddProfileFilter(0, w!("((RES==;FRT<=30,1;SUT==))"))?;
        profiles.AddProfile(&legacy)?;
        attributes.SetUnknown(&MF_DEVICEMFT_SENSORPROFILE_COLLECTION, &profiles)?;

        let media_type = format::media_type()?;
        // The arrays hold Option<...>: Media Foundation allows empty entries in these lists.
        let stream_descriptor = MFCreateStreamDescriptor(0, &[Some(media_type.clone())])?;
        stream_descriptor
            .GetMediaTypeHandler()?
            .SetCurrentMediaType(&media_type)?;
        let descriptor = MFCreatePresentationDescriptor(Some(&[Some(stream_descriptor.clone())]))?;
        descriptor.SelectStream(0)?;

        let source = MediaSource {
            attributes,
            stream_attributes: stream_attributes.clone(),
            queue: MFCreateEventQueue()?,
            descriptor,
            stream: Mutex::new(None),
            state: Mutex::new(State {
                shutdown: false,
                started: false,
            }),
        };

        // The stream holds a reference to the source, so create the COM object first and
        // then place the stream inside it.
        let object = windows::core::ComObject::new(source);
        let as_source: IMFMediaSource = object.cast()?;
        let stream = MediaStream::new(as_source, stream_descriptor, stream_attributes)?;
        let stream: IMFMediaStream2 = windows::core::ComObject::new(stream).cast()?;
        *object.stream.lock().expect("stream slot") = Some(stream);

        object.cast()
    }
}

impl MediaSource_Impl {
    fn queue(&self) -> &IMFMediaEventQueue {
        &self.this.queue
    }

    fn attributes(&self) -> &IMFAttributes {
        &self.this.attributes
    }

    fn stream(&self) -> Result<IMFMediaStream2> {
        self.this
            .stream
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| MF_E_SHUTDOWN.into())
    }

    fn check_running(&self) -> Result<()> {
        let state = self.this.state.lock().expect("source state");
        if state.shutdown {
            Err(MF_E_SHUTDOWN.into())
        } else {
            Ok(())
        }
    }
}

impl IMFMediaSource_Impl for MediaSource_Impl {
    fn GetCharacteristics(&self) -> Result<u32> {
        self.check_running()?;
        // A live source: no seeking and no pausing.
        Ok(MFMEDIASOURCE_IS_LIVE.0 as u32)
    }

    fn CreatePresentationDescriptor(&self) -> Result<IMFPresentationDescriptor> {
        self.check_running()?;
        unsafe { self.this.descriptor.Clone() }
    }

    fn Start(
        &self,
        _descriptor: Ref<'_, IMFPresentationDescriptor>,
        _time_format: *const GUID,
        start_position: *const PROPVARIANT,
    ) -> Result<()> {
        crate::log::line("source: Start");
        self.check_running()?;
        let stream = self.stream()?;
        let first_start = {
            let mut state = self.this.state.lock().expect("source state");
            let first = !state.started;
            state.started = true;
            first
        };

        unsafe {
            // The event value is a COM object, so the ...ParamUnk form is used rather than
            // packing it into a PROPVARIANT.
            // The first start expects MENewStream, later ones MEUpdatedStream. Sending
            // nothing on a restart left the consumer waiting.
            let value: IUnknown = stream.cast()?;
            self.this.queue.QueueEventParamUnk(
                if first_start {
                    MENewStream.0 as u32
                } else {
                    MEUpdatedStream.0 as u32
                },
                &GUID::zeroed(),
                HRESULT(0),
                &value,
            )?;
            stream
                .SetStreamState(windows::Win32::Media::MediaFoundation::MF_STREAM_STATE_RUNNING)?;
            self.this.queue.QueueEventParamVar(
                MESourceStarted.0 as u32,
                &GUID::zeroed(),
                HRESULT(0),
                start_position,
            )
        }
    }

    fn Stop(&self) -> Result<()> {
        self.check_running()?;
        let stream = self.stream()?;
        unsafe {
            stream
                .SetStreamState(windows::Win32::Media::MediaFoundation::MF_STREAM_STATE_STOPPED)?;
            self.this.queue.QueueEventParamVar(
                MESourceStopped.0 as u32,
                &GUID::zeroed(),
                HRESULT(0),
                std::ptr::null(),
            )
        }
    }

    fn Pause(&self) -> Result<()> {
        // A live source cannot be paused.
        Err(windows::Win32::Media::MediaFoundation::MF_E_INVALIDREQUEST.into())
    }

    fn Shutdown(&self) -> Result<()> {
        {
            let mut state = self.this.state.lock().expect("source state");
            if state.shutdown {
                return Ok(());
            }
            state.shutdown = true;
        }
        if let Ok(mut slot) = self.this.stream.lock() {
            slot.take();
        }
        unsafe {
            let _ = self.this.queue.Shutdown();
        }
        Ok(())
    }
}

impl IMFMediaSourceEx_Impl for MediaSource_Impl {
    fn GetSourceAttributes(&self) -> Result<IMFAttributes> {
        self.check_running()?;
        Ok(self.this.attributes.clone())
    }

    fn GetStreamAttributes(&self, _stream_id: u32) -> Result<IMFAttributes> {
        self.check_running()?;
        Ok(self.this.stream_attributes.clone())
    }

    fn SetD3DManager(&self, _manager: Ref<'_, IUnknown>) -> Result<()> {
        // Frames are produced on the CPU, so no Direct3D manager is needed.
        Ok(())
    }
}

impl IMFGetService_Impl for MediaSource_Impl {
    fn GetService(
        &self,
        _service: *const GUID,
        interface_id: *const GUID,
        _out: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        unsafe {
            crate::log::line(&format!(
                "source: GetService(service={:?}, iid={:?}) -> unsupported",
                if _service.is_null() {
                    GUID::zeroed()
                } else {
                    *_service
                },
                if interface_id.is_null() {
                    GUID::zeroed()
                } else {
                    *interface_id
                },
            ));
        }
        // No extra services; the documentation says to return this error in that case.
        Err(MF_E_UNSUPPORTED_SERVICE.into())
    }
}

impl IMFActivate_Impl for MediaSource_Impl {
    fn ActivateObject(
        &self,
        interface_id: *const GUID,
        out: *mut *mut core::ffi::c_void,
    ) -> Result<()> {
        // IUnknownImpl exposes our object's own QueryInterface directly.
        unsafe { windows::core::IUnknownImpl::QueryInterface(self, interface_id, out).ok() }
    }

    fn DetachObject(&self) -> Result<()> {
        Ok(())
    }

    fn ShutdownObject(&self) -> Result<()> {
        IMFMediaSource_Impl::Shutdown(self)
    }
}

impl_event_generator!(MediaSource_Impl, queue);
impl_attributes!(MediaSource_Impl, attributes);
