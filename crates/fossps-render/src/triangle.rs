//! The first thing on screen: one coloured triangle over a dark background.

use std::io::Cursor;
use std::sync::Arc;

use ash::vk;

use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::swapchain::COLOUR;

/// The triangle shaders, compiled from `shaders/triangle.wgsl` by the build script.
const SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/triangle.spv"));

/// The background colour in linear light: sRGB `#181C24`.
pub(crate) const BACKGROUND: [f32; 4] = [0.0091, 0.0116, 0.0176, 1.0];

/// What happens to an image after the triangle is drawn into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Then {
    /// It is shown in the window.
    Show,
    /// Its pixels are copied out.
    #[cfg_attr(not(test), allow(dead_code))]
    Copy,
}

/// Draws the triangle into images of one colour format.
pub(crate) struct TrianglePipeline {
    gpu: Arc<Gpu>,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    /// The colour format of the images this can draw into.
    pub(crate) format: vk::Format,
}

impl TrianglePipeline {
    /// Builds the pipeline for images of `format`.
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

        // SAFETY: `code` is SPIR-V produced and validated by naga at build time.
        let module = unsafe {
            device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
        }
        .map_err(RenderError::driver(doing))?;

        // SAFETY: an empty layout needs nothing but a live device.
        let layout = unsafe {
            device.create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None)
        };
        let layout = match layout {
            Ok(layout) => layout,
            Err(code) => {
                // SAFETY: the module was created above and nothing uses it yet.
                unsafe { device.destroy_shader_module(module, None) };
                return Err(RenderError::driver(doing)(code));
            }
        };

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
            .layout(layout)
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
            Ok(pipelines) if !pipelines.is_empty() => Ok(Self {
                gpu: Arc::clone(gpu),
                layout,
                pipeline: pipelines[0],
                format,
            }),
            failed => {
                // SAFETY: no pipeline was created, so nothing uses the layout.
                unsafe { device.destroy_pipeline_layout(layout, None) };
                let code = failed.map_or_else(|(_, code)| code, |_| vk::Result::ERROR_UNKNOWN);
                Err(RenderError::driver(doing)(code))
            }
        }
    }

    /// Records clearing `image` and drawing the triangle into it.
    ///
    /// The image's previous contents are discarded.
    ///
    /// # Safety
    ///
    /// `commands` must be a command buffer of this pipeline's device that is
    /// being recorded. `image` and `view` must be a live colour image of this
    /// pipeline's format and its whole-image view, `extent` its size, and the
    /// image must not be in use by other work on the graphics card.
    pub(crate) unsafe fn record(
        &self,
        commands: vk::CommandBuffer,
        image: vk::Image,
        view: vk::ImageView,
        extent: vk::Extent2D,
        then: Then,
    ) {
        let device = self.gpu.device();
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

        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: BACKGROUND,
                },
            })];
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

        // SAFETY: the caller guarantees the command buffer is recording and
        // the image and view are live and match this pipeline; every struct
        // passed lives until its call returns.
        unsafe {
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_draw),
            );
            device.cmd_begin_rendering(commands, &rendering);
            device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
            device.cmd_set_viewport(commands, 0, &[viewport]);
            device.cmd_set_scissor(commands, 0, &[area]);
            device.cmd_draw(commands, 3, 1, 0, 0);
            device.cmd_end_rendering(commands);
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_finish),
            );
        }
    }
}

impl Drop for TrianglePipeline {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping the pipeline, so it is not in use.
        unsafe {
            self.gpu.device().destroy_pipeline(self.pipeline, None);
            self.gpu.device().destroy_pipeline_layout(self.layout, None);
        }
    }
}
