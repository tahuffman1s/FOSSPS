//! Drawing with no window, to measure how long a frame takes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ash::vk;
use fossps_assets::Scene;
use fossps_ui::DrawList;

use crate::buffer::Buffer;
use crate::camera::Camera;
use crate::error::RenderError;
use crate::font::Font;
use crate::gpu::Gpu;
use crate::mesh::{Contents, MeshPipeline, Meshes};
use crate::overlay::Overlay;
use crate::swapchain::COLOUR;
use crate::tonemap::{Canvas, Then, TonemapPipeline};
use crate::ui_pass::UiPass;
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
    ui: UiPass,
    /// The HUD and menus drawn over each frame.
    overlay: Overlay,
    tonemap: TonemapPipeline,
    mesh: MeshPipeline,
    meshes: Option<Meshes>,
    /// Whether the editor's ground grid is drawn.
    grid: bool,
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
        let ui = UiPass::new(&gpu, FORMAT, 1)?;
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
            ui,
            overlay: Overlay::new(Font::bundled()),
            tonemap,
            mesh,
            meshes: None,
            grid: false,
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

    /// Draws the editor's ground grid or not from now on, as
    /// [`Renderer::set_grid`](crate::Renderer::set_grid) does.
    pub fn set_grid(&mut self, shown: bool) {
        self.grid = shown;
    }

    /// Draws `list` over every frame from now on, as
    /// [`Renderer::set_ui`](crate::Renderer::set_ui) does.
    pub fn set_ui(&mut self, list: DrawList<'_>) {
        self.overlay.set(list);
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
                Contents {
                    meshes: self.meshes.as_ref(),
                    grid: self.grid,
                },
                camera,
                FRAME_SECONDS,
            )?;
            self.ui.prepare(self.commands, 0, &mut self.overlay)?;
            self.tonemap.record(
                self.commands,
                Canvas {
                    image: self.image,
                    view: self.image_view,
                    extent: self.extent,
                },
                1.0,
                Some(&self.ui),
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

    /// Draws one frame as `camera` sees it, as [`render`](Self::render)
    /// does, and gives the finished picture: four bytes a pixel (red,
    /// green, blue, opacity), row by row from the top-left corner, as a
    /// screen would show them.
    ///
    /// It waits for the picture to be copied off the graphics card, so it
    /// is for tests and tools, not for every frame.
    ///
    /// # Errors
    ///
    /// Fails if the graphics driver or the upscaler reports a failure, or
    /// there is no memory for the picture.
    pub fn screenshot(&mut self, camera: &Camera) -> Result<Vec<u8>, RenderError> {
        let doing = "reading a picture back";
        self.render(camera)?;
        let length = self.extent.width as usize * self.extent.height as usize * 4;
        let pixels = Buffer::writable(
            &self.gpu,
            length as u64,
            vk::BufferUsageFlags::TRANSFER_DST,
            doing,
        )?;
        let device = self.gpu.device();
        let whole = vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: 0,
            base_array_layer: 0,
            layer_count: 1,
        };
        // SAFETY: every handle belongs to `self.gpu`'s device and is alive.
        // The frame above was waited for, so the command buffer is free to
        // record again, and it left the picture laid out to be copied from.
        // The buffer holds exactly the picture's bytes, and the copy is
        // waited for before the buffer is read or dropped.
        unsafe {
            device
                .reset_command_pool(self.pool, vk::CommandPoolResetFlags::empty())
                .map_err(RenderError::driver(doing))?;
            device
                .begin_command_buffer(
                    self.commands,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )
                .map_err(RenderError::driver(doing))?;
            device.cmd_copy_image_to_buffer(
                self.commands,
                self.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                pixels.handle,
                &[vk::BufferImageCopy::default()
                    .image_subresource(whole)
                    .image_extent(self.extent.into())],
            );
            device
                .end_command_buffer(self.commands)
                .map_err(RenderError::driver(doing))?;
            device
                .reset_fences(&[self.finished])
                .map_err(RenderError::driver(doing))?;
            let buffers = [vk::CommandBufferSubmitInfo::default().command_buffer(self.commands)];
            device
                .queue_submit2(
                    self.gpu.queue(),
                    &[vk::SubmitInfo2::default().command_buffer_infos(&buffers)],
                    self.finished,
                )
                .map_err(RenderError::driver(doing))?;
            if let Err(code) = device.wait_for_fences(&[self.finished], true, u64::MAX) {
                // The copy may still be running, and the buffer it writes
                // is about to be dropped. If this fails too the driver is
                // gone, and so is its work.
                let _ = device.device_wait_idle();
                return Err(RenderError::driver(doing)(code));
            }
            pixels.read(length)
        }
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

    /// Something to draw `width` by `height` pictures with, or `None` on a
    /// computer that cannot draw, unless `FOSSPS_REQUIRE_VULKAN` says it must.
    fn headless(width: u32, height: u32) -> Option<Headless> {
        // FSR runs on one graphics device at a time in a program, and the
        // offscreen tests beside these use it; `fossps bench`'s own test
        // draws through here with upscaling on.
        match Headless::new(width, height, Upscaling::Off) {
            Ok(headless) => Some(headless),
            Err(
                error @ (RenderError::NoVulkan(_)
                | RenderError::OldVulkan { .. }
                | RenderError::NoSuitableGpu { .. }),
            ) => {
                let required =
                    std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
                assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {error}");
                eprintln!("Skipping the drawing test: {error}");
                None
            }
            Err(error) => panic!("{error}"),
        }
    }

    #[test]
    fn the_ground_grid_is_drawn_with_its_axes_in_colour() {
        const SIZE: [usize; 2] = [256, 144];
        let Some(mut headless) = headless(256, 144) else {
            return;
        };
        headless.gpu().require_validation();
        // Five metres back from the middle of the world and three up,
        // looking straight at it: the X axis runs across the middle of the
        // picture, and the Z axis from the middle down to the bottom.
        let camera = Camera {
            position: [0.0, 3.0, 5.0],
            pitch: -(3.0_f32 / 5.0).atan(),
            ..Camera::default()
        };
        let plain = headless.screenshot(&camera).unwrap();
        headless.set_grid(true);
        let with_grid = headless.screenshot(&camera).unwrap();
        // Set `FOSSPS_SAVE_PICTURES` to a folder to look at the picture:
        // 256 by 144 pixels of four bytes each.
        if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
            std::fs::write(std::path::Path::new(&folder).join("grid.rgba"), &with_grid).unwrap();
        }
        let pixel = |picture: &[u8], x: usize, y: usize| {
            let at = (y * SIZE[0] + x) * 4;
            [picture[at], picture[at + 1], picture[at + 2]].map(i32::from)
        };
        // The brightest of the pixels around the place, since a line is a
        // pixel or two wide.
        let near = |x: usize, y: usize| {
            let mut best = [0; 3];
            for (x, y) in (y - 2..=y + 2).flat_map(|y| (x - 2..=x + 2).map(move |x| (x, y))) {
                let found = pixel(&with_grid, x, y);
                if found.iter().sum::<i32>() > best.iter().sum::<i32>() {
                    best = found;
                }
            }
            best
        };
        let [red, green, blue] = near(SIZE[0] / 4 + 5, SIZE[1] / 2);
        assert!(
            red > 2 * green && red > 2 * blue,
            "X axis: {red} {green} {blue}"
        );
        let [red, green, blue] = near(SIZE[0] / 2, SIZE[1] * 3 / 4 + 3);
        assert!(
            blue > 2 * red && blue > green,
            "Z axis: {red} {green} {blue}"
        );
        // Lines a metre apart, lighter than the background between them.
        let background = pixel(&plain, 40, 100);
        let lit = (0..SIZE[0])
            .filter(|&x| pixel(&with_grid, x, 100)[1] > background[1] + 12)
            .count();
        assert!((4..SIZE[0] / 2).contains(&lit), "{lit} lit pixels in a row");
        // Nothing above the horizon, and nothing once it is switched off.
        assert_eq!(pixel(&with_grid, 128, 4), pixel(&plain, 128, 4));
        headless.set_grid(false);
        assert_eq!(headless.screenshot(&camera).unwrap(), plain);
        drop(headless);
        assert_eq!(validation_errors(), 0);
    }

    #[test]
    fn frames_are_drawn_and_timed_with_no_window() {
        let Some(mut headless) = headless(96, 64) else {
            return;
        };
        headless.gpu().require_validation();
        let (width, height) = headless.drawn_size();
        assert_eq!((width, height), (96, 64));
        // With nothing to draw, and then with a scene.
        headless.render(&Camera::default()).unwrap();
        headless.set_scene(&Scene::default()).unwrap();
        // From the second of these frames on, a HUD is drawn over them, and
        // on the last a menu with letters the graphics card has not seen.
        let mut ui = fossps_ui::Ui::default();
        ui.hud_text(fossps_ui::Anchor::BottomRight, [0.0, 0.0], 32.0, "30 / 90");
        let menu = ui.add_screen("Paused", |screen| {
            screen.button("Resume");
        });
        for step in 0..4_u8 {
            if step == 3 {
                ui.open(menu.clone().unwrap()).unwrap();
            }
            if step > 0 {
                ui.update(&fossps_ui::UiInput::default(), 1.0 / 60.0, [96, 64]);
                headless.set_ui(ui.draw());
            }
            let camera = Camera {
                yaw: f32::from(step) * 0.1,
                ..Camera::default()
            };
            let time = headless.render(&camera).unwrap();
            assert_eq!(time.total(), time.recording + time.drawing);
        }
        // The finished picture can be read back, and the menu is in it.
        let with_menu = headless.screenshot(&Camera::default()).unwrap();
        assert_eq!(with_menu.len(), 96 * 64 * 4);
        headless.set_ui(fossps_ui::Ui::default().draw());
        let without = headless.screenshot(&Camera::default()).unwrap();
        assert_ne!(with_menu, without);
        assert_eq!(without, headless.screenshot(&Camera::default()).unwrap());
        // A frame can be drawn after one was read back.
        headless.render(&Camera::default()).unwrap();
        drop(headless);
        assert_eq!(validation_errors(), 0);
    }
}
