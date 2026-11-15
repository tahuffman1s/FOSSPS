//! Draws a scene's meshes: each in its material's colour, nearer ones in
//! front of farther ones.

use std::io::Cursor;
use std::sync::Arc;

use ash::vk;
use fossps_assets::{Material, MeshPart, Scene};

use crate::buffer::Buffer;
use crate::camera::{Mat4, normal_matrix};
use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::grid::{self, grid_bytes};
use crate::scene::{DEPTH, SceneTarget};
use crate::swapchain::COLOUR;

/// The mesh shaders, compiled from `shaders/mesh.wgsl` by the build script.
const SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mesh.spv"));

/// The background colour in linear light: sRGB `#181C24`.
pub(crate) const BACKGROUND: [f32; 4] = [0.0091, 0.0116, 0.0176, 1.0];

/// The bytes one corner point takes on the graphics card: a position and a
/// surface direction of three floats each, and a colour of four.
const VERTEX_BYTES: usize = 40;

/// The size in bytes of the shader's `Draw`: a 4 by 4 matrix, a 3 by 3
/// matrix padded to three columns of four, and a colour. Vulkan promises
/// room for exactly this much.
const DRAW_BYTES: usize = 128;

/// The size in bytes of the shader's `View`: three 4 by 4 matrices.
const VIEW_BYTES: usize = 192;

/// How the scene is seen in one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct View {
    /// Takes a point in the world to where it is drawn, including the
    /// part-of-a-pixel shift the upscaler asks for.
    pub(crate) clip: Mat4,
    /// The same without the shift.
    pub(crate) steady: Mat4,
    /// `steady` as it was in the last frame.
    pub(crate) before: Mat4,
}

impl View {
    fn bytes(&self) -> [u8; VIEW_BYTES] {
        let numbers = [self.clip, self.steady, self.before];
        let mut bytes = [0_u8; VIEW_BYTES];
        for (slot, number) in bytes
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(numbers.iter().flatten().flatten())
        {
            *slot = number.to_ne_bytes();
        }
        bytes
    }
}

/// What a frame shows of the world.
#[derive(Clone, Copy, Default)]
pub(crate) struct Contents<'a> {
    /// The level's meshes, if a level has been set.
    pub(crate) meshes: Option<&'a Meshes>,
    /// Whether the editor's ground grid is drawn over them.
    pub(crate) grid: bool,
}

/// One mesh part placed in the world.
struct Draw {
    /// Where the part stands.
    world: Mat4,
    /// Turns its surface directions to match.
    normal: [[f32; 4]; 3],
    /// Its material's base colour.
    colour: [f32; 4],
    /// Where its triangles start in the index buffer, and how many numbers they are.
    first_index: u32,
    index_count: u32,
    /// Where its corner points start in the vertex buffer.
    first_vertex: i32,
    /// Its triangles run clockwise seen from the front, because it is mirrored.
    mirrored: bool,
    /// Both of its sides are drawn.
    double_sided: bool,
}

/// Where one mesh part's data sits in the buffers, or `None` if it was left out.
type Placed = Option<(u32, u32, i32)>;

/// A scene's meshes, copied to the graphics card and ready to draw.
pub(crate) struct Meshes {
    /// `None` when the scene has nothing to draw.
    buffers: Option<(Buffer, Buffer)>,
    draws: Vec<Draw>,
}

