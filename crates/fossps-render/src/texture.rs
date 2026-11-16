//! A scene's pictures on the graphics card, each with smaller copies of
//! itself for when it is seen from far away, and the ways they are sampled.

use std::sync::Arc;

use ash::vk;
use fossps_assets::{Filter, Image, Sampler, Scene, Wrap};

use crate::buffer::{Buffer, once};
use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::scene::Picture;

/// Pictures that hold colours: the graphics card turns sRGB into light when
/// it reads one, and back when it makes the smaller copies.
const COLOUR_FORMAT: vk::Format = vk::Format::R8G8B8A8_SRGB;
/// Pictures that hold plain numbers, such as normal maps and roughness.
const NUMBER_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// How many samples a texture seen at a slant is read with, where the
/// graphics card can: floors and walls seen along their length are most of
/// what a shooter draws. A setting for it is #141.
const SLANT_SAMPLES: f32 = 16.0;

const DOING: &str = "copying the level's textures to the graphics card";

/// How many sizes a picture is stored in: itself, then half as wide and
/// tall each time, down to a single pixel.
pub(crate) fn sizes(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// The pictures and samplers of one scene.
pub(crate) struct Textures {
    gpu: Arc<Gpu>,
    pictures: Vec<Picture>,
    /// Which of `pictures` holds a scene image, read as colours or as numbers.
    made: Vec<((u32, bool), Option<usize>)>,
    samplers: Vec<(Sampler, vk::Sampler)>,
    /// The widest and tallest picture the graphics card takes.
    largest: u32,
    /// The most samples a sampler may ask for at a slant, or `None` if the
    /// graphics card cannot do it.
    slant: Option<f32>,
    /// Whether the graphics card can make smaller copies of each kind of
    /// picture itself: colours first, then numbers.
    shrinks: [bool; 2],
}

impl Textures {
    /// Gets ready to hold pictures. The first is a single white pixel, for
    /// materials to read where they have no texture of their own.
    pub(crate) fn new(gpu: &Arc<Gpu>) -> Result<Self, RenderError> {
        // SAFETY: the instance and the physical device are alive as long as `gpu` is.
        let (limits, shrinks) = unsafe {
            let instance = gpu.instance();
            let physical = gpu.physical_device();
            let needed = vk::FormatFeatureFlags::BLIT_SRC
                | vk::FormatFeatureFlags::BLIT_DST
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR;
            let shrinks = [COLOUR_FORMAT, NUMBER_FORMAT].map(|format| {
                instance
                    .get_physical_device_format_properties(physical, format)
                    .optimal_tiling_features
                    .contains(needed)
            });
            (
                instance.get_physical_device_properties(physical).limits,
                shrinks,
            )
        };
        let mut textures = Self {
            gpu: Arc::clone(gpu),
            pictures: Vec::new(),
            made: Vec::new(),
            samplers: Vec::new(),
            largest: limits.max_image_dimension2_d,
            slant: gpu
                .anisotropy()
                .then(|| limits.max_sampler_anisotropy.min(SLANT_SAMPLES)),
            shrinks,
        };
        textures.upload(1, 1, &[255; 4], false)?;
        Ok(textures)
    }

    /// The single white pixel.
    pub(crate) fn white(&self) -> vk::ImageView {
        self.pictures
            .first()
            .map_or(vk::ImageView::null(), |picture| picture.view)
    }

    /// The picture `image` of `scene` on the graphics card, copied there the
    /// first time it is asked for. `colour` says whether it holds colours
    /// (base colour, glow) or plain numbers (everything else).
    ///
    /// `None`, with a warning, if the scene has no such picture or it cannot
    /// be used; the material is then drawn without it.
    pub(crate) fn view(
        &mut self,
        scene: &Scene,
        image: u32,
        colour: bool,
    ) -> Result<Option<vk::ImageView>, RenderError> {
        let key = (image, colour);
        let index = if let Some((_, index)) = self.made.iter().find(|(made, _)| *made == key) {
            *index
        } else {
            let index = match scene.images.get(image as usize) {
                Some(picture) if self.usable(picture) => {
                    Some(self.upload(picture.width, picture.height, &picture.pixels, colour)?)
                }
                _ => None,
            };
            self.made.push((key, index));
            index
        };
        Ok(index
            .and_then(|index| self.pictures.get(index))
            .map(|picture| picture.view))
    }

    /// Whether `picture` can be copied to the graphics card; says why not.
    fn usable(&self, picture: &Image) -> bool {
        let name = picture.name.as_deref().unwrap_or("without a name");
        let bytes = u64::from(picture.width) * u64::from(picture.height) * 4;
        if picture.width == 0 || picture.height == 0 || picture.pixels.len() as u64 != bytes {
            log::warn!(
                "The image {name} is not drawn: it says it is {} by {} pixels but holds {} \
                 bytes instead of four for each pixel. If you built the scene in code, check \
                 the image's `width`, `height` and `pixels`; otherwise export the file again.",
                picture.width,
                picture.height,
                picture.pixels.len(),
            );
            return false;
        }
        if picture.width > self.largest || picture.height > self.largest {
            log::warn!(
                "The image {name} is not drawn: it is {} by {} pixels, and this graphics card \
                 takes at most {} each way. Make the image smaller in your 3D tool or an \
                 image editor and export the file again.",
                picture.width,
                picture.height,
                self.largest,
            );
            return false;
        }
        true
    }

    /// Copies one picture to the graphics card, makes its smaller copies,
    /// and says where in `pictures` it is.
    fn upload(
        &mut self,
        width: u32,
        height: u32,
        pixels: &[u8],
        colour: bool,
    ) -> Result<usize, RenderError> {
        let gpu = &self.gpu;
        let device = gpu.device();
        let (format, shrinks) = if colour {
            (COLOUR_FORMAT, self.shrinks[0])
        } else {
            (NUMBER_FORMAT, self.shrinks[1])
        };
        let levels = if shrinks { sizes(width, height) } else { 1 };
        let range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: levels,
            base_array_layer: 0,
            layer_count: 1,
        };
        let staging = Buffer::staging(gpu, pixels.len() as u64, DOING)?;
        staging.fill(pixels, DOING)?;

        // Kept in the list from the start, so that it is freed with the
        // rest whatever happens next.
        self.pictures.push(Picture::null());
        let index = self.pictures.len() - 1;
        let picture = &mut self.pictures[index];
        picture.create(
            gpu,
            vk::Extent2D { width, height },
            format,
            vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_DST
                | vk::ImageUsageFlags::TRANSFER_SRC,
            range,
            DOING,
        )?;
        let image = picture.image;

        let level = |level: u32| vk::ImageSubresourceRange {
            base_mip_level: level,
            level_count: 1,
            ..range
        };
        let layers = |level: u32| vk::ImageSubresourceLayers {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            mip_level: level,
            base_array_layer: 0,
            layer_count: 1,
        };
        let barrier = || {
            vk::ImageMemoryBarrier2::default()
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
        };
        // The far corner of a size: half the one before, and never less than one pixel.
        let corner = |level: u32| vk::Offset3D {
            x: i32::try_from((width >> level).max(1)).unwrap_or(i32::MAX),
            y: i32::try_from((height >> level).max(1)).unwrap_or(i32::MAX),
            z: 1,
        };

        // SAFETY: the image and the staging buffer are alive until `once`
        // has waited for the commands. Each size is written before it is
        // read, in the layout the barrier before it left it in, and the
        // format was checked to allow smooth shrinking when there is more
        // than one size.
        unsafe {
            once(gpu, DOING, |commands| {
                let to_fill = [barrier()
                    .dst_stage_mask(vk::PipelineStageFlags2::ALL_TRANSFER)
                    .dst_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .subresource_range(range)];
                device.cmd_pipeline_barrier2(
                    commands,
                    &vk::DependencyInfo::default().image_memory_barriers(&to_fill),
                );
                device.cmd_copy_buffer_to_image(
                    commands,
                    staging.handle,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[vk::BufferImageCopy::default()
                        .image_subresource(layers(0))
                        .image_extent(vk::Extent3D {
                            width,
                            height,
                            depth: 1,
                        })],
                );
                for smaller in 1..levels {
                    // The size just written is read to make the next.
                    let to_read = [barrier()
                        .src_stage_mask(vk::PipelineStageFlags2::ALL_TRANSFER)
                        .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                        .dst_stage_mask(vk::PipelineStageFlags2::ALL_TRANSFER)
                        .dst_access_mask(vk::AccessFlags2::TRANSFER_READ)
                        .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                        .subresource_range(level(smaller - 1))];
                    device.cmd_pipeline_barrier2(
                        commands,
                        &vk::DependencyInfo::default().image_memory_barriers(&to_read),
                    );
                    let regions = [vk::ImageBlit::default()
                        .src_subresource(layers(smaller - 1))
                        .src_offsets([vk::Offset3D::default(), corner(smaller - 1)])
                        .dst_subresource(layers(smaller))
                        .dst_offsets([vk::Offset3D::default(), corner(smaller)])];
                    device.cmd_blit_image(
                        commands,
                        image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        image,
                        vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                        &regions,
                        vk::Filter::LINEAR,
                    );
                }
                // Every size but the last was read from; the last was only written.
                let read = barrier()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_TRANSFER)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_READ)
                    .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .subresource_range(vk::ImageSubresourceRange {
                        level_count: levels - 1,
                        ..range
                    });
                let written = barrier()
                    .src_stage_mask(vk::PipelineStageFlags2::ALL_TRANSFER)
                    .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::FRAGMENT_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .subresource_range(level(levels - 1));
                let to_show = [written, read];
                // With only one size there is nothing that was read from.
                let to_show = if levels > 1 {
                    &to_show[..]
                } else {
                    &to_show[..1]
                };
                device.cmd_pipeline_barrier2(
                    commands,
                    &vk::DependencyInfo::default().image_memory_barriers(to_show),
                );
            })
        }?;
        Ok(index)
    }

    /// The sampler that reads a texture the way `wanted` says, made the
    /// first time it is asked for.
    pub(crate) fn sampler(&mut self, wanted: &Sampler) -> Result<vk::Sampler, RenderError> {
        if let Some((_, sampler)) = self.samplers.iter().find(|(made, _)| made == wanted) {
            return Ok(*sampler);
        }
        let filter = |filter| match filter {
            Filter::Nearest => vk::Filter::NEAREST,
            Filter::Linear => vk::Filter::LINEAR,
        };
        let wrap = |wrap| match wrap {
            Wrap::Repeat => vk::SamplerAddressMode::REPEAT,
            Wrap::MirroredRepeat => vk::SamplerAddressMode::MIRRORED_REPEAT,
            Wrap::ClampToEdge => vk::SamplerAddressMode::CLAMP_TO_EDGE,
        };
        let mut info = vk::SamplerCreateInfo::default()
            .mag_filter(filter(wanted.magnify))
            .min_filter(filter(wanted.minify))
            .mipmap_mode(match wanted.mipmaps {
                Some(Filter::Linear) => vk::SamplerMipmapMode::LINEAR,
                _ => vk::SamplerMipmapMode::NEAREST,
            })
            .address_mode_u(wrap(wanted.wrap_u))
            .address_mode_v(wrap(wanted.wrap_v))
            .address_mode_w(vk::SamplerAddressMode::REPEAT)
            // A quarter is how Vulkan spells "only ever the full-size picture".
            .max_lod(if wanted.mipmaps.is_some() {
                vk::LOD_CLAMP_NONE
            } else {
                0.25
            });
        // Only a texture that is smoothed in every way can be read at a
        // slant; one that asks for sharp pixels keeps them.
        let smooth = *wanted
            == Sampler {
                magnify: Filter::Linear,
                minify: Filter::Linear,
                mipmaps: Some(Filter::Linear),
                ..*wanted
            };
        if let (Some(samples), true) = (self.slant, smooth) {
            info = info.anisotropy_enable(true).max_anisotropy(samples);
        }
        // SAFETY: the device is alive and the settings are valid: slanted
        // sampling is only asked for where the device has it switched on.
        let sampler = unsafe { self.gpu.device().create_sampler(&info, None) }
            .map_err(RenderError::driver(DOING))?;
        self.samplers.push((*wanted, sampler));
        Ok(sampler)
    }
}

impl Drop for Textures {
    fn drop(&mut self) {
        // SAFETY: the owner waits for the graphics card to go idle before
        // dropping a scene's textures, so nothing is using them, and all of
        // them were made on this device.
        unsafe {
            for (_, sampler) in &self.samplers {
                self.gpu.device().destroy_sampler(*sampler, None);
            }
            for picture in &self.pictures {
                picture.destroy(&self.gpu);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_stored_in_every_size_down_to_one_pixel() {
        assert_eq!(sizes(1, 1), 1);
        assert_eq!(sizes(2, 2), 2);
        assert_eq!(sizes(256, 256), 9);
        // The longer side decides, and a size that is not a power of two rounds down.
        assert_eq!(sizes(1024, 4), 11);
        assert_eq!(sizes(5, 3), 3);
        // Nothing is still one size, not none.
        assert_eq!(sizes(0, 0), 1);
    }
}
