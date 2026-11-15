//! Drawing with no window, to measure how long a frame takes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ash::vk;
use fossps_assets::Scene;

use crate::camera::Camera;
use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::mesh::{MeshPipeline, Meshes};
use crate::swapchain::COLOUR;
use crate::tonemap::{Then, TonemapPipeline};
use crate::upscale::Upscaling;
use crate::view::SceneView;

/// The format of the picture nobody sees: what most windows use, without
/// the sRGB curve built in.
const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// The frame time the upscaler is told about. Fixed, so that every run of a
/// benchmark gives the graphics card the same work.
const FRAME_SECONDS: f32 = 1.0 / 60.0;

/// How long one frame drawn by [`Headless::render`] took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTime {
    /// The engine's own work: telling the graphics card what to draw.
    pub recording: Duration,
    /// From handing the frame over until the graphics card had finished it.
    pub drawing: Duration,
}

impl FrameTime {
    /// The whole frame: recording and drawing.
    #[must_use]
    pub fn total(&self) -> Duration {
        self.recording + self.drawing
    }
}

/// Draws a scene exactly as [`Renderer`](crate::Renderer) does, but into a
/// picture that is never shown, and waits for each frame to finish.
///
/// It needs no window or display, so it runs on a server or in automated
/// checks, on any computer with Vulkan 1.3. It is what `fossps bench` times.
///
/// ```no_run
/// use fossps_render::{Camera, Headless, Upscaling};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut headless = Headless::new(1280, 720, Upscaling::Quality)?;
/// headless.set_scene(&fossps_assets::Scene::load("level.glb")?)?;
/// let time = headless.render(&Camera::default())?;
/// println!("That frame took {:?}.", time.total());
/// # Ok(())
/// # }
/// ```
pub struct Headless {
    // Dropped top to bottom after `drop` has destroyed the raw handles: the
    // tone mapping before the picture it reads, the graphics card last.
    tonemap: TonemapPipeline,
    mesh: MeshPipeline,
    meshes: Option<Meshes>,
    view: SceneView,
    extent: vk::Extent2D,
    image: vk::Image,
    memory: vk::DeviceMemory,
    image_view: vk::ImageView,
    pool: vk::CommandPool,
    commands: vk::CommandBuffer,
    finished: vk::Fence,
    gpu: Arc<Gpu>,
}

/// The raw Vulkan objects of a [`Headless`], destroyed together.
struct Raw {
    image: vk::Image,
    memory: vk::DeviceMemory,
    image_view: vk::ImageView,
    pool: vk::CommandPool,
    commands: vk::CommandBuffer,
    finished: vk::Fence,
}

