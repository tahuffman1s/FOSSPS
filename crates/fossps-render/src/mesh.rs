//! Draws a scene's meshes: each with its material, lit by the sun and the
//! sky, nearer ones in front of farther ones.

use std::cell::RefCell;
use std::io::Cursor;
use std::sync::Arc;

use ash::vk;
use fossps_assets::{AlphaMode, LightKind, Material, MeshPart, Scene};

use crate::buffer::Buffer;
use crate::camera::{Mat4, normal_matrix};
use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::grid::{self, grid_bytes};
use crate::material::{self, Materials};
use crate::scene::{DEPTH, SceneTarget};
use crate::swapchain::COLOUR;

/// The mesh shaders, compiled from `shaders/mesh.wgsl` by the build script.
const SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/mesh.spv"));

/// The background colour in linear light: sRGB `#181C24`.
pub(crate) const BACKGROUND: [f32; 4] = [0.0091, 0.0116, 0.0176, 1.0];

/// The bytes one corner point takes on the graphics card: a position of
/// three floats, a surface direction folded into two 16-bit numbers, a
/// tangent and a colour of four bytes each, and two UV maps of two floats.
const VERTEX_BYTES: usize = 40;

/// The size in bytes of the shader's `Draw`: a 4 by 4 matrix, a 3 by 3
/// matrix padded to three columns of four, and one more row of which the
/// first number is used. Vulkan promises room for exactly this much.
const DRAW_BYTES: usize = 128;

/// The size in bytes of the shader's `View`: three 4 by 4 matrices and five
/// rows of four numbers.
const VIEW_BYTES: usize = 272;

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
    /// Where the eye is, in metres.
    pub(crate) eye: [f32; 3],
    /// How many sizes larger than usual textures are read (a negative mip
    /// bias, given as a positive number), for a picture that is upscaled.
    pub(crate) sharper: f32,
}

impl View {
    fn bytes(&self, lighting: &Lighting) -> [u8; VIEW_BYTES] {
        let row = |[x, y, z]: [f32; 3], last: f32| [x, y, z, last];
        let rows = [
            row(self.eye, -self.sharper),
            row(lighting.towards_sun, 0.0),
            row(lighting.sun, 0.0),
            row(lighting.sky, 0.0),
            row(lighting.ground, 0.0),
        ];
        let numbers = [self.clip, self.steady, self.before];
        let mut bytes = [0_u8; VIEW_BYTES];
        for (slot, number) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(
            numbers
                .iter()
                .flatten()
                .flatten()
                .chain(rows.iter().flatten()),
        ) {
            *slot = number.to_ne_bytes();
        }
        bytes
    }
}

/// The light a scene is drawn in: one sun, and the sky and ground around it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lighting {
    /// The direction from any surface to the sun; length one.
    pub(crate) towards_sun: [f32; 3],
    /// The sun's light on a surface that faces it (red, green, blue; linear).
    pub(crate) sun: [f32; 3],
    /// What a matte white surface facing straight up shows of the sky alone.
    pub(crate) sky: [f32; 3],
    /// What a matte white surface facing straight down shows of the ground.
    pub(crate) ground: [f32; 3],
}

impl Default for Lighting {
    /// A warm sun high in the sky to the right and behind the default
    /// camera, under a cool sky.
    ///
    /// The two add up so that a matte surface facing straight up shows
    /// exactly its own colour, which keeps a level that sets no light of its
    /// own looking the way its author painted it.
    fn default() -> Self {
        let towards_sun = Self::TOWARDS_SUN;
        // What the sun adds to a surface facing up, out of a total of 1.
        let share = [0.70, 0.65, 0.58];
        Self {
            towards_sun,
            // A matte surface shows the light on it divided by pi.
            sun: share.map(|part| part * std::f32::consts::PI / towards_sun[1]),
            sky: share.map(|part| 1.0 - part),
            ground: Self::GROUND,
        }
    }
}

