//! Draws the scene smaller than the window and upscales it with FSR 3.1.

use std::sync::Arc;

use ash::vk;
use fossps_fsr::{Frame, FsrError, Image, Quality, Upscaler};

use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::scene::{Picture, SceneTarget};
use crate::swapchain::COLOUR;

/// How much smaller than the window the scene is drawn, before AMD's FSR 3.1
/// turns it into a full-size picture.
///
/// Drawing fewer pixels is faster. FSR recovers most of the detail from the
/// frames before, and smooths jagged edges while it is at it. The names are
/// the ones players know from other games' settings.
///
/// ```
/// use fossps_render::{RenderConfig, Upscaling};
///
/// // The fastest setting that still looks good on a 1080p display.
/// let config = RenderConfig {
///     upscaling: Upscaling::Balanced,
///     ..RenderConfig::default()
/// };
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Upscaling {
    /// Drawn at full size with nothing done to it. Edges are jagged.
    Off,
    /// Drawn at full size; FSR only smooths the edges. The best picture
    /// and the most work for the graphics card.
    Native,
    /// 1.5 times smaller each way. Hard to tell from full size. The default.
    #[default]
    Quality,
    /// 1.7 times smaller each way.
    Balanced,
    /// 2 times smaller each way: a quarter of the pixels.
    Performance,
    /// 3 times smaller each way: a ninth of the pixels. Visibly softer;
    /// meant for 4K displays and slow graphics cards.
    UltraPerformance,
}

impl Upscaling {
    fn quality(self) -> Option<Quality> {
        match self {
            Self::Off => None,
            Self::Native => Some(Quality::NativeAa),
            Self::Quality => Some(Quality::Quality),
            Self::Balanced => Some(Quality::Balanced),
            Self::Performance => Some(Quality::Performance),
            Self::UltraPerformance => Some(Quality::UltraPerformance),
        }
    }
}

/// How much FSR sharpens the upscaled picture, from 0 (not at all) to 1.
///
/// Upscaling softens a picture a little, and a little sharpening gives the
/// crispness back. Much more draws bright rims around edges.
const SHARPNESS: f32 = 0.3;

/// What one frame's upscaling needs to know about how the frame was drawn.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Drawn {
    /// The camera shift the frame was drawn with, from [`Upscale::jitter`].
    pub(crate) jitter: (f32, f32),
    /// How long the last frame took, in seconds.
    pub(crate) seconds: f32,
    /// The camera's near distance in metres.
    pub(crate) near: f32,
    /// The camera's field of view from bottom to top, in radians.
    pub(crate) fov: f32,
    /// The view jumped, so the frames before must not be blended into this one.
    pub(crate) cut: bool,
}

/// The upscaler and the full-size picture it writes.
pub(crate) struct Upscale {
    gpu: Arc<Gpu>,
    fsr: Upscaler,
    /// The upscaled picture, in high dynamic range like the scene.
    pub(crate) picture: Picture,
    /// The size of `picture`.
    output: vk::Extent2D,
    /// The size the scene is drawn at.
    pub(crate) render: vk::Extent2D,
}

impl Upscale {
    const USAGE: vk::ImageUsageFlags = vk::ImageUsageFlags::from_raw(
        vk::ImageUsageFlags::STORAGE.as_raw() | vk::ImageUsageFlags::SAMPLED.as_raw(),
    );

    /// Makes an upscaler writing pictures of `output` pixels, or `None` when
    /// `upscaling` is off or cannot be had, in which case the reason is logged
    /// and the scene is to be drawn at full size.
    pub(crate) fn new(
        gpu: &Arc<Gpu>,
        output: vk::Extent2D,
        upscaling: Upscaling,
    ) -> Result<Option<Self>, RenderError> {
        let Some(quality) = upscaling.quality() else {
            return Ok(None);
        };
        if !gpu.can_upscale() {
            log::warn!(
                "{} cannot run FSR upscaling, so the game is drawn at full size. \
                 Updating the graphics driver may help.",
                gpu.name()
            );
            return Ok(None);
        }
        let render = quality.render_size(output);
        // SAFETY: the device was created from this instance and card with
        // Vulkan 1.3 and, as `can_upscale` says, the upscaler's features. The
        // owner keeps the `Gpu` alive longer than this and waits for the
        // card to go idle before dropping it.
        let fsr = match unsafe {
            Upscaler::new(
                gpu.instance(),
                gpu.physical_device(),
                gpu.device(),
                render,
                output,
            )
        } {
            Ok(fsr) => fsr,
            Err(error) => {
                log::warn!("FSR upscaling is off and the game is drawn at full size: {error}");
                return Ok(None);
            }
        };
        let mut upscale = Self {
            gpu: Arc::clone(gpu),
            fsr,
            picture: Picture::null(),
            output,
            render,
        };
        // Dropping `upscale` cleans up whatever was created.
        upscale
            .picture
            .create(gpu, output, SceneTarget::FORMAT, Self::USAGE, COLOUR)?;
        Ok(Some(upscale))
    }

