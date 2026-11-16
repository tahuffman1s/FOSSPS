//! Memory on the graphics card that holds a mesh's points and triangles.

use std::sync::Arc;

use ash::vk;

use crate::error::RenderError;
use crate::gpu::Gpu;

/// A block of memory on the graphics card.
pub(crate) struct Buffer {
    gpu: Arc<Gpu>,
    pub(crate) handle: vk::Buffer,
    memory: vk::DeviceMemory,
}

impl Buffer {
    /// Makes an empty buffer of `size` bytes in memory with the `wanted`
    /// properties, or failing that (when `or_any` is set) any memory.
    fn new(
        gpu: &Arc<Gpu>,
        size: u64,
        usage: vk::BufferUsageFlags,
        wanted: vk::MemoryPropertyFlags,
        or_any: bool,
        doing: &'static str,
    ) -> Result<Self, RenderError> {
        let device = gpu.device();
        let mut buffer = Self {
            gpu: Arc::clone(gpu),
            handle: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
        };
        // Dropping `buffer` cleans up whatever was created; destroying a null handle is allowed.

        // SAFETY: the device is alive, and each object is created from valid
        // settings and stored in `buffer` straight away. The memory is as
        // large as the buffer asks for and of a kind it allows.
        unsafe {
            buffer.handle = device
                .create_buffer(
                    &vk::BufferCreateInfo::default().size(size).usage(usage),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            let needs = device.get_buffer_memory_requirements(buffer.handle);
            let kind = gpu
                .memory_type(needs.memory_type_bits, wanted)
                .or_else(|| {
                    or_any
                        .then(|| {
                            gpu.memory_type(
                                needs.memory_type_bits,
                                vk::MemoryPropertyFlags::empty(),
                            )
                        })
                        .flatten()
                })
                .ok_or(RenderError::Driver {
                    doing,
                    code: vk::Result::ERROR_FEATURE_NOT_PRESENT,
                })?;
            buffer.memory = device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(kind),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            device
                .bind_buffer_memory(buffer.handle, buffer.memory, 0)
                .map_err(RenderError::driver(doing))?;
        }
        Ok(buffer)
    }

    /// Makes a buffer of `size` bytes for `usage` that [`write`](Self::write)
    /// can fill every frame.
    pub(crate) fn writable(
        gpu: &Arc<Gpu>,
        size: u64,
        usage: vk::BufferUsageFlags,
        doing: &'static str,
    ) -> Result<Self, RenderError> {
        Self::new(
            gpu,
            size,
            usage,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            false,
            doing,
        )
    }

    /// Replaces the start of the buffer with `bytes`.
    ///
    /// # Safety
    ///
    /// The buffer must come from [`writable`](Self::writable) and be at
    /// least as long as `bytes`, and the graphics card must not be reading
    /// it: commands that use it must have finished.
    pub(crate) unsafe fn write(&self, bytes: &[u8]) -> Result<(), RenderError> {
        let device = self.gpu.device();
        // SAFETY: the caller guarantees the memory can be mapped, is at least
        // `bytes.len()` long and is not in use; it is unmapped straight after.
        unsafe {
            let mapped = device
                .map_memory(
                    self.memory,
                    0,
                    bytes.len() as u64,
                    vk::MemoryMapFlags::empty(),
                )
                .map_err(RenderError::driver("drawing a frame"))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), bytes.len());
            device.unmap_memory(self.memory);
        }
        Ok(())
    }

    /// The first `length` bytes of the buffer.
    ///
    /// # Safety
    ///
    /// The buffer must come from [`writable`](Self::writable) and be at
    /// least `length` bytes long, and the graphics card must not be writing
    /// it: commands that use it must have finished.
    pub(crate) unsafe fn read(&self, length: usize) -> Result<Vec<u8>, RenderError> {
        let device = self.gpu.device();
        // SAFETY: the caller guarantees the memory can be mapped, is at least
        // `length` long and is not in use; the bytes are copied out before
        // it is unmapped straight after.
        unsafe {
            let mapped = device
                .map_memory(self.memory, 0, length as u64, vk::MemoryMapFlags::empty())
                .map_err(RenderError::driver("reading a picture back"))?;
            let bytes = std::slice::from_raw_parts(mapped.cast::<u8>(), length).to_vec();
            device.unmap_memory(self.memory);
            Ok(bytes)
        }
    }

    /// Makes a buffer of `size` bytes whose contents can be copied to the
    /// graphics card's own memory once [`fill`](Self::fill) has written them.
    pub(crate) fn staging(
        gpu: &Arc<Gpu>,
        size: u64,
        doing: &'static str,
    ) -> Result<Self, RenderError> {
        Self::new(
            gpu,
            size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            false,
            doing,
        )
    }

    /// Writes `bytes` to the start of a buffer that no command has used yet.
    /// The buffer must come from [`staging`](Self::staging) or
    /// [`writable`](Self::writable); anything past its end is not written.
    pub(crate) fn fill(&self, bytes: &[u8], doing: &'static str) -> Result<(), RenderError> {
        let device = self.gpu.device();
        // SAFETY: the memory is this buffer's own, of a kind that can be
        // mapped, and nothing else is using it. No more is written than the
        // buffer was made to hold, and it is unmapped straight after.
        unsafe {
            let needs = device.get_buffer_memory_requirements(self.handle);
            let length = bytes
                .len()
                .min(usize::try_from(needs.size).unwrap_or(usize::MAX));
            let mapped = device
                .map_memory(self.memory, 0, length as u64, vk::MemoryMapFlags::empty())
                .map_err(RenderError::driver(doing))?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped.cast::<u8>(), length);
            device.unmap_memory(self.memory);
        }
        Ok(())
    }

    /// Makes a buffer for `usage` that holds `bytes`, which must not be empty.
    ///
    /// Waits for the graphics card to finish the copy, so this is for
    /// loading, not for every frame.
    pub(crate) fn holding(
        gpu: &Arc<Gpu>,
        bytes: &[u8],
        usage: vk::BufferUsageFlags,
        doing: &'static str,
    ) -> Result<Self, RenderError> {
        let device = gpu.device();
        let size = bytes.len() as u64;
        // The graphics card's own memory usually cannot be written from
        // here, so the bytes go through a buffer that can.
        let staging = Self::staging(gpu, size, doing)?;
        let buffer = Self::new(
            gpu,
            size,
            usage | vk::BufferUsageFlags::TRANSFER_DST,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
            true,
            doing,
        )?;

        staging.fill(bytes, doing)?;
        // SAFETY: both buffers are alive and `size` bytes long, and stay
        // alive until the copy, which `once` waits for, has finished.
        unsafe {
            once(gpu, doing, |commands| {
                device.cmd_copy_buffer(
                    commands,
                    staging.handle,
                    buffer.handle,
                    &[vk::BufferCopy::default().size(size)],
                );
            })
        }?;
        Ok(buffer)
    }
}