impl Meshes {
    /// Copies the meshes of `scene` to the graphics card.
    ///
    /// Every mesh is stored once however many times it is placed. A part
    /// whose lists do not match is left out with a warning.
    pub(crate) fn new(gpu: &Arc<Gpu>, scene: &Scene) -> Result<Self, RenderError> {
        let doing = "copying the level's meshes to the graphics card";
        let mut vertices: Vec<u8> = Vec::new();
        let mut indices: Vec<u8> = Vec::new();
        let mut placed: Vec<Vec<Placed>> = Vec::with_capacity(scene.meshes.len());
        for (number, mesh) in scene.meshes.iter().enumerate() {
            let mut parts = Vec::with_capacity(mesh.parts.len());
            for part in &mesh.parts {
                let range = append(part, &mut vertices, &mut indices);
                if range.is_none() && !part.indices.is_empty() {
                    log::warn!(
                        "Mesh {} ({number}) has a part that cannot be drawn: its triangles \
                         name points it does not have, or its lists differ in length. \
                         Export the file again; if you built the scene in code, check that \
                         `positions`, `normals` and `colors` are the same length.",
                        mesh.name.as_deref().unwrap_or("without a name"),
                    );
                }
                parts.push(range);
            }
            placed.push(parts);
        }

        let fallback = Material::default();
        let mut draws = Vec::new();
        for instance in &scene.instances {
            let (Some(mesh), Some(parts)) = (
                scene.meshes.get(instance.mesh as usize),
                placed.get(instance.mesh as usize),
            ) else {
                continue;
            };
            for (part, range) in mesh.parts.iter().zip(parts) {
                let Some((first_index, index_count, first_vertex)) = *range else {
                    continue;
                };
                let material = scene
                    .materials
                    .get(part.material as usize)
                    .unwrap_or(&fallback);
                draws.push(Draw {
                    world: instance.transform,
                    normal: normal_matrix(&instance.transform),
                    colour: material.base_color,
                    first_index,
                    index_count,
                    first_vertex,
                    mirrored: instance.mirrored,
                    double_sided: material.double_sided,
                });
            }
        }

        let buffers = if draws.is_empty() {
            None
        } else {
            Some((
                Buffer::holding(gpu, &vertices, vk::BufferUsageFlags::VERTEX_BUFFER, doing)?,
                Buffer::holding(gpu, &indices, vk::BufferUsageFlags::INDEX_BUFFER, doing)?,
            ))
        };
        Ok(Self { buffers, draws })
    }
}

/// Adds one part's points and triangles to the ends of `vertices` and
/// `indices`, and says where they went. Adds nothing and returns `None` if
/// the part is empty or does not hold together.
fn append(part: &MeshPart, vertices: &mut Vec<u8>, indices: &mut Vec<u8>) -> Placed {
    let points = part.positions.len();
    let whole = part.indices.len() - part.indices.len() % 3;
    let colours_fit = part
        .colors
        .as_ref()
        .is_none_or(|colours| colours.len() == points);
    if whole == 0
        || part.normals.len() != points
        || !colours_fit
        || part.indices[..whole]
            .iter()
            .any(|&index| index as usize >= points)
    {
        return None;
    }
    let first_index = u32::try_from(indices.len() / 4).ok()?;
    let index_count = u32::try_from(whole).ok()?;
    let first_vertex = i32::try_from(vertices.len() / VERTEX_BYTES).ok()?;
    // The last point's number must fit too.
    first_index.checked_add(index_count)?;
    first_vertex.checked_add(i32::try_from(points).ok()?)?;

    vertices.reserve(points * VERTEX_BYTES);
    for (number, (position, normal)) in part.positions.iter().zip(&part.normals).enumerate() {
        let colour = part
            .colors
            .as_ref()
            .map_or([1.0; 4], |colours| colours[number]);
        for value in position.iter().chain(normal).chain(&colour) {
            vertices.extend_from_slice(&value.to_ne_bytes());
        }
    }
    indices.reserve(whole * 4);
    for index in &part.indices[..whole] {
        indices.extend_from_slice(&index.to_ne_bytes());
    }
    Some((first_index, index_count, first_vertex))
}

/// Draws meshes into a [`SceneTarget`].
pub(crate) struct MeshPipeline {
    gpu: Arc<Gpu>,
    set_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    pipeline: vk::Pipeline,
    /// Draws the editor's ground grid, with the same layout and view.
    grid: vk::Pipeline,
    pool: vk::DescriptorPool,
    /// The [`View`] of each frame that can be in progress at once, with the
    /// set that points the shader at it.
    views: Vec<(Buffer, vk::DescriptorSet)>,
}