impl Lighting {
    /// Up 50 degrees, to the right and towards the default camera. Length one.
    const TOWARDS_SUN: [f32; 3] = [0.399_44, 0.768_92, 0.499_3];
    const GROUND: [f32; 3] = [0.20, 0.19, 0.17];

    /// The light `scene` asks for: its first sun, with the usual sky and
    /// ground, or the usual sun too if it has none.
    pub(crate) fn of(scene: &Scene) -> Self {
        let usual = Self::default();
        let sun = scene.lights.iter().find_map(|light| {
            if light.kind != LightKind::Sun {
                return None;
            }
            let [x, y, z] = light.direction;
            let length = (x * x + y * y + z * z).sqrt();
            let sound = length.is_finite()
                && length > 0.0
                && light.intensity.is_finite()
                && light.color.iter().all(|part| part.is_finite());
            sound.then(|| {
                (
                    // It shines the way it points, so it is found the other way.
                    [-x / length, -y / length, -z / length],
                    light.color.map(|part| (part * light.intensity).max(0.0)),
                )
            })
        });
        match sun {
            Some((towards_sun, sun)) => Self {
                towards_sun,
                sun,
                ..usual
            },
            None => usual,
        }
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

/// How a surface covers what is behind it. Drawn in this order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Surface {
    /// All of it hides what is behind.
    Solid,
    /// Its alpha cuts holes in it.
    CutOut,
    /// What is behind shows through, by its alpha.
    SeeThrough,
}

impl Surface {
    /// In the order of the pipelines that draw them.
    const ALL: [Self; 3] = [Self::Solid, Self::CutOut, Self::SeeThrough];

    fn of(material: &Material) -> Self {
        match material.alpha_mode {
            AlphaMode::Opaque => Self::Solid,
            AlphaMode::Mask { .. } => Self::CutOut,
            AlphaMode::Blend => Self::SeeThrough,
        }
    }
}

/// One mesh part placed in the world.
struct Draw {
    /// Where the part stands.
    world: Mat4,
    /// Turns its surface directions to match.
    normal: [[f32; 4]; 3],
    /// Which of the scene's materials it is drawn with, as
    /// [`Materials::set`] numbers them.
    material: u32,
    surface: Surface,
    /// The middle of the part in the world, which see-through parts are
    /// put in order by.
    middle: [f32; 3],
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

/// A scene's meshes and materials, copied to the graphics card and ready to draw.
pub(crate) struct Meshes {
    /// `None` when the scene has nothing to draw.
    buffers: Option<(Buffer, Buffer)>,
    /// Solid parts, then cut-out ones, each kind grouped by material; then
    /// see-through ones.
    draws: Vec<Draw>,
    /// How many of `draws` come before the first see-through one.
    see_through_from: usize,
    materials: Materials,
    lighting: Lighting,
}

impl Meshes {
    /// Copies the meshes of `scene`, its materials and the textures they
    /// use to the graphics card.
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
                         `positions`, `normals` and any of `tangents`, `uv0`, `uv1` and \
                         `colors` are the same length.",
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
                let [low, high] = [part.bounds.min, part.bounds.max];
                draws.push(Draw {
                    world: instance.transform,
                    normal: normal_matrix(&instance.transform),
                    material: Materials::number(scene, part.material),
                    surface: Surface::of(material),
                    middle: moved(
                        &instance.transform,
                        [0, 1, 2].map(|axis| (low[axis] + high[axis]) * 0.5),
                    ),
                    first_index,
                    index_count,
                    first_vertex,
                    mirrored: instance.mirrored,
                    double_sided: material.double_sided,
                });
            }
        }
        // Parts that share a material follow one another, so the material
        // is handed to the graphics card once for all of them.
        draws.sort_by_key(|draw| (draw.surface, draw.material));
        let see_through_from = draws
            .iter()
            .position(|draw| draw.surface == Surface::SeeThrough)
            .unwrap_or(draws.len());

