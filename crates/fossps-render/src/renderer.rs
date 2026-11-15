//! Draws frames into a window.

use std::any::Any;
use std::sync::Arc;
use std::time::Instant;

use ash::vk;
use fossps_assets::Scene;
use fossps_ui::DrawList;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::camera::Camera;
use crate::error::RenderError;
use crate::font::Font;
use crate::gpu::Gpu;
use crate::mesh::{Contents, MeshPipeline, Meshes};
use crate::overlay::Overlay;
use crate::swapchain::Swapchain;
use crate::tonemap::{Canvas, Then, TonemapPipeline, exposure_scale};
use crate::ui_pass::UiPass;
use crate::upscale::Upscaling;
use crate::view::SceneView;

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
    /// How much smaller than the window the scene is drawn before FSR makes
    /// it full size. Smaller is faster; see [`Upscaling`].
    pub upscaling: Upscaling,
}

impl Default for RenderConfig {
    /// Vsync on, and [`Upscaling::Quality`].
    fn default() -> Self {
        Self {
            vsync: true,
            upscaling: Upscaling::default(),
        }
    }
}

/// The frame time given to the upscaler when none has been measured yet, and
/// the longest it is told about: a pause is not a slow frame.
const USUAL_FRAME_SECONDS: f32 = 1.0 / 60.0;
const LONGEST_FRAME_SECONDS: f32 = 0.25;

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
    // card first, then the card, then the window it draws into. The tone
    // mapping goes before the picture it reads.
    frames: Vec<FrameSlot>,
    /// Which of `frames` the next frame uses.
    next: usize,
    /// Draws the HUD and menus; made for the swapchain's colour format.
    ui: Option<UiPass>,
    /// The HUD and menus to draw over each frame.
    overlay: Overlay,
    tonemap: Option<TonemapPipeline>,
    mesh: Option<MeshPipeline>,
    /// The level's meshes on the graphics card; `None` until a scene is set.
    meshes: Option<Meshes>,
    /// Whether the editor's ground grid is drawn.
    grid: bool,
    /// Where the scene is seen from.
    camera: Camera,
    /// What the scene is drawn into and upscaled by; its finished picture
    /// is the size of the swapchain's images.
    view: Option<SceneView>,
    /// The picture size and upscaling `view` was made for.
    view_for: (vk::Extent2D, Upscaling),
    /// When the last frame was drawn.
    last_frame: Option<Instant>,
    /// What the scene's light is multiplied by before tone mapping.
    exposure: f32,
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
            ui: None,
            overlay: Overlay::new(Font::bundled()),
            tonemap: None,
            mesh: None,
            meshes: None,
            grid: false,
            camera: Camera::default(),
            view: None,
            view_for: (vk::Extent2D::default(), config.upscaling),
            last_frame: None,
            exposure: 1.0,
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

    /// Draws the meshes of `scene` from now on, in place of any scene set before.
    ///
    /// Each mesh is drawn in its material's base colour, with nearer
    /// surfaces in front of farther ones and the backs of surfaces left out
    /// unless the material is double-sided. Textures and lights are not
    /// drawn yet. The meshes are copied to the graphics card here, so call
    /// this when a level loads, not every frame.
    ///
    /// ```no_run
    /// # fn show(renderer: &mut fossps_render::Renderer) -> Result<(), Box<dyn std::error::Error>> {
    /// let level = fossps_assets::Scene::load("level.glb")?;
    /// renderer.set_scene(&level)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Fails if the graphics card runs out of memory or its driver reports a
    /// failure. The scene set before is no longer drawn.
    pub fn set_scene(&mut self, scene: &Scene) -> Result<(), RenderError> {
        // Frames still being drawn use the old meshes.
        // SAFETY: the device is alive for as long as `self.gpu` is.
        unsafe { self.gpu.device().device_wait_idle() }
            .map_err(RenderError::driver("waiting for the graphics card"))?;
        // Dropped first, so the old and new meshes are never both held.
        self.meshes = None;
        self.meshes = Some(Meshes::new(&self.gpu, scene)?);
        // Nothing of the old level belongs in the new one's first frame.
        if let Some(view) = &mut self.view {
            view.cut();
        }
        Ok(())
    }

    /// Sets where the scene is seen from, from the next frame on.
    ///
    /// Use this for a view that moves smoothly: walking, turning, falling.
    /// When the view jumps, use [`cut_to`](Self::cut_to).
    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    /// Jumps the view to `camera`: for a respawn, a teleport or switching
    /// to another player's eyes.
    ///
    /// Upscaling builds each frame from the ones before it. After a jump
    /// those show somewhere else, and would smear into the new view for a
    /// moment; this throws them away instead.
    ///
    /// ```no_run
    /// # fn respawn(renderer: &mut fossps_render::Renderer) {
    /// renderer.cut_to(fossps_render::Camera {
    ///     position: [12.0, 1.7, -30.0],
    ///     ..fossps_render::Camera::default()
    /// });
    /// # }
    /// ```
    pub fn cut_to(&mut self, camera: Camera) {
        self.camera = camera;
        if let Some(view) = &mut self.view {
            view.cut();
        }
    }

    /// Draws the editor's ground grid over the level, or stops drawing it,
    /// from the next frame on.
    ///
    /// The grid lies at height 0 and reaches as far as can be seen: a line
    /// every metre, a stronger one every ten, the X axis in red and the Z
    /// axis in blue. A game being played has no use for it.
    pub fn set_grid(&mut self, shown: bool) {
        self.grid = shown;
    }

    /// Changes how much smaller than the window the scene is drawn, from the
    /// next frame on. For a settings menu; see [`Upscaling`].
    ///
    /// If the graphics card cannot run FSR, the reason is logged and the
    /// scene is drawn at full size.
    pub fn set_upscaling(&mut self, upscaling: Upscaling) {
        if self.config.upscaling != upscaling {
            self.config.upscaling = upscaling;
            self.stale = true;
        }
    }

    /// Makes the whole picture brighter or darker, in camera stops.
    ///
    /// `0.0` shows the scene's light as it is. Each `+1.0` doubles it and each
    /// `-1.0` halves it. Bright areas roll off towards white instead of
    /// turning into flat patches. Values beyond 16 stops either way are
    /// held at 16, and a value that is not a number is ignored.
    pub fn set_exposure(&mut self, stops: f32) {
        // A not-a-number here would turn the whole picture black.
        if stops.is_finite() {
            // Sixteen stops either way is 65 536 times: more than any scene
            // needs, and far from where the multiplier stops being a number.
            self.exposure = exposure_scale(stops.clamp(-16.0, 16.0));
        }
    }

    /// Draws `list` over the game from the next frame on, in place of any
    /// list given before: the boxes and text of a HUD or a menu, as
    /// `fossps_ui::Ui::draw` gives them.
    ///
    /// The list is drawn after upscaling, at the full size of the window,
    /// so text stays sharp whatever [`Upscaling`] is, and exposure does not
    /// change its colours. Positions are in pixels from the window's
    /// top-left corner, which is what a `Ui` gives when it is updated with
    /// the window's size. Call this every frame with what the UI wants
    /// drawn; an empty list draws nothing.
    ///
    /// ```no_run
    /// # fn show(renderer: &mut fossps_render::Renderer, size: [u32; 2]) {
    /// use fossps_ui::{Anchor, Ui, UiInput};
    ///
    /// let mut ui = Ui::default();
    /// ui.set_text_measure(Box::new(renderer.font()));
    /// ui.hud_text(Anchor::BottomRight, [0.0, 0.0], 32.0, "30 / 90");
    ///
    /// // Every frame:
    /// ui.update(&UiInput::default(), 1.0 / 60.0, size);
    /// renderer.set_ui(ui.draw());
    /// # }
    /// ```
    ///
    /// At most 8192 boxes and letters are drawn in a frame; the rest are
    /// left out and a warning is logged once.
    pub fn set_ui(&mut self, list: DrawList<'_>) {
        self.overlay.set(list);
    }

    /// The font text is drawn in. Give it to a `Ui` with
    /// `set_text_measure` so that it centres text by its real width.
    #[must_use]
    pub fn font(&self) -> Font {
        self.overlay.font().clone()
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
        let (format, extent) = (swapchain.format, swapchain.extent);
        // The graphics card is idle, so the old swapchain can go. It is
        // replaced before anything else can fail, because a swapchain that
        // has been replaced once must not be offered for replacement again.
        self.swapchain = Some(swapchain);
        if self.mesh.is_none() {
            self.mesh = Some(MeshPipeline::new(&self.gpu, FRAMES_IN_FLIGHT)?);
        }
        if self
            .tonemap
            .as_ref()
            .is_none_or(|tonemap| tonemap.format != format)
        {
            self.tonemap = Some(TonemapPipeline::new(&self.gpu, format)?);
            // The new pipeline has not been told about the picture yet.
            self.view = None;
        }
        if self.ui.as_ref().is_none_or(|ui| ui.format() != format) {
            // Dropped first, so the old and new atlases are never both held.
            self.ui = None;
            self.ui = Some(UiPass::new(&self.gpu, format, FRAMES_IN_FLIGHT)?);
        }
        let wanted = (extent, self.config.upscaling);
        if self.view.is_none() || self.view_for != wanted {
            // Dropped first, so the old and new images are never both held.
            self.view = None;
            let view = SceneView::new(&self.gpu, extent, self.config.upscaling)?;
            let drawn = view.render_size();
            if drawn != extent {
                log::info!(
                    "Drawing at {}x{} and upscaling to {}x{} with FSR 3.1.",
                    drawn.width,
                    drawn.height,
                    extent.width,
                    extent.height
                );
            }
            if let Some(tonemap) = &self.tonemap {
                // SAFETY: the picture is the new view's, which is stored next
                // to the pipeline and replaced only here; the card is idle.
                unsafe { tonemap.read_from(view.picture()) };
            }
            self.view = Some(view);
            self.view_for = wanted;
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
        let (Some(swapchain), Some(mesh), Some(tonemap), Some(view), Some(ui)) = (
            &self.swapchain,
            &self.mesh,
            &self.tonemap,
            &mut self.view,
            &mut self.ui,
        ) else {
            return Ok(Drawn::No);
        };
        let slot = &self.frames[self.next];
        let device = self.gpu.device();
        let now = Instant::now();
        let seconds = self.last_frame.map_or(USUAL_FRAME_SECONDS, |last| {
            (now - last).as_secs_f32().min(LONGEST_FRAME_SECONDS)
        });
        self.last_frame = Some(now);

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
            view.record(
                slot.commands,
                self.next,
                mesh,
                Contents {
                    meshes: self.meshes.as_ref(),
                    grid: self.grid,
                },
                &self.camera,
                seconds,
            )?;
            ui.prepare(slot.commands, self.next, &mut self.overlay)?;
            tonemap.record(
                slot.commands,
                Canvas {
                    image: target.image,
                    view: target.view,
                    extent: swapchain.extent,
                },
                self.exposure,
                Some(ui),
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
