//! A client mod's retained GPU storage buffers (`ClientRenderClientHost::create_storage_buffer`,
//! `StorageBufferRef::write_range`, `size` and `release`): vanilla's `GpuBuffer` written through
//! `CommandEncoder.writeToBuffer(GpuBufferSlice, ByteBuffer)` and bound by range
//! (`GpuBuffer.slice`, `GpuBufferSlice`).

/// Generational handle to a retained GPU storage buffer, created via
/// [`crate::prelude::ClientRenderClientHost::create_storage_buffer`].
///
/// Handles are owner-checked. The buffer outlives a resource reload and a world close, and is
/// freed by [`Self::release`] or when its mod unloads.
// no wire: process-local generational handle to a retained GPU storage buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: only the engine creates handles.
pub struct StorageBufferRef(pub(crate) u64, pub(crate) u32);

impl StorageBufferRef {
    pub(crate) fn from_parts(generation: u64, slot: u32) -> Result<Self, &'static str> {
        if generation == 0 {
            return Err("unpublished storage buffer generation");
        }
        Ok(Self(generation, slot))
    }

    /// The range `offset..offset + length` of this buffer, to bind with
    /// [`crate::DrawBindings::storage_buffer`] (`GpuBuffer.slice`). The range is checked when
    /// it is bound, not here.
    pub const fn slice(self, offset: u64, length: u64) -> StorageBufferSlice {
        StorageBufferSlice {
            buffer: self,
            offset,
            length,
        }
    }
}

impl crate::__private::game::Fields for StorageBufferRef {
    type Fields<'a> = crate::__private::game::ModelRefFields;
    fn fields(&self) -> Self::Fields<'_> {
        crate::__private::game::ModelRefFields {
            generation: self.0,
            slot: self.1,
        }
    }
}

/// A byte range of a [`StorageBufferRef`] (vanilla's `GpuBufferSlice`).
///
/// A binding takes a range of at most the device's `max_storage_buffer_binding_size` bytes, its
/// offset a multiple of `min_storage_buffer_offset_alignment`, its length a non-zero multiple of
/// 4. The buffer itself may be as large as `max_buffer_size`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
// no wire: local client storage buffer range.
pub struct StorageBufferSlice {
    /// The buffer the range lies in.
    pub buffer: StorageBufferRef,
    /// First byte of the range.
    pub offset: u64,
    /// Length of the range in bytes.
    pub length: u64,
}

/// What [`crate::prelude::ClientRenderClientHost::create_storage_buffer`] makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
// no wire: local client storage buffer descriptor.
pub struct StorageBufferDescriptor {
    /// Size in bytes: at least 4, a multiple of 4 and at most the device's `max_buffer_size`.
    pub size: u64,
    /// Whether the buffer can also hold indirect draw or dispatch arguments (`wgpu`'s
    /// `BufferUsages::INDIRECT`; vanilla's `USAGE_INDIRECT_PARAMETERS`). Off by default.
    pub indirect: bool,
}

impl StorageBufferDescriptor {
    /// A storage buffer of `size` bytes.
    pub const fn new(size: u64) -> Self {
        Self {
            size,
            indirect: false,
        }
    }

    /// Let the buffer also hold indirect draw or dispatch arguments.
    pub const fn indirect(mut self, indirect: bool) -> Self {
        self.indirect = indirect;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_descriptor_starts_direct_and_a_slice_names_its_range() {
        let desc = StorageBufferDescriptor::new(64);
        assert_eq!((desc.size, desc.indirect), (64, false));
        assert!(desc.indirect(true).indirect);
        let buffer = StorageBufferRef::from_parts(3, 1).unwrap();
        let slice = buffer.slice(16, 32);
        assert_eq!((slice.buffer, slice.offset, slice.length), (buffer, 16, 32));
        assert!(StorageBufferRef::from_parts(0, 0).is_err());
    }
}