    /// How far to shift the camera for frame number `frame`, in pixels of
    /// the drawn picture: x to the right, y down.
    pub(crate) fn jitter(&self, frame: u32) -> (f32, f32) {
        fossps_fsr::jitter(frame, self.render.width, self.output.width)
    }

    /// Records upscaling `target` into [`picture`](Self::picture), which is
    /// left ready for shaders to read.
    ///
    /// # Safety
    ///
    /// `commands` must be a command buffer of this upscaler's device that is
    /// being recorded, outside a render pass. `target` must be of the same
    /// device and [`render`](Self::render) in size, hold a frame drawn by
    /// earlier commands in `commands` with its images ready for shaders to
    /// read, and stay alive until the commands have run.
    pub(crate) unsafe fn record(
        &mut self,
        commands: vk::CommandBuffer,
        target: &SceneTarget,
        drawn: &Drawn,
    ) -> Result<(), FsrError> {
        let device = self.gpu.device();
        let barrier = || {
            vk::ImageMemoryBarrier2::default()
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(self.picture.image)
                .subresource_range(COLOUR)
        };
        let to_write = [barrier()
            // The last frame's tone mapping may still be reading the picture.
            .src_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_STORAGE_WRITE)
            // Every pixel is written, so the old ones need not be kept.
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::GENERAL)];
        let to_read = [barrier()
            .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
            .src_access_mask(vk::AccessFlags2::SHADER_STORAGE_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
            .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
            .old_layout(vk::ImageLayout::GENERAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];

        let sampled = vk::ImageUsageFlags::SAMPLED;
        let input = |picture: &Picture, format: vk::Format| Image {
            image: picture.image,
            format,
            usage: sampled,
            extent: target.extent,
        };
        // Sizes are far below the point where f32 stops being exact.
        #[allow(clippy::cast_precision_loss)]
        let motion_scale = (target.extent.width as f32, target.extent.height as f32);
        let frame = Frame {
            color: input(&target.colour, SceneTarget::FORMAT),
            depth: input(&target.depth, SceneTarget::DEPTH_FORMAT),
            motion: input(&target.motion, SceneTarget::MOTION_FORMAT),
            output: Image {
                image: self.picture.image,
                format: SceneTarget::FORMAT,
                usage: Self::USAGE,
                extent: self.output,
            },
            jitter: drawn.jitter,
            // Motion is stored as a fraction of the picture.
            motion_scale,
            sharpness: SHARPNESS,
            frame_seconds: drawn.seconds,
            camera_near: drawn.near,
            camera_fov_y: drawn.fov,
            reset: drawn.cut,
        };

        // SAFETY: the caller guarantees the command buffer is recording
        // outside a render pass and the target's images are live, as created
        // by `SceneTarget::new` and ready to be read; the output picture is
        // this upscaler's own and is put in the layout FSR writes it in.
        unsafe {
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_write),
            );
            let upscaled = self.fsr.upscale(commands, &frame);
            // Even after a failure, the picture is left as later commands expect it.
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_read),
            );
            upscaled
        }
    }
}

impl Drop for Upscale {
    fn drop(&mut self) {
        // SAFETY: the picture was made on `self.gpu`, and the owner waits for
        // the graphics card to go idle before dropping the upscaler.
        unsafe { self.picture.destroy(&self.gpu) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_is_quality_and_off_draws_at_full_size() {
        assert_eq!(Upscaling::default(), Upscaling::Quality);
        assert_eq!(Upscaling::Off.quality(), None);
        let window = vk::Extent2D {
            width: 1920,
            height: 1080,
        };
        let size = |upscaling: Upscaling| {
            let render = upscaling.quality().unwrap().render_size(window);
            (render.width, render.height)
        };
        assert_eq!(size(Upscaling::Native), (1920, 1080));
        assert_eq!(size(Upscaling::Quality), (1280, 720));
        assert_eq!(size(Upscaling::Performance), (960, 540));
        assert_eq!(size(Upscaling::UltraPerformance), (640, 360));
    }
}