impl MeshPipeline {
    /// Builds the pipeline, for up to `frames` frames in progress at once.
    pub(crate) fn new(gpu: &Arc<Gpu>, frames: usize) -> Result<Self, RenderError> {
        let doing = "preparing the shaders";
        let device = gpu.device();
        let mut mesh = Self {
            gpu: Arc::clone(gpu),
            set_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            pipeline: vk::Pipeline::null(),
            grid: vk::Pipeline::null(),
            pool: vk::DescriptorPool::null(),
            views: Vec::with_capacity(frames),
        };
        // Dropping `mesh` cleans up whatever was created; destroying a null handle is allowed.
        let count = u32::try_from(frames.max(1)).unwrap_or(1);
        let code = ash::util::read_spv(&mut Cursor::new(SHADER)).map_err(|_| {
            // Only possible if the build script wrote a broken file.
            RenderError::Driver {
                doing,
                code: vk::Result::ERROR_INVALID_SHADER_NV,
            }
        })?;

        let bindings = [vk::DescriptorSetLayoutBinding::default()
            .binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::VERTEX)];
        let sizes = [vk::DescriptorPoolSize::default()
            .ty(vk::DescriptorType::UNIFORM_BUFFER)
            .descriptor_count(count)];
        let ranges = [vk::PushConstantRange::default()
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)
            .size(DRAW_BYTES as u32)];
        // SAFETY: the device is alive, and each object is created from valid
        // settings that live until the call returns, and stored in `mesh`
        // straight away.
        unsafe {
            mesh.set_layout = device
                .create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            let set_layouts = [mesh.set_layout];
            mesh.layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&set_layouts)
                        .push_constant_ranges(&ranges),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            mesh.pool = device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(count)
                        .pool_sizes(&sizes),
                    None,
                )
                .map_err(RenderError::driver(doing))?;
            for _ in 0..count {
                let buffer = Buffer::writable(
                    gpu,
                    VIEW_BYTES as u64,
                    vk::BufferUsageFlags::UNIFORM_BUFFER,
                    doing,
                )?;
                let set = device
                    .allocate_descriptor_sets(
                        &vk::DescriptorSetAllocateInfo::default()
                            .descriptor_pool(mesh.pool)
                            .set_layouts(&set_layouts),
                    )
                    .map_err(RenderError::driver(doing))?
                    .first()
                    .copied()
                    .unwrap_or_default();
                let buffers = [vk::DescriptorBufferInfo::default()
                    .buffer(buffer.handle)
                    .range(VIEW_BYTES as u64)];
                let writes = [vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(0)
                    .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                    .buffer_info(&buffers)];
                device.update_descriptor_sets(&writes, &[]);
                mesh.views.push((buffer, set));
            }
        }
        let layout = mesh.layout;

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
        let bindings = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(VERTEX_BYTES as u32)
            .input_rate(vk::VertexInputRate::VERTEX)];
        let attributes = [
            vk::VertexInputAttributeDescription::default()
                .location(0)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(0),
            vk::VertexInputAttributeDescription::default()
                .location(1)
                .format(vk::Format::R32G32B32_SFLOAT)
                .offset(12),
            vk::VertexInputAttributeDescription::default()
                .location(2)
                .format(vk::Format::R32G32B32A32_SFLOAT)
                .offset(24),
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        // The viewport and scissor are set while drawing, so a resized window
        // does not need a new pipeline. Which side of a triangle is its front
        // and whether backs are skipped are set for each mesh.
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .line_width(1.0);
        let multisample = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        // Depth is 1 at the camera and 0 far away, so the greater depth wins.
        let depth = vk::PipelineDepthStencilStateCreateInfo::default()
            .depth_test_enable(true)
            .depth_write_enable(true)
            .depth_compare_op(vk::CompareOp::GREATER);
        // The colour, then how far the pixel moved.
        let attachments = [vk::PipelineColorBlendAttachmentState::default()
            .color_write_mask(vk::ColorComponentFlags::RGBA); 2];
        let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
        let dynamic_states = [
            vk::DynamicState::VIEWPORT,
            vk::DynamicState::SCISSOR,
            vk::DynamicState::CULL_MODE,
            vk::DynamicState::FRONT_FACE,
        ];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let formats = [SceneTarget::FORMAT, SceneTarget::MOTION_FORMAT];
        let mut rendering = vk::PipelineRenderingCreateInfo::default()
            .color_attachment_formats(&formats)
            .depth_attachment_format(SceneTarget::DEPTH_FORMAT);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&multisample)
            .depth_stencil_state(&depth)
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
            Ok(pipelines) if !pipelines.is_empty() => {
                mesh.pipeline = pipelines[0];
                mesh.grid = grid::pipeline(gpu, layout)?;
                Ok(mesh)
            }
            failed => {
                let code = failed.map_or_else(|(_, code)| code, |_| vk::Result::ERROR_UNKNOWN);
                Err(RenderError::driver(doing)(code))
            }
        }
    }

    /// Records clearing `target` and drawing `meshes` into it as seen
    /// through `view`, and then the editor's ground grid if `grid` says
    /// where the eye is.
    ///
    /// The target's previous contents are discarded, and its three images
    /// are left ready for shaders to read.
    ///
    /// # Errors
    ///
    /// Fails if the graphics driver refuses to take the view.
    ///
    /// # Safety
    ///
    /// `commands` must be a command buffer of this pipeline's device that is
    /// being recorded. `target` and `meshes` must belong to the same device,
    /// stay alive until the commands have run, and the target must only be
    /// in use by earlier commands on the same queue. `frame` picks which of
    /// the pipeline's frames this is, and the commands last recorded with
    /// the same `frame` must have finished.
    pub(crate) unsafe fn record(
        &self,
        commands: vk::CommandBuffer,
        frame: usize,
        target: &SceneTarget,
        view: &View,
        meshes: Option<&Meshes>,
        grid: Option<[f32; 3]>,
    ) -> Result<(), RenderError> {
        let device = self.gpu.device();
        let Some((view_buffer, view_set)) = self.views.get(frame % self.views.len().max(1)) else {
            return Ok(());
        };
        // SAFETY: the buffer was made writable and `VIEW_BYTES` long, and the
        // caller guarantees the commands that read it last have finished.
        unsafe { view_buffer.write(&view.bytes()) }?;
        let readers =
            vk::PipelineStageFlags2::FRAGMENT_SHADER | vk::PipelineStageFlags2::COMPUTE_SHADER;
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent: target.extent,
        };
        let depth_stages = vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
            | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;

        // The last frame's upscaling or tone mapping may still be reading the images.
        let colour_to_draw = |image: vk::Image| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(readers)
                .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(COLOUR)
        };
        let colour_to_read = |image: vk::Image| {
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
                .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
                .dst_stage_mask(readers)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(COLOUR)
        };
        let to_draw = [
            colour_to_draw(target.colour.image),
            colour_to_draw(target.motion.image),
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(depth_stages | readers)
                .src_access_mask(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_stage_mask(depth_stages)
                .dst_access_mask(
                    vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
                        | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE,
                )
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(target.depth.image)
                .subresource_range(DEPTH),
        ];
        let to_finish = [
            colour_to_read(target.colour.image),
            colour_to_read(target.motion.image),
            vk::ImageMemoryBarrier2::default()
                .src_stage_mask(depth_stages)
                .src_access_mask(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE)
                .dst_stage_mask(readers)
                .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                .old_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(target.depth.image)
                .subresource_range(DEPTH),
        ];

        let attachment = |view: vk::ImageView, clear: [f32; 4]| {
            vk::RenderingAttachmentInfo::default()
                .image_view(view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::CLEAR)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    color: vk::ClearColorValue { float32: clear },
                })
        };
        // The background is one flat colour, so it has not visibly moved.
        let attachments = [
            attachment(target.colour.view, BACKGROUND),
            attachment(target.motion.view, [0.0; 4]),
        ];
        let depth = vk::RenderingAttachmentInfo::default()
            .image_view(target.depth.view)
            .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue {
                    depth: SceneTarget::FARTHEST,
                    stencil: 0,
                },
            });
        let rendering = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&attachments)
            .depth_attachment(&depth);
        // Sizes are far below the point where f32 stops being exact.
        #[allow(clippy::cast_precision_loss)]
        let viewport = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: target.extent.width as f32,
            height: target.extent.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };

        // SAFETY: the caller guarantees the command buffer is recording and
        // the target and meshes are live and of this device; every struct
        // passed lives until its call returns. The draws only name points
        // and triangles that `Meshes::new` put in the buffers.
        unsafe {
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_draw),
            );
            device.cmd_begin_rendering(commands, &rendering);
            device.cmd_set_viewport(commands, 0, &[viewport]);
            device.cmd_set_scissor(commands, 0, &[area]);
            device.cmd_bind_descriptor_sets(
                commands,
                vk::PipelineBindPoint::GRAPHICS,
                self.layout,
                0,
                &[*view_set],
                &[],
            );
            if let Some(Meshes {
                buffers: Some((vertices, indices)),
                draws,
            }) = meshes
            {
                device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, self.pipeline);
                device.cmd_bind_vertex_buffers(commands, 0, &[vertices.handle], &[0]);
                device.cmd_bind_index_buffer(commands, indices.handle, 0, vk::IndexType::UINT32);
                for draw in draws {
                    // Triangles are listed anticlockwise seen from the front;
                    // a mirrored mesh is the other way round.
                    device.cmd_set_front_face(
                        commands,
                        if draw.mirrored {
                            vk::FrontFace::CLOCKWISE
                        } else {
                            vk::FrontFace::COUNTER_CLOCKWISE
                        },
                    );
                    device.cmd_set_cull_mode(
                        commands,
                        if draw.double_sided {
                            vk::CullModeFlags::NONE
                        } else {
                            vk::CullModeFlags::BACK
                        },
                    );
                    device.cmd_push_constants(
                        commands,
                        self.layout,
                        vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                        0,
                        &draw_bytes(draw),
                    );
                    device.cmd_draw_indexed(
                        commands,
                        draw.index_count,
                        1,
                        draw.first_index,
                        draw.first_vertex,
                        0,
                    );
                }
            }
            // After the meshes, so that its see-through lines lie over them.
            if let Some(eye) = grid {
                device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, self.grid);
                device.cmd_push_constants(
                    commands,
                    self.layout,
                    vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT,
                    0,
                    &grid_bytes(eye),
                );
                device.cmd_draw(commands, 6, 1, 0, 0);
            }
            device.cmd_end_rendering(commands);
            device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&to_finish),
            );
        }
        Ok(())
    }
}

