//! Draws frames into a window.

use std::any::Any;
use std::sync::Arc;

use ash::vk;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::swapchain::Swapchain;
use crate::triangle::{Then, TrianglePipeline};

/// How many frames can be in progress on the graphics card at once.
///
/// Two lets the next frame be recorded while the last one is still being
/// drawn, without queueing up so many that input feels late.
const FRAMES_IN_FLIGHT: usize = 2;

/// The longest to wait for the window system to hand over an image, in nanoseconds.
///
/// Some window systems stop handing out images while a window is hidden. A
/// limit keeps the main loop, and with it the simulation, running.
const ACQUIRE_TIMEOUT: u64 = 100_000_000;

/// How the renderer should draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderConfig {
    /// Wait for the display before showing each frame.
    ///
    /// On: no tearing, and the frame rate matches the display. Off: frames are
    /// drawn as fast as possible for the lowest input delay, but this uses more
    /// power and can show a visible split line (tearing).
    pub vsync: bool,
}

impl Default for RenderConfig {
    /// Vsync on.
    fn default() -> Self {
        Self { vsync: true }
    }
}

/// What one in-progress frame needs for itself.
struct FrameSlot {
    gpu: Arc<Gpu>,
    pool: vk::CommandPool,
    commands: vk::CommandBuffer,
    /// Signalled when the window system has handed over an image.
    acquired: vk::Semaphore,
    /// Signalled when the graphics card has finished this frame.
    finished: vk::Fence,
}

