//! Draws the HUD and menus over the finished picture.
//!
//! This runs inside the tone mapping's pass, after the scene has been
//! upscaled and tone mapped, so text and boxes are drawn at the full size
//! of the window and are never blurred by upscaling or dimmed by exposure.
//! What to draw is worked out by [`Overlay`]; this file hands it to the
//! graphics card.

use std::io::Cursor;
use std::sync::Arc;

use ash::vk;

use crate::buffer::Buffer;
use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::overlay::{ATLAS_HEIGHT, ATLAS_WIDTH, MOST_QUADS, Overlay, QUAD_BYTES};
use crate::scene::Picture;
use crate::swapchain::COLOUR;
use crate::tonemap::stores_srgb;

/// The shaders, compiled from `shaders/ui.wgsl` by the build script.
const SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/ui.spv"));

/// The size in bytes of the shader's `Screen`: two `f32`s and a `u32`,
/// rounded up to a multiple of eight as the shader lays it out.
const SCREEN_BYTES: usize = 16;

/// One byte a pixel: how much of it a letter covers.
const ATLAS_FORMAT: vk::Format = vk::Format::R8_UNORM;

/// What one frame in progress needs for itself.
struct Slot {
    /// The frame's boxes and letters.
    quads: Buffer,
    /// New rows of the atlas, on their way to the graphics card.
    rows: Buffer,
}

/// Draws an [`Overlay`] into images of one colour format.
pub(crate) struct UiPass {
    // Dropped before `drop` runs out: the buffers free themselves.
    slots: Vec<Slot>,
    gpu: Arc<Gpu>,
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    pool: vk::DescriptorPool,
    /// Points the shader at the atlas.
    set: vk::DescriptorSet,
    sampler: vk::Sampler,
    atlas: Picture,
    /// Whether the atlas on the graphics card has been given anything yet.
    filled: bool,
    /// Which slot the frame being recorded uses and how many quads it has.
    frame: Option<(usize, u32)>,
    /// The colour format of the images this can draw into.
    format: vk::Format,
}