/// The shader's `Draw` for one mesh part.
fn draw_bytes(draw: &Draw) -> [u8; DRAW_BYTES] {
    let numbers = draw
        .world
        .iter()
        .chain(&draw.normal)
        .flatten()
        .chain(&draw.colour);
    let mut bytes = [0_u8; DRAW_BYTES];
    for (slot, number) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(numbers) {
        *slot = number.to_ne_bytes();
    }
    bytes
}

impl Drop for MeshPipeline {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping the pipeline, so it is not in use. Destroying the pool
        // frees its sets.
        unsafe {
            let device = self.gpu.device();
            device.destroy_pipeline(self.pipeline, None);
            device.destroy_pipeline(self.grid, None);
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.set_layout, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> MeshPart {
        MeshPart {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..MeshPart::default()
        }
    }

    #[test]
    fn parts_are_laid_end_to_end() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        assert_eq!(
            append(&triangle(), &mut vertices, &mut indices),
            Some((0, 3, 0))
        );
        assert_eq!(
            append(&triangle(), &mut vertices, &mut indices),
            Some((3, 3, 3))
        );
        assert_eq!(vertices.len(), 6 * VERTEX_BYTES);
        assert_eq!(indices.len(), 6 * 4);
        // A point with no colour of its own is white, so the material's colour shows.
        assert_eq!(vertices[24..28], 1.0_f32.to_ne_bytes());
    }

