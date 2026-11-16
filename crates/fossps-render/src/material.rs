//! A scene's materials as the mesh shader reads them: a block of numbers and
//! five textures each.

use std::sync::Arc;

use ash::vk;
use fossps_assets::{AlphaMode, Material, Sampler, Scene, TextureRef};

use crate::buffer::Buffer;
use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::texture::Textures;

/// The textures a material can have, in the order the shader numbers them:
/// base colour, metallic-roughness, normal, occlusion and emissive.
const SLOTS: usize = 5;

/// The size in bytes of the shader's `Material`: four rows of four numbers,
/// then two rows for each texture's placing.
pub(crate) const MATERIAL_BYTES: usize = 64 + SLOTS * 32;

const DOING: &str = "copying the level's materials to the graphics card";

/// A material's texture slots in the shader's order, each with whether it
/// holds colours rather than plain numbers.
fn slots(material: &Material) -> [(Option<&TextureRef>, bool); SLOTS] {
    [
        (material.base_color_texture.as_ref(), true),
        (material.metallic_roughness_texture.as_ref(), false),
        (material.normal_texture.as_ref(), false),
        (material.occlusion_texture.as_ref(), false),
        (material.emissive_texture.as_ref(), true),
    ]
}

/// The two rows that take a mesh's UV to where a texture is read, and in
/// the first row's last number which of the mesh's UV maps is used.
///
/// A texture with a transform is scaled, then turned, then moved, as glTF's
/// `KHR_texture_transform` says.
fn placing(texture: Option<&TextureRef>) -> [[f32; 4]; 2] {
    let Some(texture) = texture else {
        return [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]];
    };
    let set = f32::from(texture.uv_set.min(1));
    match texture.transform {
        None => [[1.0, 0.0, 0.0, set], [0.0, 1.0, 0.0, 0.0]],
        Some(transform) => {
            let (sin, cos) = transform.rotation.sin_cos();
            let [wide, tall] = transform.scale;
            let [right, down] = transform.offset;
            [
                [cos * wide, sin * tall, right, set],
                [-sin * wide, cos * tall, down, 0.0],
            ]
        }
    }
}

/// The shader's `Material` for `material`. `present` says which of its
/// textures are really there to be read, in the shader's order.
pub(crate) fn material_bytes(material: &Material, present: [bool; SLOTS]) -> [u8; MATERIAL_BYTES] {
    let cutoff = match material.alpha_mode {
        AlphaMode::Mask { cutoff } => cutoff,
        AlphaMode::Opaque | AlphaMode::Blend => 0.0,
    };
    let glow = material
        .emissive
        .map(|light| light * material.emissive_strength);
    let numbers = [
        material.base_color,
        [glow[0], glow[1], glow[2], cutoff],
        [
            material.metallic,
            material.roughness,
            material.normal_scale,
            material.occlusion_strength,
        ],
    ];
    let mut flags = 0_u32;
    for (slot, there) in present.iter().enumerate() {
        flags |= u32::from(*there) << slot;
    }
    let placings = slots(material).map(|(texture, _)| placing(texture));

    let mut bytes = [0_u8; MATERIAL_BYTES];
    let (rows, _) = bytes.as_chunks_mut::<4>();
    for (slot, number) in rows.iter_mut().zip(numbers.iter().flatten()) {
        *slot = number.to_ne_bytes();
    }
    rows[12] = flags.to_ne_bytes();
    for (slot, number) in rows[16..]
        .iter_mut()
        .zip(placings.iter().flatten().flatten())
    {
        *slot = number.to_ne_bytes();
    }
    bytes
}

/// Makes the layout of a material's descriptor set: its block of numbers,
/// its five textures, and a sampler for each.
///
/// The mesh pipeline and every scene's materials make one each; Vulkan
/// treats two layouts made the same way as the same.
pub(crate) fn set_layout(gpu: &Gpu) -> Result<vk::DescriptorSetLayout, RenderError> {
    let mut bindings = [vk::DescriptorSetLayoutBinding::default(); 1 + 2 * SLOTS];
    for (number, binding) in (0_u32..).zip(&mut bindings) {
        *binding = binding
            .binding(number)
            .descriptor_type(match number as usize {
                0 => vk::DescriptorType::UNIFORM_BUFFER,
                slot if slot <= SLOTS => vk::DescriptorType::SAMPLED_IMAGE,
                _ => vk::DescriptorType::SAMPLER,
            })
            .descriptor_count(1)
            .stage_flags(vk::ShaderStageFlags::FRAGMENT);
    }
    // SAFETY: the device is alive and the bindings live until the call returns.
    unsafe {
        gpu.device().create_descriptor_set_layout(
            &vk::DescriptorSetLayoutCreateInfo::default().bindings(&bindings),
            None,
        )
    }
    .map_err(RenderError::driver(DOING))
}

