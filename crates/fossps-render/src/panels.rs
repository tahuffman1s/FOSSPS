//! The editor's panels: lists, buttons and fields made with `egui`, drawn
//! over the finished picture after the HUD.
//!
//! Only built with the `panels` feature, which the editor turns on. A game
//! being played never makes one of these, so it pays nothing for them.

use std::sync::Arc;

use ash::vk;
use egui_ash_renderer::allocator::DefaultAllocator;
use egui_ash_renderer::{DynamicRendering, Options, RenderMode, RendererError};

use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::tonemap::stores_srgb;

/// One frame of editor panels, as `egui` hands them over.
///
/// Give it to [`Renderer::set_panels`](crate::Renderer::set_panels) once for
/// every frame `egui` was run, even one that is never drawn: `textures`
/// holds only what changed since the frame before, so a frame left out
/// would lose letters.
///
/// ```no_run
/// # fn show(renderer: &mut fossps_render::Renderer, input: egui::RawInput) {
/// let context = egui::Context::default();
/// let output = context.run_ui(input, |ui| {
///     ui.label("Level");
/// });
/// renderer.set_panels(fossps_render::Panels {
///     shapes: context.tessellate(output.shapes, output.pixels_per_point),
///     textures: output.textures_delta,
///     pixels_per_point: output.pixels_per_point,
/// });
/// # }
/// ```
#[derive(Default)]
pub struct Panels {
    /// What to draw, from `egui::Context::tessellate`.
    pub shapes: Vec<egui::ClippedPrimitive>,
    /// The pictures (the letters, mostly) that changed this frame, from
    /// `egui::FullOutput::textures_delta`.
    pub textures: egui::TexturesDelta,
    /// How many pixels one of `egui`'s points is wide, from
    /// `egui::FullOutput::pixels_per_point`.
    pub pixels_per_point: f32,
}

/// Draws panels into images of one format.
struct Pass {
    /// Declared before `gpu` so it is dropped while the device is alive.
    drawer: egui_ash_renderer::Renderer<DefaultAllocator>,
    /// Texture uploads are recorded from this pool.
    pool: vk::CommandPool,
    format: vk::Format,
    gpu: Arc<Gpu>,
}

impl Drop for Pass {
    fn drop(&mut self) {
        // SAFETY: the pool belongs to this device and nothing recorded from
        // it is running: every upload was waited for when it was made.
        unsafe { self.gpu.device().destroy_command_pool(self.pool, None) };
    }
}

/// The panels to draw and what draws them. Empty, and free, until panels
/// are first set.
#[derive(Default)]
pub(crate) struct PanelHost {
    pass: Option<Pass>,
    shapes: Vec<egui::ClippedPrimitive>,
    pixels_per_point: f32,
    /// Texture changes not yet sent to the graphics card, oldest first.
    waiting: Vec<egui::TexturesDelta>,
}

impl Drop for PanelHost {
    fn drop(&mut self) {
        // A window closed before its next frame leaves changes that will
        // never be sent; `egui` asks to be told that is meant.
        for delta in &mut self.waiting {
            delta.clear();
        }
    }
}

/// Says what the renderer was doing when the panels failed.
fn failed(doing: &'static str) -> impl FnOnce(RendererError) -> RenderError {
    move |error| match error {
        RendererError::Vulkan(code) => RenderError::driver(doing)(code),
        other => {
            log::error!("While {doing}: {other}");
            RenderError::Driver {
                doing,
                code: vk::Result::ERROR_UNKNOWN,
            }
        }
    }
}

impl PanelHost {
    /// Replaces what is drawn, and queues the texture changes.
    pub(crate) fn set(&mut self, panels: Panels) {
        self.shapes = panels.shapes;
        self.pixels_per_point = panels.pixels_per_point;
        if !panels.textures.is_empty() {
            self.waiting.push(panels.textures);
        }
    }

