//! The chain of images that are drawn into and then shown in the window.

use std::sync::Arc;

use ash::vk;

use crate::error::RenderError;
use crate::gpu::Gpu;

/// Picks the colour format: 8-bit sRGB if the window offers it, so colours
/// are blended in linear light and shown correctly.
fn choose_format(offered: &[vk::SurfaceFormatKHR]) -> Option<vk::SurfaceFormatKHR> {
    let srgb = |wanted: vk::Format| {
        offered.iter().copied().find(|format| {
            format.format == wanted && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
    };
    srgb(vk::Format::B8G8R8A8_SRGB)
        .or_else(|| srgb(vk::Format::R8G8B8A8_SRGB))
        .or_else(|| offered.first().copied())
}

/// Picks how finished frames reach the screen.
///
/// With vsync, frames wait for the display (no tearing, always available).
/// Without it, the newest frame replaces a waiting one if the window system
/// can do that without tearing, and otherwise frames are shown immediately.
fn choose_present_mode(offered: &[vk::PresentModeKHR], vsync: bool) -> vk::PresentModeKHR {
    if vsync {
        return vk::PresentModeKHR::FIFO;
    }
    [vk::PresentModeKHR::MAILBOX, vk::PresentModeKHR::IMMEDIATE]
        .into_iter()
        .find(|mode| offered.contains(mode))
        .unwrap_or(vk::PresentModeKHR::FIFO)
}

/// Picks the image size, or `None` if there is nothing to draw into (a minimised window).
fn choose_extent(
    capabilities: &vk::SurfaceCapabilitiesKHR,
    window: (u32, u32),
) -> Option<vk::Extent2D> {
    let (least, most) = (capabilities.min_image_extent, capabilities.max_image_extent);
    // Most window systems dictate the size. Wayland reports u32::MAX instead,
    // meaning the swapchain decides, so the window's own size is used.
    let extent = if capabilities.current_extent.width == u32::MAX {
        vk::Extent2D {
            // `max` first so a driver reporting most < least cannot make `clamp` panic.
            width: window.0.max(least.width).min(most.width),
            height: window.1.max(least.height).min(most.height),
        }
    } else {
        capabilities.current_extent
    };
    (extent.width > 0 && extent.height > 0).then_some(extent)
}

/// Picks how many images to ask for: one more than the minimum, so the
/// renderer never has to wait for the driver to hand one back.
fn choose_image_count(capabilities: &vk::SurfaceCapabilitiesKHR) -> u32 {
    let wanted = capabilities.min_image_count.saturating_add(1);
    // A maximum of 0 means there is no limit.
    if capabilities.max_image_count == 0 {
        wanted
    } else {
        wanted.min(capabilities.max_image_count)
    }
}

/// Picks how the window is blended with what is behind it: not at all, if possible.
fn choose_alpha(supported: vk::CompositeAlphaFlagsKHR) -> vk::CompositeAlphaFlagsKHR {
    [
        vk::CompositeAlphaFlagsKHR::OPAQUE,
        vk::CompositeAlphaFlagsKHR::INHERIT,
        vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
        vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
    ]
    .into_iter()
    .find(|&mode| supported.contains(mode))
    .unwrap_or(vk::CompositeAlphaFlagsKHR::OPAQUE)
}

/// One image of the swapchain and what is needed to draw into and show it.
pub(crate) struct Target {
    pub(crate) image: vk::Image,
    pub(crate) view: vk::ImageView,
    /// Signalled when drawing into this image has finished.
    pub(crate) drawn: vk::Semaphore,
}

/// The images shown in the window.
pub(crate) struct Swapchain {
    gpu: Arc<Gpu>,
    pub(crate) handle: vk::SwapchainKHR,
    pub(crate) format: vk::Format,
    pub(crate) extent: vk::Extent2D,
    pub(crate) targets: Vec<Target>,
}

impl Swapchain {
    /// Creates images matching the window as it is now.
    ///
    /// Returns `None` when the window has no area to draw into. `old` is the
    /// swapchain being replaced, if any; it must stay alive until this returns
    /// and can be dropped afterwards.
    pub(crate) fn new(
        gpu: &Arc<Gpu>,
        window: (u32, u32),
        vsync: bool,
        old: Option<&Self>,
    ) -> Result<Option<Self>, RenderError> {
        let (surfaces, surface) = gpu.surface();
        let physical = gpu.physical_device();
        // SAFETY: `surface` and `physical` belong to `gpu`'s instance, which
        // is alive because `gpu` is.
        let (capabilities, formats, modes) = unsafe {
            (
                surfaces
                    .get_physical_device_surface_capabilities(physical, surface)
                    .map_err(RenderError::driver("reading the window's size"))?,
                surfaces
                    .get_physical_device_surface_formats(physical, surface)
                    .map_err(RenderError::driver("reading the window's colour formats"))?,
                surfaces
                    .get_physical_device_surface_present_modes(physical, surface)
                    .map_err(RenderError::driver("reading the window's display modes"))?,
            )
        };
        let Some(extent) = choose_extent(&capabilities, window) else {
            return Ok(None);
        };
        let Some(format) = choose_format(&formats) else {
            return Err(RenderError::Driver {
                doing: "reading the window's colour formats",
                code: vk::Result::ERROR_FORMAT_NOT_SUPPORTED,
            });
        };

        let info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(choose_image_count(&capabilities))
            .image_format(format.format)
            .image_color_space(format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(capabilities.current_transform)
            .composite_alpha(choose_alpha(capabilities.supported_composite_alpha))
            .present_mode(choose_present_mode(&modes, vsync))
            .clipped(true)
            .old_swapchain(old.map_or(vk::SwapchainKHR::null(), |old| old.handle));
        // SAFETY: every value in `info` was chosen from what the surface
        // reported, and `old`, if given, is a live swapchain for this surface.
        let handle = unsafe { gpu.swapchains().create_swapchain(&info, None) }
            .map_err(RenderError::driver("creating the window's images"))?;

        let mut swapchain = Self {
            gpu: Arc::clone(gpu),
            handle,
            format: format.format,
            extent,
            targets: Vec::new(),
        };
        // From here on, dropping `swapchain` cleans up whatever was created.

        // SAFETY: `handle` is the swapchain just created on this device.
        let images = unsafe { gpu.swapchains().get_swapchain_images(handle) }
            .map_err(RenderError::driver("creating the window's images"))?;
        swapchain.targets.reserve_exact(images.len());
        for image in images {
            let info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(format.format)
                .subresource_range(COLOUR);
            // SAFETY: `image` belongs to the live swapchain, and the view
            // matches its format. The semaphore needs nothing but a live device.
            let (view, drawn) = unsafe {
                let view = gpu
                    .device()
                    .create_image_view(&info, None)
                    .map_err(RenderError::driver("creating the window's images"))?;
                // Recorded before the next call can fail, so it is not leaked.
                swapchain.targets.push(Target {
                    image,
                    view,
                    drawn: vk::Semaphore::null(),
                });
                let drawn = gpu
                    .device()
                    .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                    .map_err(RenderError::driver("creating the window's images"))?;
                (view, drawn)
            };
            if let Some(target) = swapchain.targets.last_mut() {
                debug_assert_eq!(target.view, view);
                target.drawn = drawn;
            }
        }
        Ok(Some(swapchain))
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping a swapchain, so none of this is in use. Destroying a null
        // semaphore is allowed.
        unsafe {
            for target in &self.targets {
                self.gpu.device().destroy_image_view(target.view, None);
                self.gpu.device().destroy_semaphore(target.drawn, None);
            }
            self.gpu.swapchains().destroy_swapchain(self.handle, None);
        }
    }
}

/// The whole of a plain colour image.
pub(crate) const COLOUR: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn format(format: vk::Format) -> vk::SurfaceFormatKHR {
        vk::SurfaceFormatKHR {
            format,
            color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        }
    }

    fn capabilities(
        current: (u32, u32),
        least: (u32, u32),
        most: (u32, u32),
    ) -> vk::SurfaceCapabilitiesKHR {
        let extent = |(width, height)| vk::Extent2D { width, height };
        vk::SurfaceCapabilitiesKHR {
            current_extent: extent(current),
            min_image_extent: extent(least),
            max_image_extent: extent(most),
            ..Default::default()
        }
    }

    #[test]
    fn srgb_is_preferred_wherever_it_is_listed() {
        let offered = [
            format(vk::Format::R8G8B8A8_UNORM),
            format(vk::Format::R8G8B8A8_SRGB),
            format(vk::Format::B8G8R8A8_SRGB),
        ];
        assert_eq!(
            choose_format(&offered).map(|chosen| chosen.format),
            Some(vk::Format::B8G8R8A8_SRGB)
        );
    }

    #[test]
    fn without_srgb_the_first_format_is_used() {
        let offered = [
            format(vk::Format::A2B10G10R10_UNORM_PACK32),
            format(vk::Format::R8G8B8A8_UNORM),
        ];
        assert_eq!(
            choose_format(&offered).map(|chosen| chosen.format),
            Some(vk::Format::A2B10G10R10_UNORM_PACK32)
        );
        assert!(choose_format(&[]).is_none());
    }

    #[test]
    fn vsync_always_waits_for_the_display() {
        let offered = [vk::PresentModeKHR::IMMEDIATE, vk::PresentModeKHR::MAILBOX];
        assert_eq!(
            choose_present_mode(&offered, true),
            vk::PresentModeKHR::FIFO
        );
    }

    #[test]
    fn without_vsync_the_lowest_latency_mode_on_offer_is_used() {
        use vk::PresentModeKHR as Mode;
        assert_eq!(
            choose_present_mode(&[Mode::FIFO, Mode::IMMEDIATE, Mode::MAILBOX], false),
            Mode::MAILBOX
        );
        assert_eq!(
            choose_present_mode(&[Mode::FIFO, Mode::IMMEDIATE], false),
            Mode::IMMEDIATE
        );
        assert_eq!(choose_present_mode(&[Mode::FIFO], false), Mode::FIFO);
    }

    #[test]
    fn the_window_system_dictates_the_size_when_it_has_one() {
        let caps = capabilities((800, 600), (1, 1), (4096, 4096));
        assert_eq!(
            choose_extent(&caps, (1280, 720)),
            Some(vk::Extent2D {
                width: 800,
                height: 600
            })
        );
    }

    #[test]
    fn on_wayland_the_window_size_is_used_within_the_limits() {
        let caps = capabilities((u32::MAX, u32::MAX), (1, 1), (4096, 2160));
        assert_eq!(
            choose_extent(&caps, (1280, 720)),
            Some(vk::Extent2D {
                width: 1280,
                height: 720
            })
        );
        assert_eq!(
            choose_extent(&caps, (10_000, 10_000)),
            Some(vk::Extent2D {
                width: 4096,
                height: 2160
            })
        );
    }

    #[test]
    fn a_minimised_window_has_nothing_to_draw_into() {
        let minimised = capabilities((0, 0), (0, 0), (0, 0));
        assert_eq!(choose_extent(&minimised, (1280, 720)), None);
        let wayland = capabilities((u32::MAX, u32::MAX), (0, 0), (4096, 4096));
        assert_eq!(choose_extent(&wayland, (0, 0)), None);
        assert_eq!(choose_extent(&wayland, (1280, 0)), None);
    }

    #[test]
    fn nonsense_limits_do_not_panic() {
        let caps = capabilities((u32::MAX, u32::MAX), (64, 64), (16, 16));
        assert_eq!(
            choose_extent(&caps, (32, 32)),
            Some(vk::Extent2D {
                width: 16,
                height: 16
            })
        );
    }

    #[test]
    fn one_spare_image_is_requested_within_the_limit() {
        let count = |least, most| {
            choose_image_count(&vk::SurfaceCapabilitiesKHR {
                min_image_count: least,
                max_image_count: most,
                ..Default::default()
            })
        };
        assert_eq!(count(2, 0), 3);
        assert_eq!(count(2, 8), 3);
        assert_eq!(count(3, 3), 3);
    }

    #[test]
    fn the_window_is_opaque_when_the_window_system_allows_it() {
        use vk::CompositeAlphaFlagsKHR as Alpha;
        assert_eq!(
            choose_alpha(Alpha::OPAQUE | Alpha::PRE_MULTIPLIED),
            Alpha::OPAQUE
        );
        assert_eq!(choose_alpha(Alpha::INHERIT), Alpha::INHERIT);
    }
}