    #[test]
    fn a_part_that_does_not_hold_together_is_left_out() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        let beyond = MeshPart {
            indices: vec![0, 1, 3],
            ..triangle()
        };
        let short = MeshPart {
            normals: vec![[0.0, 0.0, 1.0]; 2],
            ..triangle()
        };
        let colours = MeshPart {
            colors: Some(vec![[1.0; 4]; 2]),
            ..triangle()
        };
        let empty = MeshPart::default();
        for part in [beyond, short, colours, empty] {
            assert_eq!(append(&part, &mut vertices, &mut indices), None);
        }
        assert!(vertices.is_empty() && indices.is_empty());

        // Two numbers left over after the last whole triangle are dropped.
        let ragged = MeshPart {
            indices: vec![0, 1, 2, 0, 1],
            ..triangle()
        };
        assert_eq!(
            append(&ragged, &mut vertices, &mut indices),
            Some((0, 3, 0))
        );
    }

    #[test]
    fn a_draw_fills_the_shader_block_exactly() {
        let mut world = [[0.0; 4]; 4];
        for (axis, column) in world.iter_mut().enumerate() {
            column[axis] = 1.0;
        }
        let draw = Draw {
            world,
            normal: normal_matrix(&world),
            colour: [0.25, 0.5, 0.75, 1.0],
            first_index: 0,
            index_count: 3,
            first_vertex: 0,
            mirrored: false,
            double_sided: false,
        };
        let bytes = draw_bytes(&draw);
        let number = |at: usize| f32::from_ne_bytes([0, 1, 2, 3].map(|byte| bytes[at * 4 + byte]));
        assert_eq!(number(0), 1.0, "the matrix comes first");
        assert_eq!(number(16), 1.0, "then the surface directions");
        assert_eq!(
            [number(28), number(29), number(30), number(31)],
            draw.colour
        );
    }
}
