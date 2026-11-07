//! Macros that forward COM interfaces to objects Windows creates for us.
//!
//! `IMFAttributes` has 30 methods and `IMFMediaEventGenerator` has four, and both the source
//! and the stream object must implement them. Rather than writing all of that out twice, we
//! forward to the objects `MFCreateAttributes` and `MFCreateEventQueue` hand back.
//!
//! The two sides do not share signatures: the implementing side takes raw pointers while the
//! calling side uses `Option`, slices and generics. The conversions live here, in one place.

/// Forwards `IMFAttributes` to an inner attribute store.
/// `$get` must name a method returning `&IMFAttributes`.
macro_rules! impl_attributes {
    ($target:ty, $get:ident) => {
        impl windows::Win32::Media::MediaFoundation::IMFAttributes_Impl for $target {
            fn GetItem(
                &self,
                key: *const windows::core::GUID,
                value: *mut windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
            ) -> windows::core::Result<()> {
                unsafe {
                    self.$get()
                        .GetItem(key, (!value.is_null()).then_some(value))
                }
            }

            fn GetItemType(
                &self,
                key: *const windows::core::GUID,
            ) -> windows::core::Result<windows::Win32::Media::MediaFoundation::MF_ATTRIBUTE_TYPE>
            {
                unsafe { self.$get().GetItemType(key) }
            }

            fn CompareItem(
                &self,
                key: *const windows::core::GUID,
                value: *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
            ) -> windows::core::Result<windows::core::BOOL> {
                unsafe { self.$get().CompareItem(key, value) }
            }

            fn Compare(
                &self,
                theirs: windows::core::Ref<
                    '_,
                    windows::Win32::Media::MediaFoundation::IMFAttributes,
                >,
                match_type: windows::Win32::Media::MediaFoundation::MF_ATTRIBUTES_MATCH_TYPE,
            ) -> windows::core::Result<windows::core::BOOL> {
                unsafe { self.$get().Compare(theirs.as_ref(), match_type) }
            }

            fn GetUINT32(&self, key: *const windows::core::GUID) -> windows::core::Result<u32> {
                unsafe { self.$get().GetUINT32(key) }
            }

            fn GetUINT64(&self, key: *const windows::core::GUID) -> windows::core::Result<u64> {
                unsafe { self.$get().GetUINT64(key) }
            }

            fn GetDouble(&self, key: *const windows::core::GUID) -> windows::core::Result<f64> {
                unsafe { self.$get().GetDouble(key) }
            }

            fn GetGUID(
                &self,
                key: *const windows::core::GUID,
            ) -> windows::core::Result<windows::core::GUID> {
                unsafe { self.$get().GetGUID(key) }
            }

            fn GetStringLength(
                &self,
                key: *const windows::core::GUID,
            ) -> windows::core::Result<u32> {
                unsafe { self.$get().GetStringLength(key) }
            }

            fn GetString(
                &self,
                key: *const windows::core::GUID,
                buffer: windows::core::PWSTR,
                capacity: u32,
                length: *mut u32,
            ) -> windows::core::Result<()> {
                // The callee wants a slice, so build one from the pointer and capacity.
                let slice = unsafe { std::slice::from_raw_parts_mut(buffer.0, capacity as usize) };
                unsafe {
                    self.$get()
                        .GetString(key, slice, (!length.is_null()).then_some(length))
                }
            }

            fn GetAllocatedString(
                &self,
                key: *const windows::core::GUID,
                value: *mut windows::core::PWSTR,
                length: *mut u32,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().GetAllocatedString(key, value, length) }
            }

            fn GetBlobSize(&self, key: *const windows::core::GUID) -> windows::core::Result<u32> {
                unsafe { self.$get().GetBlobSize(key) }
            }

            fn GetBlob(
                &self,
                key: *const windows::core::GUID,
                buffer: *mut u8,
                capacity: u32,
                written: *mut u32,
            ) -> windows::core::Result<()> {
                let slice = unsafe { std::slice::from_raw_parts_mut(buffer, capacity as usize) };
                unsafe {
                    self.$get()
                        .GetBlob(key, slice, (!written.is_null()).then_some(written))
                }
            }

            fn GetAllocatedBlob(
                &self,
                key: *const windows::core::GUID,
                buffer: *mut *mut u8,
                size: *mut u32,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().GetAllocatedBlob(key, buffer, size) }
            }

            fn GetUnknown(
                &self,
                key: *const windows::core::GUID,
                interface_id: *const windows::core::GUID,
                out: *mut *mut core::ffi::c_void,
            ) -> windows::core::Result<()> {
                // The callee is generic, so fetch IUnknown and query for the requested interface.
                let found: windows::core::Result<windows::core::IUnknown> =
                    unsafe { self.$get().GetUnknown(key) };
                let unknown = found?;
                unsafe { windows::core::Interface::query(&unknown, interface_id, out).ok() }
            }

            fn SetItem(
                &self,
                key: *const windows::core::GUID,
                value: *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().SetItem(key, value) }
            }

            fn DeleteItem(&self, key: *const windows::core::GUID) -> windows::core::Result<()> {
                unsafe { self.$get().DeleteItem(key) }
            }

            fn DeleteAllItems(&self) -> windows::core::Result<()> {
                unsafe { self.$get().DeleteAllItems() }
            }

            fn SetUINT32(
                &self,
                key: *const windows::core::GUID,
                value: u32,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().SetUINT32(key, value) }
            }

            fn SetUINT64(
                &self,
                key: *const windows::core::GUID,
                value: u64,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().SetUINT64(key, value) }
            }

            fn SetDouble(
                &self,
                key: *const windows::core::GUID,
                value: f64,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().SetDouble(key, value) }
            }

            fn SetGUID(
                &self,
                key: *const windows::core::GUID,
                value: *const windows::core::GUID,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().SetGUID(key, value) }
            }

            fn SetString(
                &self,
                key: *const windows::core::GUID,
                value: &windows::core::PCWSTR,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().SetString(key, *value) }
            }

            fn SetBlob(
                &self,
                key: *const windows::core::GUID,
                buffer: *const u8,
                size: u32,
            ) -> windows::core::Result<()> {
                let slice = unsafe { std::slice::from_raw_parts(buffer, size as usize) };
                unsafe { self.$get().SetBlob(key, slice) }
            }

            fn SetUnknown(
                &self,
                key: *const windows::core::GUID,
                value: windows::core::Ref<'_, windows::core::IUnknown>,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().SetUnknown(key, value.as_ref()) }
            }

            fn LockStore(&self) -> windows::core::Result<()> {
                unsafe { self.$get().LockStore() }
            }

            fn UnlockStore(&self) -> windows::core::Result<()> {
                unsafe { self.$get().UnlockStore() }
            }

            fn GetCount(&self) -> windows::core::Result<u32> {
                unsafe { self.$get().GetCount() }
            }

            fn GetItemByIndex(
                &self,
                index: u32,
                key: *mut windows::core::GUID,
                value: *mut windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
            ) -> windows::core::Result<()> {
                unsafe {
                    self.$get()
                        .GetItemByIndex(index, key, (!value.is_null()).then_some(value))
                }
            }

            fn CopyAllItems(
                &self,
                dest: windows::core::Ref<'_, windows::Win32::Media::MediaFoundation::IMFAttributes>,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().CopyAllItems(dest.as_ref()) }
            }
        }
    };
}