/// A scene's materials on the graphics card.
pub(crate) struct Materials {
    gpu: Arc<Gpu>,
    layout: vk::DescriptorSetLayout,
    pool: vk::DescriptorPool,
    /// One set for each of the scene's materials, then one for the matte
    /// white that a part with no material is drawn in.
    sets: Vec<vk::DescriptorSet>,
    // Kept for as long as the sets point at them.
    _numbers: Option<Buffer>,
    textures: Textures,
}

impl Materials {
    /// Copies the materials of `scene`, and the textures they use, to the
    /// graphics card.
    pub(crate) fn new(gpu: &Arc<Gpu>, scene: &Scene) -> Result<Self, RenderError> {
        let device = gpu.device();
        let fallback = Material::default();
        let count = scene.materials.len() + 1;
        let all = || scene.materials.iter().chain(std::iter::once(&fallback));

        // Dropping `materials` cleans up whatever was created; destroying a
        // null handle is allowed.
        let mut materials = Self {
            gpu: Arc::clone(gpu),
            layout: vk::DescriptorSetLayout::null(),
            pool: vk::DescriptorPool::null(),
            sets: Vec::with_capacity(count),
            _numbers: None,
            textures: Textures::new(gpu)?,
        };
        let textures = &mut materials.textures;

        // What each slot of each material reads: a picture and how.
        let plain = textures.sampler(&Sampler::default())?;
        let white = textures.white();
        let mut reads = Vec::with_capacity(count);
        for material in all() {
            let mut read = [(white, plain); SLOTS];
            let mut present = [false; SLOTS];
            for (slot, (wanted, colour)) in slots(material).into_iter().enumerate() {
                let Some(texture) =
                    wanted.and_then(|wanted| scene.textures.get(wanted.texture as usize))
                else {
                    continue;
                };
                if let Some(view) = textures.view(scene, texture.image, colour)? {
                    read[slot] = (view, textures.sampler(&texture.sampler)?);
                    present[slot] = true;
                }
            }
            reads.push((read, present));
        }

        // Every material's numbers in one buffer, each starting where the
        // graphics card allows a block to start.
        // SAFETY: the instance and the physical device are alive as long as `gpu` is.
        let step = unsafe {
            gpu.instance()
                .get_physical_device_properties(gpu.physical_device())
        }
        .limits
        .min_uniform_buffer_offset_alignment
        .max(1);
        let stride = (MATERIAL_BYTES as u64).next_multiple_of(step);
        let mut numbers = vec![0_u8; usize::try_from(stride).unwrap_or(MATERIAL_BYTES) * count];
        for ((material, (_, present)), block) in all()
            .zip(&reads)
            .zip(numbers.chunks_mut(usize::try_from(stride).unwrap_or(MATERIAL_BYTES)))
        {
            block[..MATERIAL_BYTES].copy_from_slice(&material_bytes(material, *present));
        }
        let buffer = Buffer::holding(gpu, &numbers, vk::BufferUsageFlags::UNIFORM_BUFFER, DOING)?;

        materials.layout = set_layout(gpu)?;
        let sets = u32::try_from(count).map_err(|_| RenderError::OutOfMemory { doing: DOING })?;
        let per_set = SLOTS as u32;
        let sizes = [
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::UNIFORM_BUFFER)
                .descriptor_count(sets),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(sets.saturating_mul(per_set)),
            vk::DescriptorPoolSize::default()
                .ty(vk::DescriptorType::SAMPLER)
                .descriptor_count(sets.saturating_mul(per_set)),
        ];
        let layouts = vec![materials.layout; count];
        // SAFETY: the device is alive; the pool is stored straight away and
        // is made large enough for the sets taken from it. Each set is
        // pointed at this scene's buffer, pictures and samplers, which
        // `materials` keeps alive for as long as it keeps the sets.
        unsafe {
            materials.pool = device
                .create_descriptor_pool(
                    &vk::DescriptorPoolCreateInfo::default()
                        .max_sets(sets)
                        .pool_sizes(&sizes),
                    None,
                )
                .map_err(RenderError::driver(DOING))?;
            materials.sets = device
                .allocate_descriptor_sets(
                    &vk::DescriptorSetAllocateInfo::default()
                        .descriptor_pool(materials.pool)
                        .set_layouts(&layouts),
                )
                .map_err(RenderError::driver(DOING))?;
            let mut offset = 0;
            for (set, (read, _)) in materials.sets.iter().zip(&reads) {
                let block = [vk::DescriptorBufferInfo::default()
                    .buffer(buffer.handle)
                    .offset(offset)
                    .range(MATERIAL_BYTES as u64)];
                offset += stride;
                let pictures = read.map(|(view, _)| {
                    vk::DescriptorImageInfo::default()
                        .image_view(view)
                        .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                });
                let samplers =
                    read.map(|(_, sampler)| vk::DescriptorImageInfo::default().sampler(sampler));
                let writes = [
                    vk::WriteDescriptorSet::default()
                        .dst_set(*set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER)
                        .buffer_info(&block),
                    // Bindings follow one another, so one write fills all five.
                    vk::WriteDescriptorSet::default()
                        .dst_set(*set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&pictures),
                    vk::WriteDescriptorSet::default()
                        .dst_set(*set)
                        .dst_binding(1 + per_set)
                        .descriptor_type(vk::DescriptorType::SAMPLER)
                        .image_info(&samplers),
                ];
                device.update_descriptor_sets(&writes, &[]);
            }
        }
        materials._numbers = Some(buffer);
        Ok(materials)
    }

    /// The set of the scene's material number `material`, or of matte white
    /// if the scene has no such material.
    pub(crate) fn set(&self, material: u32) -> vk::DescriptorSet {
        self.sets
            .get(material as usize)
            .or(self.sets.last())
            .copied()
            .unwrap_or_default()
    }

    /// The number [`set`](Self::set) knows `material` by: its own, or the
    /// one after the scene's last for matte white.
    pub(crate) fn number(scene: &Scene, material: u32) -> u32 {
        material.min(u32::try_from(scene.materials.len()).unwrap_or(u32::MAX))
    }
}