    /// Gets ready to draw into images of `format`: makes the pass the first
    /// time there is something to draw, and sends waiting texture changes.
    ///
    /// Call it before recording a frame. `in_flight` is how many frames the
    /// caller records before waiting for the first of them.
    pub(crate) fn prepare(
        &mut self,
        gpu: &Arc<Gpu>,
        format: vk::Format,
        in_flight: usize,
    ) -> Result<(), RenderError> {
        if self.pass.is_none() && self.shapes.is_empty() && self.waiting.is_empty() {
            return Ok(());
        }
        let doing = "preparing the editor's panels";
        let mode = || {
            RenderMode::DynamicRendering(DynamicRendering {
                color_attachment_format: format,
                depth_attachment_format: None,
                stencil_attachment_format: None,
            })
        };
        let pass = match &mut self.pass {
            Some(pass) => pass,
            empty => {
                let device = gpu.device();
                // SAFETY: the device is alive and the settings are valid.
                let pool = unsafe {
                    device.create_command_pool(
                        &vk::CommandPoolCreateInfo::default()
                            .queue_family_index(gpu.queue_family()),
                        None,
                    )
                }
                .map_err(RenderError::driver(doing))?;
                let drawer = egui_ash_renderer::Renderer::with_default_allocator(
                    gpu.instance(),
                    gpu.physical_device(),
                    device.clone(),
                    mode(),
                    Options {
                        in_flight_frames: in_flight.max(1),
                        srgb_framebuffer: stores_srgb(format),
                        ..Options::default()
                    },
                );
                let drawer = match drawer {
                    Ok(drawer) => drawer,
                    Err(error) => {
                        // SAFETY: the pool was made above and never used.
                        unsafe { device.destroy_command_pool(pool, None) };
                        return Err(failed(doing)(error));
                    }
                };
                empty.insert(Pass {
                    drawer,
                    pool,
                    format,
                    gpu: Arc::clone(gpu),
                })
            }
        };
        let changes = !self.waiting.is_empty();
        if changes || pass.format != format {
            // A texture or pipeline about to be replaced may be in a frame
            // the graphics card has not finished.
            // SAFETY: the device is alive, and this thread is the only one
            // that uses it.
            unsafe { gpu.device().device_wait_idle() }.map_err(RenderError::driver(doing))?;
        }
        if pass.format != format {
            if stores_srgb(pass.format) != stores_srgb(format) {
                log::warn!(
                    "The window changed how it stores colour; the editor's panels may look \
                     too dark or too light until the editor is started again."
                );
            }
            pass.drawer.set_render_mode(mode()).map_err(failed(doing))?;
            pass.format = format;
        }
        for delta in &mut self.waiting {
            for (id, changes) in &delta.set {
                for change in changes {
                    pass.drawer
                        .set_texture(gpu.queue(), pass.pool, *id, change)
                        .map_err(failed(doing))?;
                }
            }
            for id in &delta.free {
                pass.drawer.free_texture(*id).map_err(failed(doing))?;
            }
            // Applied; `egui` checks that none is thrown away unapplied.
            delta.clear();
        }
        self.waiting.clear();
        Ok(())
    }

    /// Records drawing the panels over an image `extent` in size.
    ///
    /// # Safety
    ///
    /// `commands` must be a command buffer of the device given to
    /// [`prepare`](Self::prepare), being recorded inside a rendering pass
    /// whose one colour image has the format prepared for; `prepare` must
    /// have been called since the panels were last set.
    pub(crate) unsafe fn draw(
        &mut self,
        commands: vk::CommandBuffer,
        extent: vk::Extent2D,
    ) -> Result<(), RenderError> {
        let Some(pass) = &mut self.pass else {
            return Ok(());
        };
        if self.shapes.is_empty() || self.pixels_per_point <= 0.0 {
            return Ok(());
        }
        pass.drawer
            .cmd_draw(commands, extent, self.pixels_per_point, &self.shapes)
            .map_err(failed("drawing the editor's panels"))
    }
}