impl FrameSlot {
    fn new(gpu: &Arc<Gpu>) -> Result<Self, RenderError> {
        let doing = "preparing to draw";
        let device = gpu.device();
        let mut slot = Self {
            gpu: Arc::clone(gpu),
            pool: vk::CommandPool::null(),
            commands: vk::CommandBuffer::null(),
            acquired: vk::Semaphore::null(),
            finished: vk::Fence::null(),
        };
        // Dropping `slot` cleans up whatever was created; destroying a null handle is allowed.

        // SAFETY: the device is alive, and each object is created from valid
        // settings and stored in `slot` straight away.
        unsafe {
            slot.pool = device
                .create_command_pool(
                    &vk::CommandPoolCreateInfo::default().queue_family_index(gpu.queue_family()),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            slot.commands = device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(slot.pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .map_err(RenderError::driver(doing))?
                .first()
                .copied()
                .unwrap_or_default();
            slot.acquired = device
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)
                .map_err(RenderError::driver(doing))?;
            // Starts signalled so the first frame does not wait for a frame that never ran.
            slot.finished = device
                .create_fence(
                    &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
        }
        Ok(slot)
    }
}

impl Drop for FrameSlot {
    fn drop(&mut self) {
        // SAFETY: the renderer waits for the graphics card to go idle before
        // dropping its frames. Destroying the pool frees its command buffer.
        unsafe {
            let device = self.gpu.device();
            device.destroy_fence(self.finished, None);
            device.destroy_semaphore(self.acquired, None);
            device.destroy_command_pool(self.pool, None);
        }
    }
}

/// Draws into one window.
///
/// Create it once the window exists, call [`resize`](Self::resize) when the
/// window changes size and [`render`](Self::render) once per frame.
pub struct Renderer {
    // Fields are dropped top to bottom: everything made from the graphics
    // card first, then the card, then the window it draws into.
    frames: Vec<FrameSlot>,
    /// Which of `frames` the next frame uses.
    next: usize,
    pipeline: Option<TrianglePipeline>,
    /// `None` while the window has no area to draw into.
    swapchain: Option<Swapchain>,
    /// The window size in pixels, as last reported.
    size: (u32, u32),
    /// Set when the swapchain no longer matches the window.
    stale: bool,
    config: RenderConfig,
    gpu: Arc<Gpu>,
    /// Keeps the window open for as long as the renderer draws into it.
    _window: Box<dyn Any>,
}

impl Renderer {
    /// Starts drawing into `window`, which is `width` by `height` pixels.
    ///
    /// The renderer keeps `window` until it is dropped. Pass a shared handle
    /// such as `Rc<Window>` or `Arc<Window>` to keep using the window elsewhere.
    ///
    /// # Errors
    ///
    /// Fails if Vulkan 1.3 is not available, no graphics card can draw into
    /// the window, or the graphics driver reports a failure. The error's
    /// message says what the player can do about it.
    pub fn new<W>(
        window: W,
        width: u32,
        height: u32,
        config: RenderConfig,
    ) -> Result<Self, RenderError>
    where
        W: HasWindowHandle + HasDisplayHandle + 'static,
    {
        let display = window
            .display_handle()
            .map_err(RenderError::Window)?
            .as_raw();
        let handle = window
            .window_handle()
            .map_err(RenderError::Window)?
            .as_raw();
        // SAFETY: `window` is stored in the renderer's last field, so it is
        // dropped after the `Gpu`; on an early return below, the locals are
        // dropped in reverse order, which also drops `gpu` before `window`.
        let gpu = Arc::new(unsafe { Gpu::new(Some((display, handle))) }?);

        let mut frames = Vec::with_capacity(FRAMES_IN_FLIGHT);
        for _ in 0..FRAMES_IN_FLIGHT {
            frames.push(FrameSlot::new(&gpu)?);
        }
        let mut renderer = Self {
            frames,
            next: 0,
            pipeline: None,
            swapchain: None,
            size: (width, height),
            stale: true,
            config,
            gpu,
            _window: Box::new(window),
        };
        // Building the swapchain and pipeline now reports any problem at
        // startup instead of on the first frame.
        renderer.rebuild()?;
        Ok(renderer)
    }

    /// The graphics card being used, with its raw Vulkan handles.
    #[must_use]
    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    /// Tells the renderer the window is now `width` by `height` pixels.
    ///
    /// Cheap: the images are rebuilt on the next [`render`](Self::render).
    pub fn resize(&mut self, width: u32, height: u32) {
        if self.size != (width, height) {
            self.size = (width, height);
            self.stale = true;
        }
    }

    /// Draws one frame and shows it in the window.
    ///
    /// Returns `false` if nothing was drawn because the window is minimised
    /// or hidden. With vsync on, this waits for the display.
    ///
    /// # Errors
    ///
    /// Fails if the graphics driver reports a failure; the renderer should
    /// not be used afterwards.
    pub fn render(&mut self) -> Result<bool, RenderError> {
        // A second try covers the window changing just as the frame begins.
        for _ in 0..2 {
            if self.stale {
                self.rebuild()?;
            }
            match self.draw()? {
                Drawn::Yes => return Ok(true),
                Drawn::No => return Ok(false),
                Drawn::Stale => self.stale = true,
            }
        }
        Ok(false)
    }

    /// Replaces the swapchain with one matching the window as it is now.
    fn rebuild(&mut self) -> Result<(), RenderError> {
        // SAFETY: the device is alive for as long as `self.gpu` is.
        unsafe { self.gpu.device().device_wait_idle() }
            .map_err(RenderError::driver("waiting for the graphics card"))?;

        let rebuilt = Swapchain::new(
            &self.gpu,
            self.size,
            self.config.vsync,
            self.swapchain.as_ref(),
        )?;
        // With no area to draw into, the old swapchain is kept so there is
        // something to replace later, and this is tried again next frame.
        let Some(swapchain) = rebuilt else {
            return Ok(());
        };
        let format = swapchain.format;
        // The graphics card is idle, so the old swapchain can go. It is
        // replaced before anything else can fail, because a swapchain that
        // has been replaced once must not be offered for replacement again.
        self.swapchain = Some(swapchain);
        if self
            .pipeline
            .as_ref()
            .is_none_or(|pipeline| pipeline.format != format)
        {
            self.pipeline = Some(TrianglePipeline::new(&self.gpu, format)?);
        }
        self.stale = false;
        Ok(())
    }

    /// Draws one frame into the current swapchain.
    fn draw(&mut self) -> Result<Drawn, RenderError> {
        if self.stale {
            // `rebuild` found no area to draw into.
            return Ok(Drawn::No);
        }
        let (Some(swapchain), Some(pipeline)) = (&self.swapchain, &self.pipeline) else {
            return Ok(Drawn::No);
        };
        let slot = &self.frames[self.next];
        let device = self.gpu.device();

        // SAFETY: every handle used here belongs to `self.gpu`'s device and
        // is alive; the fence wait makes sure this slot's command buffer and
        // semaphore are no longer in use before they are reused.
        unsafe {
            device
                .wait_for_fences(&[slot.finished], true, u64::MAX)
                .map_err(RenderError::driver("waiting for the graphics card"))?;

            let acquired = self.gpu.swapchains().acquire_next_image(
                swapchain.handle,
                ACQUIRE_TIMEOUT,
                slot.acquired,
                vk::Fence::null(),
            );
            let index = match acquired {
                // A swapchain that works but is not ideal is fine to draw
                // into; `resize` marks it stale when the window size changes.
                Ok((index, _suboptimal)) => index,
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => return Ok(Drawn::Stale),
                Err(vk::Result::TIMEOUT | vk::Result::NOT_READY) => return Ok(Drawn::No),
                Err(code) => return Err(RenderError::driver("starting a frame")(code)),
            };
            let Some(target) = swapchain.targets.get(index as usize) else {
                return Err(RenderError::Driver {
                    doing: "starting a frame",
                    code: vk::Result::ERROR_UNKNOWN,
                });
            };

            // Only now is it certain this frame will be submitted. Resetting
            // the fence earlier would leave the next wait on it stuck.
            device
                .reset_fences(&[slot.finished])
                .map_err(RenderError::driver("starting a frame"))?;
            device
                .reset_command_pool(slot.pool, vk::CommandPoolResetFlags::empty())
                .map_err(RenderError::driver("starting a frame"))?;
            device
                .begin_command_buffer(
                    slot.commands,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(RenderError::driver("recording a frame"))?;
            pipeline.record(
                slot.commands,
                target.image,
                target.view,
                swapchain.extent,
                Then::Show,
            );
            device
                .end_command_buffer(slot.commands)
                .map_err(RenderError::driver("recording a frame"))?;

            let waits = [vk::SemaphoreSubmitInfo::default()
                .semaphore(slot.acquired)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
            let buffers = [vk::CommandBufferSubmitInfo::default().command_buffer(slot.commands)];
            let signals = [vk::SemaphoreSubmitInfo::default().semaphore(target.drawn)];
            let submit = [vk::SubmitInfo2::default()
                .wait_semaphore_infos(&waits)
                .command_buffer_infos(&buffers)
                .signal_semaphore_infos(&signals)];
            device
                .queue_submit2(self.gpu.queue(), &submit, slot.finished)
                .map_err(RenderError::driver("drawing a frame"))?;

            let swapchains = [swapchain.handle];
            let indices = [index];
            let drawn = [target.drawn];
            let present = vk::PresentInfoKHR::default()
                .wait_semaphores(&drawn)
                .swapchains(&swapchains)
                .image_indices(&indices);
            self.next = (self.next + 1) % self.frames.len();
            match self
                .gpu
                .swapchains()
                .queue_present(self.gpu.queue(), &present)
            {
                Ok(_suboptimal) => {}
                // The frame was drawn but the window changed; rebuild before the next one.
                Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => self.stale = true,
                Err(code) => return Err(RenderError::driver("showing a frame")(code)),
            }
        }
        Ok(Drawn::Yes)
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // The fields are destroyed right after this, so the graphics card
        // must have finished with them. If waiting fails the device is lost
        // and nothing is running on it anyway.
        // SAFETY: the device is alive for as long as `self.gpu` is.
        let _ = unsafe { self.gpu.device().device_wait_idle() };
    }
}

/// How an attempt to draw a frame went.
enum Drawn {
    /// A frame was drawn and handed to the window system.
    Yes,
    /// Nothing can be drawn right now.
    No,
    /// The swapchain no longer matches the window and must be rebuilt first.
    Stale,
}