/// Runs the commands `record` writes, once, and waits for the graphics card
/// to finish them. For loading, not for every frame.
///
/// # Safety
///
/// Everything the recorded commands use must belong to `gpu`'s device, be
/// used as Vulkan allows, and stay alive until this returns.
pub(crate) unsafe fn once(
    gpu: &Gpu,
    doing: &'static str,
    record: impl FnOnce(vk::CommandBuffer),
) -> Result<(), RenderError> {
    let device = gpu.device();
    // SAFETY: the pool, command buffer and fence are created here, used by
    // the one submission below, which is waited for, and destroyed before
    // returning. The caller answers for what the commands themselves use.
    unsafe {
        let pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default()
                    .flags(vk::CommandPoolCreateFlags::TRANSIENT)
                    .queue_family_index(gpu.queue_family()),
                None,
            )
            .map_err(RenderError::driver(doing))?;
        let fence = match device.create_fence(&vk::FenceCreateInfo::default(), None) {
            Ok(fence) => fence,
            Err(code) => {
                device.destroy_command_pool(pool, None);
                return Err(RenderError::driver(doing)(code));
            }
        };
        let ran = (|| {
            let commands = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )?
                .first()
                .copied()
                .ok_or(vk::Result::ERROR_UNKNOWN)?;
            device.begin_command_buffer(
                commands,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            record(commands);
            device.end_command_buffer(commands)?;
            let buffers = [vk::CommandBufferSubmitInfo::default().command_buffer(commands)];
            device.queue_submit2(
                gpu.queue(),
                &[vk::SubmitInfo2::default().command_buffer_infos(&buffers)],
                fence,
            )?;
            device.wait_for_fences(&[fence], true, u64::MAX)
        })();
        if ran.is_err() {
            // The commands may have been submitted; nothing may be freed under them.
            let _ = device.device_wait_idle();
        }
        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        ran.map_err(RenderError::driver(doing))
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping the buffer, so nothing is using it.
        unsafe {
            let device = self.gpu.device();
            device.destroy_buffer(self.handle, None);
            device.free_memory(self.memory, None);
        }
    }
}