impl Raw {
    /// # Safety
    ///
    /// The objects must belong to `gpu`'s device and no longer be in use.
    unsafe fn destroy(&self, gpu: &Gpu) {
        let device = gpu.device();
        // SAFETY: the caller's guarantee; destroying a null handle is allowed.
        unsafe {
            device.destroy_fence(self.finished, None);
            device.destroy_command_pool(self.pool, None);
            device.destroy_image_view(self.image_view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

impl Headless {
    /// Gets ready to draw pictures `width` by `height` pixels.
    ///
    /// If `upscaling` cannot be had on this graphics card, the reason is
    /// logged and the scene is drawn at full size.
    ///
    /// # Errors
    ///
    /// Fails if Vulkan 1.3 is not available, no graphics card can run the
    /// renderer, or the graphics driver reports a failure. The error's
    /// message says what to do about it.
    pub fn new(width: u32, height: u32, upscaling: Upscaling) -> Result<Self, RenderError> {
        // SAFETY: there is no window, so nothing has to outlive the `Gpu`.
        let gpu = Arc::new(unsafe { Gpu::new(None) }?);
        let extent = vk::Extent2D {
            width: width.max(1),
            height: height.max(1),
        };
        let view = SceneView::new(&gpu, extent, upscaling)?;
        let mesh = MeshPipeline::new(&gpu, 1)?;
        let tonemap = TonemapPipeline::new(&gpu, FORMAT)?;
        // SAFETY: the picture is `view`'s, which lives as long as `tonemap`
        // does, and nothing is being drawn yet.
        unsafe { tonemap.read_from(view.picture()) };

        let mut raw = Raw {
            image: vk::Image::null(),
            memory: vk::DeviceMemory::null(),
            image_view: vk::ImageView::null(),
            pool: vk::CommandPool::null(),
            commands: vk::CommandBuffer::null(),
            finished: vk::Fence::null(),
        };
        // SAFETY: the device is alive and every object is made from valid settings.
        if let Err(error) = unsafe { raw.create(&gpu, extent) } {
            // SAFETY: whatever was made has not been used.
            unsafe { raw.destroy(&gpu) };
            return Err(error);
        }
        let Raw {
            image,
            memory,
            image_view,
            pool,
            commands,
            finished,
        } = raw;
        Ok(Self {
            tonemap,
            mesh,
            meshes: None,
            view,
            extent,
            image,
            memory,
            image_view,
            pool,
            commands,
            finished,
            gpu,
        })
    }

    /// The graphics card being used.
    #[must_use]
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// The size the scene is drawn at, in pixels: smaller than the finished
    /// picture when it is upscaled.
    #[must_use]
    pub fn drawn_size(&self) -> (u32, u32) {
        let size = self.view.render_size();
        (size.width, size.height)
    }

    /// Draws the meshes of `scene` from now on, as
    /// [`Renderer::set_scene`](crate::Renderer::set_scene) does.
    ///
    /// # Errors
    ///
    /// Fails if the graphics card runs out of memory or its driver reports a
    /// failure.
    pub fn set_scene(&mut self, scene: &Scene) -> Result<(), RenderError> {
        // Every frame is waited for, so the old meshes are not in use.
        self.meshes = None;
        self.meshes = Some(Meshes::new(&self.gpu, scene)?);
        Ok(())
    }

    /// Draws one frame as `camera` sees it, waits until the graphics card
    /// has finished it, and says how long each half took.
    ///
    /// # Errors
    ///
    /// Fails if the graphics driver or the upscaler reports a failure; this
    /// should not be used afterwards.
    pub fn render(&mut self, camera: &Camera) -> Result<FrameTime, RenderError> {
        let device = self.gpu.device();
        let start = Instant::now();
        // SAFETY: every handle belongs to `self.gpu`'s device and is alive;
        // the last frame was waited for, so nothing here is still in use,
        // and this frame is waited for before anything can be destroyed.
        let handed_over = unsafe {
            device
                .reset_command_pool(self.pool, vk::CommandPoolResetFlags::empty())
                .map_err(RenderError::driver("starting a frame"))?;
            device
                .begin_command_buffer(
                    self.commands,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(RenderError::driver("recording a frame"))?;
            self.view.record(
                self.commands,
                0,
                &self.mesh,
                self.meshes.as_ref(),
                camera,
                FRAME_SECONDS,
            )?;
            self.tonemap.record(
                self.commands,
                self.image,
                self.image_view,
                self.extent,
                1.0,
                Then::Copy,
            );
            device
                .end_command_buffer(self.commands)
                .map_err(RenderError::driver("recording a frame"))?;
            device
                .reset_fences(&[self.finished])
                .map_err(RenderError::driver("drawing a frame"))?;
            let buffers = [vk::CommandBufferSubmitInfo::default().command_buffer(self.commands)];
            device
                .queue_submit2(
                    self.gpu.queue(),
                    &[vk::SubmitInfo2::default().command_buffer_infos(&buffers)],
                    self.finished,
                )
                .map_err(RenderError::driver("drawing a frame"))?;
            let handed_over = Instant::now();
            device
                .wait_for_fences(&[self.finished], true, u64::MAX)
                .map_err(RenderError::driver("waiting for the graphics card"))?;
            handed_over
        };
        Ok(FrameTime {
            recording: handed_over - start,
            drawing: handed_over.elapsed(),
        })
    }
}

impl Raw {
    /// Makes the picture, its memory and what a frame is recorded with.
    ///
    /// # Safety
    ///
    /// `self` must hold only null handles, and be destroyed on `gpu`'s device.
    unsafe fn create(&mut self, gpu: &Gpu, extent: vk::Extent2D) -> Result<(), RenderError> {
        let doing = "preparing to draw without a window";
        let device = gpu.device();
        // SAFETY: the device is alive, the settings are valid, and each
        // object is stored in `self` straight away so the caller can destroy it.
        unsafe {
            self.image = device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(FORMAT)
                        .extent(extent.into())
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .usage(
                            vk::ImageUsageFlags::COLOR_ATTACHMENT
                                | vk::ImageUsageFlags::TRANSFER_SRC,
                        ),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            let needs = device.get_image_memory_requirements(self.image);
            let kind = gpu
                .memory_type(needs.memory_type_bits, vk::MemoryPropertyFlags::empty())
                .ok_or(RenderError::OutOfMemory { doing })?;
            self.memory = device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(kind),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            device
                .bind_image_memory(self.image, self.memory, 0)
                .map_err(RenderError::driver(doing))?;
            self.image_view = device
                .create_image_view(
                    &vk::ImageViewCreateInfo::default()
                        .image(self.image)
                        .view_type(vk::ImageViewType::TYPE_2D)
                        .format(FORMAT)
                        .subresource_range(COLOUR),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            self.pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default().queue_family_index(gpu.queue_family()),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            self.commands = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(self.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(RenderError::driver(doing))?
                .first()
                .copied()
                .unwrap_or_default();
            self.finished = device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .map_err(RenderError::driver(doing))?;
        }
        Ok(())
    }
}

impl Drop for Headless {
    fn drop(&mut self) {
        let raw = Raw {
            image: self.image,
            memory: self.memory,
            image_view: self.image_view,
            pool: self.pool,
            commands: self.commands,
            finished: self.finished,
        };
        // SAFETY: the device is alive; once it is idle nothing is in use,
        // even if a frame failed half way.
        unsafe {
            // If waiting fails the driver is gone, and so is its work.
            let _ = self.gpu.device().device_wait_idle();
            raw.destroy(&self.gpu);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu::validation_errors;

    #[test]
    fn frames_are_drawn_and_timed_with_no_window() {
        // FSR runs on one graphics device at a time in a program, and the
        // offscreen tests beside this one use it; `fossps bench`'s own test
        // draws through here with upscaling on.
        let mut headless = match Headless::new(96, 64, Upscaling::Off) {
            Ok(headless) => headless,
            Err(
                error @ (RenderError::NoVulkan(_)
                | RenderError::OldVulkan { .. }
                | RenderError::NoSuitableGpu { .. }),
            ) => {
                let required =
                    std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
                assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {error}");
                eprintln!("Skipping the drawing test: {error}");
                return;
            }
            Err(error) => panic!("{error}"),
        };
        let (width, height) = headless.drawn_size();
        assert_eq!((width, height), (96, 64));
        // With nothing to draw, and then with a scene.
        headless.render(&Camera::default()).unwrap();
        headless.set_scene(&Scene::default()).unwrap();
        for step in 0..4_u8 {
            let camera = Camera {
                yaw: f32::from(step) * 0.1,
                ..Camera::default()
            };
            let time = headless.render(&camera).unwrap();
            assert_eq!(time.total(), time.recording + time.drawing);
        }
        drop(headless);
        assert_eq!(validation_errors(), 0);
    }
}