impl UiPass {
    /// Builds the pass for drawing into images of `format`, for up to
    /// `frames` frames in progress at once.
    pub(crate) fn new(
        gpu: &Arc<Gpu>,
        format: vk::Format,
        frames: usize,
    ) -> Result<Self, RenderError> {
        let doing = "preparing to draw the HUD and menus";
        let device = gpu.device();
        let code = ash::util::read_spv(&mut Cursor::new(SHADER)).map_err(|_| {
            // Only possible if the build script wrote a broken file.
            RenderError::Driver {
                doing,
                code: vk::Result::ERROR_INVALID_SHADER_NV,
            }
        })?;
        let mut pass = Self {
            slots: Vec::with_capacity(frames.max(1)),
            gpu: Arc::clone(gpu),
            set_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            pool: vk::DescriptorPool::null(),
            set: vk::DescriptorSet::null(),
            sampler: vk::Sampler::null(),
            atlas: Picture::null(),
            filled: false,
            frame: None,
            format,
        };
        // Dropping `pass` cleans up whatever was created; destroying a null handle is allowed.

        for _ in 0..frames.max(1) {
            pass.slots.push(Slot {
                quads: Buffer::writable(
                    gpu,
                    (MOST_QUADS * QUAD_BYTES) as u64,
                    vk::BufferUsageFlags::VERTEX_BUFFER,
                    doing,
                )?,
                rows: Buffer::writable(
                    gpu,
                    u64::from(ATLAS_WIDTH * ATLAS_HEIGHT),
                    vk::BufferUsageFlags::TRANSFER_SRC,
                    doing,
                )?,
            });
        }
        pass.atlas.create(
            gpu,
            vk::Extent2D {
                width: ATLAS_WIDTH,
                height: ATLAS_HEIGHT,
            },
            ATLAS_FORMAT,
            vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
            COLOUR,
            doing,
        )?;

        let bindings = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .descriptor_count(1)
                .stage_flags(vk::ShaderStageFlags::FRAGMENT),
        ];
        let sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(1),
        ];
        let ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .size(SCREEN_BYTES as u32)];

        // SAFETY: the device is alive, and each object is created from valid
        // settings that live until the call returns, and stored in `pass`
        // straight away. The set is pointed at the atlas and sampler made
        // just before, which live as long as it does.
        unsafe {
            pass.set_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            let set_layouts = [pass.set_layout];
            pass.layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&set_layouts)
                        .push_constant_ranges(&ranges),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            pass.pool = device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(1)
                        .pool_sizes(&sizes),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            pass.set = device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(pass.pool)
                        .set_layouts(&set_layouts),
                )
                .map_err(RenderError::driver(doing))?
                .first()
                .copied()
                .unwrap_or_default();
            // Letters are drawn pixel for pixel, where smoothing changes
            // nothing; it only shows on text larger than the largest kept.
            pass.sampler = device
                .create_sampler(
                    &vk::SamplerCreateInfo::default()
                        .mag_filter(vk::Filter::LINEAR)
                        .min_filter(vk::Filter::LINEAR)
                        .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
                        .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            let images = [vk::DescriptorImageInfo::default()
                .image_view(pass.atlas.view)
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let samplers = [vk::DescriptorImageInfo::default().sampler(pass.sampler)];
            let writes = [
                vk::WriteDescriptorSet::default()
                    .dst_set(pass.set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(&images),
                vk::WriteDescriptorSet::default()
                    .dst_set(pass.set)
                    .dst_binding(1)
                    .descriptor_type(vk::DescriptorType::SAMPLER)
                    .image_info(&samplers),
            ];
            device.update_descriptor_sets(&writes, &[]);
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
        // One entry in the buffer for each box or letter, not for each corner.
        let vertex_bindings = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(QUAD_BYTES as u32)
            .input_rate(vk::VertexInputRate::INSTANCE)];
        let attributes = [
            vk::VertexInputAttributeDescription::default()
                .location(0)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .location(1)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(16),
            vk::VertexInputAttributeDescription::default()
                .location(2)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(32),
            vk::VertexInputAttributeDescription::default()
                .location(3)
                .format(vk::Format::R32G32_SFLOAT)
                .offset(48),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&vertex_bindings)
            .vertex_attribute_descriptions(&attributes);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_STRIP);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // Each quad is laid over what is there by how solid it is. The
        // picture's own alpha is left alone: a window must stay solid.
        let attachments = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(true)
            .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
            .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
            .color_blend_op(vk::BlendOp::ADD)
            .src_alpha_blend_factor(vk::BlendFactor::ZERO)
            .dst_alpha_blend_factor(vk::BlendFactor::ONE)
            .alpha_blend_op(vk::BlendOp::ADD)
            .color_write_mask(
                vk::ColorComponentFlags::R
                    | vk::ColorComponentFlags::G
                    | vk::ColorComponentFlags::B,
            )];
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
            .layout(pass.layout)
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
                pass.pipeline = pipelines[0];
                Ok(pass)
            }
            failed => {
                let code = failed.map_or_else(|(_, code)| code, |_| vk::Result::ERROR_UNKNOWN);
                Err(RenderError::driver(doing)(code))
            }
        }
    }

    /// The colour format of the images this draws into.
    pub(crate) fn format(&self) -> vk::Format {
        self.format
    }

    /// Gets `overlay` ready to be drawn by the frame being recorded: its
    /// boxes and letters go to the graphics card, with any letters it has
    /// not seen before.
    ///
    /// Call this before the pass that [`draw`](Self::draw) is recorded in
    /// begins.
    ///
    /// # Errors
    ///
    /// Fails if the graphics driver reports a failure.
    ///
    /// # Safety
    ///
    /// `commands` must be a command buffer of this pass's device that is
    /// being recorded, outside a render pass. `frame` picks which of the
    /// pass's frames this is, and the commands last recorded with the same
    /// `frame` must have finished. The pass must stay alive until the
    /// commands have run.
    pub(crate) unsafe fn prepare(
        &mut self,
        commands: vk::CommandBuffer,
        frame: usize,
        overlay: &mut Overlay,
    ) -> Result<(), RenderError> {
        self.frame = None;
        let count = overlay.count();
        let index = frame % self.slots.len().max(1);
        let Some(slot) = self.slots.get(index) else {
            return Ok(());
        };
        if count == 0 {
            // Nothing reads the atlas, so new letters can wait.
            return Ok(());
        }
        // SAFETY: the buffer was made writable and long enough for the most
        // quads an overlay holds, and the caller guarantees the commands
        // that read it last have finished.
        unsafe { slot.quads.write(overlay.quads()) }?;

        let new = if self.filled {
            overlay.changed()
        } else {
            overlay.everything()
        };
        if let Some((rows, pixels)) = new {
            let device = self.gpu.device();
            // What is in the atlas already is kept, except the first time,
            // when there is nothing to keep.
            let (before, readers, reading) = if self.filled {
                (
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    vk::PipelineStageFlags2::FRAGMENT_SHADER,
                    vk::AccessFlags2::SHADER_SAMPLED_READ,
                )
            } else {
                (
                    vk::ImageLayout::UNDEFINED,
                    vk::PipelineStageFlags2::NONE,
                    vk::AccessFlags2::NONE,
                )
            };
            let to_fill = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(readers)
                .src_access_mask(reading)
                .dst_stage_mask(vk::PipelineStageFlags2::COPY)
                .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .old_layout(before)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(self.atlas.image)
                .subresource_range(COLOUR)];
            let to_read = [vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COPY)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(self.atlas.image)
                .subresource_range(COLOUR)];
            // Whole rows, as wide as the atlas, so the buffer needs no gaps.
            let copy = [vk::BufferImageCopy::default()
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_offset(vk::Offset3D {
                    x: 0,
                    y: i32::try_from(rows.start).unwrap_or(0),
                    z: 0,
                })
                .image_extent(vk::Extent3D {
                    width: ATLAS_WIDTH,
                    height: rows.end - rows.start,
                    depth: 1,
                })];
            // SAFETY: the rows buffer is writable and as large as the whole
            // atlas, of which `pixels` is a part, and the commands that read
            // it last have finished. The atlas is this pass's own, in the
            // layout the first barrier names: undefined until first filled,
            // and left for shaders to read ever after. Every struct passed
            // lives until its call returns.
            unsafe {
                slot.rows.write(pixels)?;
                device.cmd_pipeline_barrier2(
                    commands,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_fill),
                );
                device.cmd_copy_buffer_to_image(
                    commands,
                    slot.rows.handle,
                    self.atlas.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &copy,
                );
                device.cmd_pipeline_barrier2(
                    commands,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_read),
                );
            }
            overlay.sent();
            self.filled = true;
        }
        self.frame = Some((index, count));
        Ok(())
    }

    /// Records drawing what [`prepare`](Self::prepare) got ready over a
    /// picture `extent` pixels in size.
    ///
    /// # Safety
    ///
    /// `commands` must be the command buffer `prepare` was last called
    /// with, inside a render pass whose one colour image is `extent` in
    /// size and of this pass's format.
    pub(crate) unsafe fn draw(&self, commands: vk::CommandBuffer, extent: vk::Extent2D) {
        let Some((index, count)) = self.frame else {
            return;
        };
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        let device = self.gpu.device();
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        };
        // Sizes are far below the point where f32 stops being exact.
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (extent.width as f32, extent.height as f32);
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width,
            height,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        let mut screen = [0_u8; SCREEN_BYTES];
        screen[..4].copy_from_slice(&width.to_ne_bytes());
        screen[4..8].copy_from_slice(&height.to_ne_bytes());
        screen[8..12].copy_from_slice(&u32::from(stores_srgb(self.format)).to_ne_bytes());

        // SAFETY: the caller guarantees the command buffer is recording
        // inside a matching render pass; the pipeline, set and buffer are
        // this pass's own, and `prepare` filled the buffer with `count`
        // quads and left the atlas ready to be read.
        unsafe {
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
                vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                0,
                &screen,
            );
            device.cmd_set_viewport(commands, 0, &[viewport]);
            device.cmd_set_scissor(commands, 0, &[area]);
            device.cmd_bind_vertex_buffers(commands, 0, &[slot.quads.handle], &[0]);
            device.cmd_draw(commands, 4, count, 0, 0);
        }
    }
}

impl Drop for UiPass {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping the pass, so nothing here is in use. Destroying the pool
        // frees its set.
        unsafe {
            let device = self.gpu.device();
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.set_layout, None);
            device.destroy_sampler(self.sampler, None);
            self.atlas.destroy(&self.gpu);
        }
    }
}
