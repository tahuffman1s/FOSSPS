//! The last pass: maps the scene's high-dynamic-range light to the display.

use std::io::Cursor;
use std::sync::Arc;

use ash::vk;

use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::swapchain::COLOUR;
use crate::ui_pass::UiPass;

/// The shaders, compiled from `shaders/tonemap.wgsl` by the build script.
const SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tonemap.spv"));

/// The size in bytes of the shader's `Settings`: an `f32` and a `u32`.
const SETTINGS_BYTES: usize = 8;

/// What happens to an image after the picture is drawn into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Then {
    /// It is shown in the window.
    Show,
    /// Its pixels are copied out.
    Copy,
}

/// An image to draw the finished picture into.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Canvas {
    pub(crate) image: vk::Image,
    /// A view of the whole image.
    pub(crate) view: vk::ImageView,
    /// The image's size in pixels.
    pub(crate) extent: vk::Extent2D,
}

/// How much brighter the picture is for an exposure of `stops`.
///
/// One stop doubles the light, as on a camera.
pub(crate) fn exposure_scale(stops: f32) -> f32 {
    stops.exp2()
}

/// Whether images of `format` apply the sRGB curve themselves when written.
pub(crate) fn stores_srgb(format: vk::Format) -> bool {
    matches!(
        format,
        vk::Format::B8G8R8A8_SRGB | vk::Format::R8G8B8A8_SRGB | vk::Format::A8B8G8R8_SRGB_PACK32
    )
}

/// Draws the scene, tone mapped, into images of one colour format.
pub(crate) struct TonemapPipeline {
    gpu: Arc<Gpu>,
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    pool: vk::DescriptorPool,
    /// Says which scene image the shader reads.
    set: vk::DescriptorSet,
    /// The colour format of the images this can draw into.
    pub(crate) format: vk::Format,
}

/// What is drawn over the tone-mapped scene, in this order.
#[derive(Default)]
pub(crate) struct Over<'a> {
    /// The HUD and menus.
    pub(crate) ui: Option<&'a UiPass>,
    /// The editor's panels.
    #[cfg(feature = "panels")]
    pub(crate) panels: Option<&'a mut crate::panels::PanelHost>,
}

impl<'a> Over<'a> {
    /// The HUD and menus alone.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn ui(ui: Option<&'a UiPass>) -> Self {
        Self {
            ui,
            #[cfg(feature = "panels")]
            panels: None,
        }
    }
}

