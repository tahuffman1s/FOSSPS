//! The image the scene is drawn into before it is tone mapped for the display.

use std::sync::Arc;

use ash::vk;

use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::swapchain::COLOUR;

/// A high-dynamic-range colour image the size of the picture.
///
/// Light is stored as it is, with no upper limit, so the sun can be thousands
/// of times brighter than a shadow; [`TonemapPipeline`](crate::tonemap::TonemapPipeline)
/// turns it into colours a display can show.
pub(crate) struct SceneTarget {
    gpu: Arc<Gpu>,
    pub(crate) image: vk::Image,
    memory: vk::DeviceMemory,
    pub(crate) view: vk::ImageView,
    pub(crate) extent: vk::Extent2D,
}

impl SceneTarget {
    /// Sixteen-bit floats for red, green and blue.
    ///
    /// Every Vulkan 1.3 graphics card can draw into and read this format. The
    /// smaller `B10G11R11` has too few steps for upscaling and anti-aliasing
    /// to work on without banding.
    pub(crate) const FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

    /// Makes a target of `extent` pixels.
    pub(crate) fn new(gpu: &Arc<Gpu>, extent: vk::Extent2D) -> Result<Self, RenderError> {
        let doing = "making the image the scene is drawn into";
        let device = gpu.device();
        let mut target = Self {
            gpu: Arc::clone(gpu),
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            view: vk::ImageView::null(),
            extent,
        };
        // Dropping `target` cleans up whatever was created; destroying a null handle is allowed.

        // SAFETY: the device is alive, and each object is created from valid
        // settings and stored in `target` straight away. The memory is as
        // large as the image asks for and of a kind it allows.
        unsafe {
            target.image = device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(Self::FORMAT)
                        .extent(extent.into())
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .usage(
                            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
                        ),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            let needs = device.get_image_memory_requirements(target.image);
            let kind = gpu
                .memory_type(
                    needs.memory_type_bits,
                    vk::MemoryPropertyFlags::DEVICE_LOCAL,
                )
                .or_else(|| {
                    gpu.memory_type(needs.memory_type_bits, vk::MemoryPropertyFlags::empty())
                })
                .ok_or(RenderError::Driver {
                    doing,
                    code: vk::Result::ERROR_FEATURE_NOT_PRESENT,
                })?;
            target.memory = device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(kind),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            device
                .bind_image_memory(target.image, target.memory, 0)
                .map_err(RenderError::driver(doing))?;
            target.view = device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(target.image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(Self::FORMAT)
                        .subresource_range(COLOUR),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
        }
        Ok(target)
    }
}

impl Drop for SceneTarget {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping the target, so nothing is using it.
        unsafe {
            let device = self.gpu.device();
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}