        if scene
            .lights
            .iter()
            .any(|light| light.kind != LightKind::Sun)
        {
            log::warn!(
                "The level has lamps (point or spot lights), which are not drawn yet; only \
                 its first sun lights it. Nothing needs changing in the file."
            );
        }

        let buffers = if draws.is_empty() {
            None
        } else {
            Some((
                Buffer::holding(gpu, &vertices, vk::BufferUsageFlags::VERTEX_BUFFER, doing)?,
                Buffer::holding(gpu, &indices, vk::BufferUsageFlags::INDEX_BUFFER, doing)?,
            ))
        };
        Ok(Self {
            buffers,
            draws,
            see_through_from,
            materials: Materials::new(gpu, scene)?,
            lighting: Lighting::of(scene),
        })
    }
}

/// Where `transform` puts `point`.
fn moved(transform: &Mat4, point: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|row| {
        transform[0][row] * point[0]
            + transform[1][row] * point[1]
            + transform[2][row] * point[2]
            + transform[3][row]
    })
}

/// A number from -1 to 1 as a signed 16-bit one.
fn snorm16(value: f32) -> i16 {
    // The clamp keeps the product inside what an `i16` holds; not-a-number becomes 0.
    #[allow(clippy::cast_possible_truncation)]
    let packed = (value.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    packed
}

/// A number from -1 to 1 as a signed byte.
fn snorm8(value: f32) -> i8 {
    #[allow(clippy::cast_possible_truncation)]
    let packed = (value.clamp(-1.0, 1.0) * 127.0).round() as i8;
    packed
}

/// A number from 0 to 1 as a byte.
fn unorm8(value: f32) -> u8 {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let packed = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    packed
}

/// A direction folded flat onto a square (octahedral encoding), which takes
/// two numbers instead of three and loses nothing a screen can show.
///
/// The shader's `unfolded` is the way back. A direction with no length, or
/// one that is not a number, comes out as straight up the Z axis.
fn folded([x, y, z]: [f32; 3]) -> [i16; 2] {
    let sum = x.abs() + y.abs() + z.abs();
    if !sum.is_finite() || sum == 0.0 {
        return [0, 0];
    }
    let (x, y) = (x / sum, y / sum);
    if z >= 0.0 {
        [snorm16(x), snorm16(y)]
    } else {
        // The lower half is folded outwards over the corners.
        [
            snorm16((1.0 - y.abs()).copysign(x)),
            snorm16((1.0 - x.abs()).copysign(y)),
        ]
    }
}

/// Adds one part's points and triangles to the ends of `vertices` and
/// `indices`, and says where they went. Adds nothing and returns `None` if
/// the part is empty or does not hold together.
fn append(part: &MeshPart, vertices: &mut Vec<u8>, indices: &mut Vec<u8>) -> Placed {
    fn fits<T>(list: Option<&Vec<T>>, points: usize) -> bool {
        list.is_none_or(|list| list.len() == points)
    }
    let points = part.positions.len();
    let whole = part.indices.len() - part.indices.len() % 3;
    if whole == 0
        || part.normals.len() != points
        || !fits(part.tangents.as_ref(), points)
        || !fits(part.uv0.as_ref(), points)
        || !fits(part.uv1.as_ref(), points)
        || !fits(part.colors.as_ref(), points)
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
        let each =
            |list: &Option<Vec<[f32; 2]>>| list.as_ref().map_or([0.0; 2], |list| list[number]);
        for value in position {
            vertices.extend_from_slice(&value.to_ne_bytes());
        }
        for value in folded(*normal) {
            vertices.extend_from_slice(&value.to_ne_bytes());
        }
        // No tangent is all zeroes, which the shader takes as "do not bend".
        let tangent = part
            .tangents
            .as_ref()
            .map_or([0.0, 0.0, 0.0, 1.0], |tangents| tangents[number]);
        vertices.extend(tangent.map(|value| snorm8(value).to_ne_bytes()[0]));
        // A point with no colour of its own is white, so the material's shows.
        let colour = part
            .colors
            .as_ref()
            .map_or([1.0; 4], |colours| colours[number]);
        vertices.extend(colour.map(unorm8));
        for value in each(&part.uv0).iter().chain(&each(&part.uv1)) {
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
    /// What a material's set holds; each scene's materials make sets like it.
    material_layout: vk::DescriptorSetLayout,
    layout: vk::PipelineLayout,
    /// One for each [`Surface`], in order.
    pipelines: [vk::Pipeline; 3],
    /// Draws the editor's ground grid, with the same layout and view.
    grid: vk::Pipeline,
    pool: vk::DescriptorPool,
    /// The [`View`] of each frame that can be in progress at once, with the
    /// set that points the shader at it.
    views: Vec<(Buffer, vk::DescriptorSet)>,
    /// The see-through parts of the frame being recorded, farthest first:
    /// how far each is from the eye, squared, and which it is. Kept between
    /// frames so that putting them in order needs no new memory.
    order: RefCell<Vec<(f32, usize)>>,
}

impl MeshPipeline {
    /// Builds the pipeline, for up to `frames` frames in progress at once.
    pub(crate) fn new(gpu: &Arc<Gpu>, frames: usize) -> Result<Self, RenderError> {
        let doing = "preparing the shaders";
        let device = gpu.device();
        let mut mesh = Self {
            gpu: Arc::clone(gpu),
            set_layout: vk::DescriptorSetLayout::null(),
            material_layout: vk::DescriptorSetLayout::null(),
            layout: vk::PipelineLayout::null(),
            pipelines: [vk::Pipeline::null(); 3],
            grid: vk::Pipeline::null(),
            pool: vk::DescriptorPool::null(),
            views: Vec::with_capacity(frames),
            order: RefCell::new(Vec::new()),
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
            .stage_flags(vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT)];
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
            mesh.material_layout = material::set_layout(gpu)?;
            let set_layouts = [mesh.set_layout];
            let both = [mesh.set_layout, mesh.material_layout];
            mesh.layout = device
                .create_pipeline_layout(
                    &vk::PipelineLayoutCreateInfo::default()
                        .set_layouts(&both)
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

        let bindings = [vk::VertexInputBindingDescription::default()
            .binding(0)
            .stride(VERTEX_BYTES as u32)
            .input_rate(vk::VertexInputRate::VERTEX)];
        // Position, folded normal, tangent, colour and the two UV maps.
        let formats = [
            (vk::Format::R32G32B32_SFLOAT, 0),
            (vk::Format::R16G16_SNORM, 12),
            (vk::Format::R8G8B8A8_SNORM, 16),
            (vk::Format::R8G8B8A8_UNORM, 20),
            (vk::Format::R32G32_SFLOAT, 24),
            (vk::Format::R32G32_SFLOAT, 32),
        ];
        let mut attributes = [vk::VertexInputAttributeDescription::default(); 6];
        for ((location, attribute), (format, offset)) in (0_u32..).zip(&mut attributes).zip(formats)
        {
            *attribute = attribute.location(location).format(format).offset(offset);
        }
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
        let dynamic_states = [
            vk::DynamicState::VIEWPORT,
            vk::DynamicState::SCISSOR,
            vk::DynamicState::CULL_MODE,
            vk::DynamicState::FRONT_FACE,
        ];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let formats = [SceneTarget::FORMAT, SceneTarget::MOTION_FORMAT];

        let mut failed = None;
        for (surface, pipeline) in Surface::ALL.into_iter().zip(&mut mesh.pipelines) {
            let see_through = surface == Surface::SeeThrough;
            let stages = [
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::VERTEX)
                    .module(module)
                    .name(c"vertex"),
                vk::PipelineShaderStageCreateInfo::default()
                    .stage(vk::ShaderStageFlags::FRAGMENT)
                    .module(module)
                    .name(match surface {
                        Surface::Solid => c"fragment",
                        Surface::CutOut => c"fragment_cutout",
                        Surface::SeeThrough => c"fragment_blend",
                    }),
            ];
            // Depth is 1 at the camera and 0 far away, so the greater depth
            // wins. A see-through surface is hidden by what is in front of
            // it but hides nothing itself.
            let depth = vk::PipelineDepthStencilStateCreateInfo::default()
                .depth_test_enable(true)
                .depth_write_enable(!see_through)
                .depth_compare_op(vk::CompareOp::GREATER);
            // The colour, then how far the pixel moved. A see-through
            // surface is mixed over the colour by its alpha and leaves the
            // movement of what is behind it alone.
            let colour = vk::PipelineColorBlendAttachmentState::default()
                .color_write_mask(vk::ColorComponentFlags::RGBA);
            let attachments = if see_through {
                [
                    colour
                        .blend_enable(true)
                        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
                        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                        .color_blend_op(vk::BlendOp::ADD)
                        .src_alpha_blend_factor(vk::BlendFactor::ZERO)
                        .dst_alpha_blend_factor(vk::BlendFactor::ONE)
                        .alpha_blend_op(vk::BlendOp::ADD),
                    vk::PipelineColorBlendAttachmentState::default(),
                ]
            } else {
                [colour; 2]
            };
            let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
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
            // SAFETY: `info` and everything it points to are alive for the
            // call; the shader module holds both entry points named in
            // `stages`.
            let created = unsafe {
                device.create_graphics_pipelines(
                    vk::PipelineCache::null(),
                    std::slice::from_ref(&info),
                    None,
                )
            };
            match created {
                Ok(pipelines) if !pipelines.is_empty() => *pipeline = pipelines[0],
                other => {
                    failed =
                        Some(other.map_or_else(|(_, code)| code, |_| vk::Result::ERROR_UNKNOWN));
                    break;
                }
            }
        }
        // SAFETY: the module is only needed while pipelines are being created.
        unsafe { device.destroy_shader_module(module, None) };
        if let Some(code) = failed {
            return Err(RenderError::driver(doing)(code));
        }
        mesh.grid = grid::pipeline(gpu, layout)?;
        Ok(mesh)
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
        let lighting = meshes.map_or_else(Lighting::default, |meshes| meshes.lighting);
        // SAFETY: the buffer was made writable and `VIEW_BYTES` long, and the
        // caller guarantees the commands that read it last have finished.
        unsafe { view_buffer.write(&view.bytes(&lighting)) }?;
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
            if let Some(
                meshes @ Meshes {
                    buffers: Some((vertices, indices)),
                    draws,
                    ..
                },
            ) = meshes
            {
                device.cmd_bind_vertex_buffers(commands, 0, &[vertices.handle], &[0]);
                device.cmd_bind_index_buffer(commands, indices.handle, 0, vk::IndexType::UINT32);
                let (solid, see_through) = draws.split_at(meshes.see_through_from.min(draws.len()));
                let mut last = None;
                for draw in solid {
                    self.draw(commands, meshes, draw, last);
                    last = Some(draw);
                }
                // Farthest first, so that each is mixed over what is behind it.
                if let Ok(mut order) = self.order.try_borrow_mut() {
                    order.clear();
                    order.extend(see_through.iter().enumerate().map(|(number, draw)| {
                        let away = [0, 1, 2].map(|axis| draw.middle[axis] - view.eye[axis]);
                        (away.iter().map(|part| part * part).sum::<f32>(), number)
                    }));
                    order.sort_unstable_by(|one, other| other.0.total_cmp(&one.0));
                    for draw in order
                        .iter()
                        .filter_map(|(_, number)| see_through.get(*number))
                    {
                        self.draw(commands, meshes, draw, last);
                        last = Some(draw);
                    }
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

impl MeshPipeline {
    /// Records drawing one mesh part, changing only what differs from the
    /// part drawn before it.
    ///
    /// # Safety
    ///
    /// As for [`record`](Self::record), inside its render pass, with the
    /// buffers of `meshes` bound; `draw` and `last` must be parts of `meshes`.
    unsafe fn draw(
        &self,
        commands: vk::CommandBuffer,
        meshes: &Meshes,
        draw: &Draw,
        last: Option<&Draw>,
    ) {
        let device = self.gpu.device();
        let changed =
            |differs: fn(&Draw, &Draw) -> bool| last.is_none_or(|last| differs(last, draw));
        // SAFETY: the caller guarantees the command buffer is recording
        // inside the mesh pass and that `draw` names points, triangles and a
        // material that `meshes` holds on this device.
        unsafe {
            if changed(|last, draw| last.surface != draw.surface) {
                let pipeline = self.pipelines[draw.surface as usize];
                device.cmd_bind_pipeline(commands, vk::PipelineBindPoint::GRAPHICS, pipeline);
            }
            if changed(|last, draw| last.material != draw.material) {
                device.cmd_bind_descriptor_sets(
                    commands,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.layout,
                    1,
                    &[meshes.materials.set(draw.material)],
                    &[],
                );
            }
            // Triangles are listed anticlockwise seen from the front; a
            // mirrored mesh is the other way round.
            if changed(|last, draw| last.mirrored != draw.mirrored) {
                device.cmd_set_front_face(
                    commands,
                    if draw.mirrored {
                        vk::FrontFace::CLOCKWISE
                    } else {
                        vk::FrontFace::COUNTER_CLOCKWISE
                    },
                );
            }
            if changed(|last, draw| last.double_sided != draw.double_sided) {
                device.cmd_set_cull_mode(
                    commands,
                    if draw.double_sided {
                        vk::CullModeFlags::NONE
                    } else {
                        vk::CullModeFlags::BACK
                    },
                );
            }
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
}

/// The shader's `Draw` for one mesh part.
fn draw_bytes(draw: &Draw) -> [u8; DRAW_BYTES] {
    // A mirrored mesh has its normal map's sideways direction turned around.
    let handedness = [if draw.mirrored { -1.0 } else { 1.0 }];
    let numbers = draw
        .world
        .iter()
        .chain(&draw.normal)
        .flatten()
        .chain(&handedness);
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
            for pipeline in self.pipelines {
                device.destroy_pipeline(pipeline, None);
            }
            device.destroy_pipeline(self.grid, None);
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_pipeline_layout(self.layout, None);
            device.destroy_descriptor_set_layout(self.set_layout, None);
            device.destroy_descriptor_set_layout(self.material_layout, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use fossps_assets::Light;

    use super::*;

    fn triangle() -> MeshPart {
        MeshPart {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..MeshPart::default()
        }
    }

    /// What the shader's `unfolded` makes of a folded direction.
    fn unfolded([x, y]: [i16; 2]) -> [f32; 3] {
        let (x, y) = (f32::from(x) / 32767.0, f32::from(y) / 32767.0);
        let z = 1.0 - x.abs() - y.abs();
        let under = (-z).max(0.0);
        let (x, y) = (x - under.copysign(x), y - under.copysign(y));
        let length = (x * x + y * y + z * z).sqrt();
        [x / length, y / length, z / length]
    }

    #[test]
    fn a_folded_direction_unfolds_to_itself() {
        let mut directions = vec![
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        // And a spread of others, all round.
        for step in 0..200_u16 {
            let turn = f32::from(step) * 2.399_963;
            let height = 1.0 - f32::from(step) / 99.5;
            let across = (1.0 - height * height).max(0.0).sqrt();
            directions.push([across * turn.cos(), height, across * turn.sin()]);
        }
        for direction in directions {
            let back = unfolded(folded(direction));
            let apart: f32 = (0..3)
                .map(|axis| (back[axis] - direction[axis]).abs())
                .sum();
            assert!(apart < 2e-4, "{direction:?} came back as {back:?}");
        }
        // A direction that is no direction is still one the shader can use.
        assert_eq!(unfolded(folded([0.0; 3])), [0.0, 0.0, 1.0]);
        assert_eq!(unfolded(folded([f32::NAN; 3])), [0.0, 0.0, 1.0]);
    }

    #[test]
    fn parts_are_laid_end_to_end() {
        let (mut vertices, mut indices) = (Vec::new(), Vec::new());
        assert_eq!(
            append(&triangle(), &mut vertices, &mut indices),
            Some((0, 3, 0))
        );
        let textured = MeshPart {
            tangents: Some(vec![[1.0, 0.0, 0.0, -1.0]; 3]),
            uv0: Some(vec![[0.25, 0.5]; 3]),
            uv1: Some(vec![[2.0, 3.0]; 3]),
            colors: Some(vec![[1.0, 0.5, 0.0, 2.0]; 3]),
            ..triangle()
        };
        assert_eq!(
            append(&textured, &mut vertices, &mut indices),
            Some((3, 3, 3))
        );
        assert_eq!(vertices.len(), 6 * VERTEX_BYTES);
        assert_eq!(indices.len(), 6 * 4);
        // A point with no tangent, colour or UV of its own: no bend, white,
        // so the material's colour shows, and the texture's corner.
        assert_eq!(vertices[16..20], [0, 0, 0, 127]);
        assert_eq!(vertices[20..24], [255; 4]);
        assert!(vertices[24..40].iter().all(|&byte| byte == 0));
        // One with all of them, in the order the pipeline reads them.
        let second = &vertices[3 * VERTEX_BYTES..];
        assert_eq!(
            second[12..16],
            [0; 4],
            "a normal along +Z folds to the middle"
        );
        assert_eq!(second[16..20], [127, 0, 0, 129]);
        // A colour brighter than white is white.
        assert_eq!(second[20..24], [255, 128, 0, 255]);
        assert_eq!(second[24..28], 0.25_f32.to_ne_bytes());
        assert_eq!(second[36..40], 3.0_f32.to_ne_bytes());
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
        let tangents = MeshPart {
            tangents: Some(vec![[1.0; 4]; 4]),
            ..triangle()
        };
        let uv = MeshPart {
            uv1: Some(vec![[1.0; 2]; 1]),
            ..triangle()
        };
        let empty = MeshPart::default();
        for part in [beyond, short, colours, tangents, uv, empty] {
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

    fn number(bytes: &[u8], at: usize) -> f32 {
        f32::from_ne_bytes([0, 1, 2, 3].map(|byte| bytes[at * 4 + byte]))
    }

    #[test]
    fn a_draw_fills_the_shader_block_exactly() {
        let mut world = [[0.0; 4]; 4];
        for (axis, column) in world.iter_mut().enumerate() {
            column[axis] = 1.0;
        }
        let mut draw = Draw {
            world,
            normal: normal_matrix(&world),
            material: 0,
            surface: Surface::Solid,
            middle: [0.0; 3],
            first_index: 0,
            index_count: 3,
            first_vertex: 0,
            mirrored: false,
            double_sided: false,
        };
        let bytes = draw_bytes(&draw);
        assert_eq!(number(&bytes, 0), 1.0, "the matrix comes first");
        assert_eq!(number(&bytes, 16), 1.0, "then the surface directions");
        assert_eq!(number(&bytes, 28), 1.0, "then which way round it is");
        draw.mirrored = true;
        assert_eq!(number(&draw_bytes(&draw), 28), -1.0);
    }

    #[test]
    fn a_view_fills_the_shader_block_exactly() {
        let mut matrix = [[0.0; 4]; 4];
        matrix[3][3] = 7.0;
        let view = View {
            clip: matrix,
            steady: matrix,
            before: matrix,
            eye: [1.0, 2.0, 3.0],
            sharper: 1.5,
        };
        let lighting = Lighting {
            towards_sun: [0.0, 1.0, 0.0],
            sun: [4.0, 5.0, 6.0],
            sky: [0.1, 0.2, 0.3],
            ground: [0.4, 0.5, 0.6],
        };
        let bytes = view.bytes(&lighting);
        assert_eq!(
            number(&bytes, 47),
            7.0,
            "the last number of the third matrix"
        );
        // The eye, and textures read one and a half sizes larger.
        assert_eq!(
            [48, 49, 50, 51].map(|at| number(&bytes, at)),
            [1.0, 2.0, 3.0, -1.5]
        );
        assert_eq!([52, 53, 54].map(|at| number(&bytes, at)), [0.0, 1.0, 0.0]);
        assert_eq!([56, 57, 58].map(|at| number(&bytes, at)), [4.0, 5.0, 6.0]);
        assert_eq!([60, 61, 62].map(|at| number(&bytes, at)), [0.1, 0.2, 0.3]);
        assert_eq!([64, 65, 66].map(|at| number(&bytes, at)), [0.4, 0.5, 0.6]);
    }

    #[test]
    fn the_usual_light_shows_a_surface_facing_up_in_its_own_colour() {
        let usual = Lighting::default();
        let [x, y, z] = usual.towards_sun;
        assert!((x * x + y * y + z * z - 1.0).abs() < 1e-4);
        // The sun's light on the ground, as a matte white surface shows it,
        // and the sky's: together exactly white.
        for (sun, sky) in usual.sun.iter().zip(usual.sky) {
            let shown = sun * y / std::f32::consts::PI + sky;
            assert!((shown - 1.0).abs() < 1e-5, "{shown}");
        }
    }

    #[test]
    fn a_level_with_a_sun_is_lit_by_it() {
        let light = |kind, direction| Light {
            name: None,
            kind,
            color: [1.0, 0.5, 0.25],
            intensity: 2.0,
            range: None,
            position: [0.0; 3],
            direction,
        };
        let mut scene = Scene::default();
        assert_eq!(Lighting::of(&scene), Lighting::default());

        // A lamp is not a sun, and a sun that points nowhere cannot be used.
        scene.lights.push(light(LightKind::Point, [0.0, -1.0, 0.0]));
        scene.lights.push(light(LightKind::Sun, [0.0; 3]));
        assert_eq!(Lighting::of(&scene), Lighting::default());

        // The first sun that can be: it shines down, so it is found up.
        scene.lights.push(light(LightKind::Sun, [0.0, -2.0, 0.0]));
        scene.lights.push(light(LightKind::Sun, [1.0, 0.0, 0.0]));
        let lighting = Lighting::of(&scene);
        assert_eq!(lighting.towards_sun, [0.0, 1.0, 0.0]);
        assert_eq!(lighting.sun, [2.0, 1.0, 0.5]);
        assert_eq!(lighting.sky, Lighting::default().sky);
    }

    #[test]
    fn solid_parts_are_drawn_before_cut_out_and_see_through_ones() {
        assert!(Surface::Solid < Surface::CutOut && Surface::CutOut < Surface::SeeThrough);
        let with = |alpha_mode| Material {
            alpha_mode,
            ..Material::default()
        };
        assert_eq!(Surface::of(&with(AlphaMode::Opaque)), Surface::Solid);
        assert_eq!(
            Surface::of(&with(AlphaMode::Mask { cutoff: 0.5 })),
            Surface::CutOut
        );
        assert_eq!(Surface::of(&with(AlphaMode::Blend)), Surface::SeeThrough);
        // The pipelines are kept in the same order.
        for (number, surface) in Surface::ALL.into_iter().enumerate() {
            assert_eq!(surface as usize, number);
        }
    }

    #[test]
    fn a_point_is_moved_by_a_transform() {
        let transform = [
            [2.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [10.0, 20.0, 30.0, 1.0],
        ];
        assert_eq!(moved(&transform, [1.0, 2.0, 3.0]), [12.0, 22.0, 33.0]);
    }
}