impl TonemapPipeline {
    /// Builds the pipeline for drawing into images of `format`.
    pub(crate) fn new(gpu: &Arc<Gpu>, format: vk::Format) -> Result<Self, RenderError> {
        let doing = "preparing the shaders";
        let device = gpu.device();
        let code = ash::util::read_spv(&mut Cursor::new(SHADER)).map_err(|_| {
            // Only possible if the build script wrote a broken file.
            RenderError::Driver {
                doing,
                code: vk::Result::ERROR_INVALID_SHADER_NV,
            }
        })?;
        let mut tonemap = Self {
            gpu: Arc::clone(gpu),
            set_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            format,
        };
        // Dropping `tonemap` cleans up whatever was created; destroying a null handle is allowed.

        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)];
        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::SAMPLED_IMAGE)
            .descriptor_count(1)];
        let ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::FRAGMENT)
            .size(SETTINGS_BYTES as u32)];

        // SAFETY: the device is alive, and each object is created from valid
        // settings that live until the call returns, and stored in `tonemap`
        // straight away.
        unsafe {
            tonemap.set_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            let set_layouts = [tonemap.set_layout];
            tonemap.layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&set_layouts)
                        .push_constant_ranges(&ranges),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            tonemap.pool = device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&sizes),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            tonemap.set = device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(tonemap.pool)
                        .set_layouts(&set_layouts),
                )
                .map_err(RenderError::driver(doing))?
                .first()
                .copied()
                .unwrap_or_default();
        }

        // SAFETY: `code` is SPIR-V produced and validated by naga at build time.
        let module = unsafe {
            device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
        }
        .map_err(RenderError::driver(doing))?;

        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(module)
                .name(c"vertex"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(module)
                .name(c"fragment"),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        // The viewport and scissor are set while drawing, so a resized window
        // does not need a new pipeline.
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let attachments = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let formats = [format];
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(tonemap.layout)
            .push_next(&mut rendering);

        // SAFETY: `info` and everything it points to are alive for the call;
        // the shader module holds both entry points named in `stages`. The
        // module is only needed while the pipeline is being created.
        let created = unsafe {
            let created = device.create_graphics_pipelines(
                vk::PipelineCache::null(),
                std::slice::from_ref(&info),
                None,
            );
            device.destroy_shader_module(module, None);
            created
        };
        match created {
            Ok(pipelines) if !pipelines.is_empty() => {
                tonemap.pipeline = pipelines[0];
                Ok(tonemap)
            }
            failed => {
                let code = failed.map_or_else(|(_, code)| code, |_| vk::Result::ERROR_UNKNOWN);
                Err(RenderError::driver(doing)(code))
            }
        }
    }

    /// Makes later frames read the scene from `view`.
    ///
    /// # Safety
    ///
    /// `view` must be a live view of a [`SceneTarget`](crate::scene::SceneTarget)
    /// on this pipeline's device, and the graphics card must not be running
    /// any commands recorded by [`record`](Self::record).
    pub(crate) unsafe fn read_from(&self, view: vk::ImageView) {
        let images = [vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let writes = [vk::WriteDescriptorSet::default()
            .dst_set(self.set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
            .image_info(&images)];
        // SAFETY: the set belongs to this pipeline, the caller guarantees the
        // view is live and that the set is not in use.
        unsafe { self.gpu.device().update_descriptor_sets(&writes, &[]) };
    }

    /// Records drawing the tone-mapped scene over the whole of `canvas`.
    ///
    /// The scene's light is multiplied by `exposure` first. The image's
    /// previous contents are discarded. What `over` holds is drawn on top:
    /// the HUD and menus it was prepared with, then the editor's panels.
    ///
    /// # Errors
    ///
    /// Fails if the editor's panels cannot be drawn. The commands are left
    /// complete, but the frame should not be shown.
    ///
    /// # Safety
    ///
    /// `commands` must be a command buffer of this pipeline's device that is
    /// being recorded. `canvas` must be a live colour image of this
    /// pipeline's format with its whole-image view and its size, and the
    /// image must not be in use by other work on the graphics card. The scene
    /// given to [`read_from`](Self::read_from) must be alive, at least
    /// `extent` in size, and ready to be read by shaders when these commands run.
    /// `over.ui` must have been made for this pipeline's format, and
    /// [`prepare`](UiPass::prepare)d with `commands` for this frame; the
    /// panels must have been prepared for this pipeline's format since they
    /// were last set.
    pub(crate) unsafe fn record(
        &self,
        commands: vk::CommandBuffer,
        canvas: Canvas,
        exposure: f32,
        over: Over<'_>,
        then: Then,
    ) -> Result<(), RenderError> {
        let device = self.gpu.device();
        let Canvas {
            image,
            view,
            extent,
        } = canvas;
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        };

        let to_draw = [vk::ImageMemoryBarrier2::default()
            // Waiting at this stage lines up with the wait on the image
            // becoming available when the commands are submitted.
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(COLOUR)];

        let (stage, access, layout) = match then {
            Then::Show => (
                vk::PipelineStageFlags2::NONE,
                vk::AccessFlags2::NONE,
                vk::ImageLayout::PRESENT_SRC_KHR,
            ),
            Then::Copy => (
                vk::PipelineStageFlags2::COPY,
                vk::AccessFlags2::TRANSFER_READ,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            ),
        };
        let to_finish = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .dst_stage_mask(stage)
            .dst_access_mask(access)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(layout)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(COLOUR)];

        // Every pixel is drawn, so there is nothing to clear.
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::DONT_CARE)
            .store_op(vk::AttachmentStoreOp::STORE)];
        let rendering = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&attachments);
        // Sizes are far below the point where f32 stops being exact.
        #[allow(clippy::cast_precision_loss)]
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: extent.width as f32,
            height: extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let mut settings = [0_u8; SETTINGS_BYTES];
        settings[..4].copy_from_slice(&exposure.to_ne_bytes());
        settings[4..].copy_from_slice(&u32::from(!stores_srgb(self.format)).to_ne_bytes());

        // SAFETY: the caller guarantees the command buffer is recording and
        // the images and views are live and match this pipeline; every struct
        // passed lives until its call returns.
        unsafe {
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_draw),
            );
            device.cmd_begin_rendering(commands, &rendering);
            device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            device.cmd_bind_descriptor_sets(
                commands,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[self.set],
                &[],
            );
            device.cmd_push_constants(
                commands,
                self.layout,
                vk::ShaderStageFlags::FRAGMENT,
                0,
                &settings,
            );
            device.cmd_set_viewport(commands, 0, &[viewport]);
            device.cmd_set_scissor(commands, 0, &[area]);
            device.cmd_draw(commands, 3, 1, 0, 0);
            if let Some(ui) = over.ui {
                ui.draw(commands, extent);
            }
            #[cfg(feature = "panels")]
            let panels = match over.panels {
                Some(panels) => panels.draw(commands, extent),
                None => Ok(()),
            };
            #[cfg(not(feature = "panels"))]
            let panels = Ok(());
            // The pass is ended even if the panels failed, so the commands
            // are whole whatever the caller does with them.
            device.cmd_end_rendering(commands);
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_finish),
            );
            panels
        }
    }
}

impl Drop for TonemapPipeline {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping the pipeline, so it is not in use. Destroying the pool
        // frees its set.
        unsafe {
            let device = self.gpu.device();
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.set_layout, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_stop_doubles_the_light() {
        assert!((exposure_scale(0.0) - 1.0).abs() < 1e-6);
        assert!((exposure_scale(1.0) - 2.0).abs() < 1e-6);
        assert!((exposure_scale(-2.0) - 0.25).abs() < 1e-6);
    }

    #[test]
    fn only_srgb_images_are_left_to_encode_themselves() {
        assert!(stores_srgb(vk::Format::B8G8R8A8_SRGB));
        assert!(!stores_srgb(vk::Format::R8G8B8A8_UNORM));
        assert!(!stores_srgb(vk::Format::A2B10G10R10_UNORM_PACK32));
    }
}