impl Drop for Materials {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping a scene's materials, so they are not in use. Destroying
        // the pool frees its sets.
        unsafe {
            let device = self.gpu.device();
            device.destroy_descriptor_pool(self.pool, None);
            device.destroy_descriptor_set_layout(self.layout, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use fossps_assets::TextureTransform;

    use super::*;

    fn number(bytes: &[u8], at: usize) -> f32 {
        f32::from_ne_bytes([0, 1, 2, 3].map(|byte| bytes[at * 4 + byte]))
    }

    #[test]
    fn a_material_fills_the_shader_block() {
        let material = Material {
            base_color: [0.1, 0.2, 0.3, 0.4],
            emissive: [1.0, 0.5, 0.0],
            emissive_strength: 4.0,
            metallic: 0.25,
            roughness: 0.75,
            normal_scale: 2.0,
            occlusion_strength: 0.5,
            alpha_mode: AlphaMode::Mask { cutoff: 0.3 },
            ..Material::default()
        };
        let bytes = material_bytes(&material, [true, false, true, false, false]);
        assert_eq!(
            [0, 1, 2, 3].map(|at| number(&bytes, at)),
            material.base_color
        );
        // The glow is already as bright as it will be; then the cut-off.
        assert_eq!(
            [4, 5, 6, 7].map(|at| number(&bytes, at)),
            [4.0, 2.0, 0.0, 0.3]
        );
        assert_eq!(
            [8, 9, 10, 11].map(|at| number(&bytes, at)),
            [0.25, 0.75, 2.0, 0.5]
        );
        // The base colour and normal textures are there: bits 0 and 2.
        assert_eq!(bytes[48..52], 0b101_u32.to_ne_bytes());
        // A texture with nothing said about it is read where the mesh says.
        assert_eq!(
            (16..24).map(|at| number(&bytes, at)).collect::<Vec<_>>(),
            [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]
        );
    }

    #[test]
    fn a_texture_is_scaled_then_turned_then_moved() {
        let texture = TextureRef {
            texture: 0,
            uv_set: 1,
            transform: Some(TextureTransform {
                offset: [0.5, 0.25],
                rotation: std::f32::consts::FRAC_PI_2,
                scale: [2.0, 4.0],
            }),
        };
        let [u, v] = placing(Some(&texture));
        let at = |uv: [f32; 2]| {
            [
                u[0] * uv[0] + u[1] * uv[1] + u[2],
                v[0] * uv[0] + v[1] * uv[1] + v[2],
            ]
        };
        let near =
            |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5;
        assert!(near(at([0.0, 0.0]), [0.5, 0.25]), "the corner only moves");
        // One step along u is doubled, then turned a quarter turn to point
        // up the picture (v runs down it).
        assert!(
            near(at([1.0, 0.0]), [0.5, 0.25 - 2.0]),
            "{:?}",
            at([1.0, 0.0])
        );
        assert!(
            near(at([0.0, 1.0]), [0.5 + 4.0, 0.25]),
            "{:?}",
            at([0.0, 1.0])
        );
        assert_eq!(u[3], 1.0, "the mesh's second UV map is used");
    }
}