/// Forwards `IMFMediaEventGenerator` to a Windows event queue.
/// `$get` must name a method returning `&IMFMediaEventQueue`.
macro_rules! impl_event_generator {
    ($target:ty, $get:ident) => {
        impl windows::Win32::Media::MediaFoundation::IMFMediaEventGenerator_Impl for $target {
            fn GetEvent(
                &self,
                flags: windows::Win32::Media::MediaFoundation::MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS,
            ) -> windows::core::Result<windows::Win32::Media::MediaFoundation::IMFMediaEvent>
            {
                unsafe { self.$get().GetEvent(flags.0 as u32) }
            }

            fn BeginGetEvent(
                &self,
                callback: windows::core::Ref<
                    '_,
                    windows::Win32::Media::MediaFoundation::IMFAsyncCallback,
                >,
                state: windows::core::Ref<'_, windows::core::IUnknown>,
            ) -> windows::core::Result<()> {
                unsafe { self.$get().BeginGetEvent(callback.as_ref(), state.as_ref()) }
            }

            fn EndGetEvent(
                &self,
                result: windows::core::Ref<
                    '_,
                    windows::Win32::Media::MediaFoundation::IMFAsyncResult,
                >,
            ) -> windows::core::Result<windows::Win32::Media::MediaFoundation::IMFMediaEvent>
            {
                unsafe { self.$get().EndGetEvent(result.as_ref()) }
            }

            fn QueueEvent(
                &self,
                event_type: u32,
                extended_type: *const windows::core::GUID,
                status: windows::core::HRESULT,
                value: *const windows::Win32::System::Com::StructuredStorage::PROPVARIANT,
            ) -> windows::core::Result<()> {
                unsafe {
                    self.$get().QueueEventParamVar(event_type, extended_type, status, value)
                }
            }
        }
    };
}

pub(crate) use {impl_attributes, impl_event_generator};
