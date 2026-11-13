//! Turns glTF materials, textures and pictures into the engine's.

use gltf::json::Value;
use gltf::texture::{MagFilter, MinFilter, WrappingMode};

use crate::load::number;
use crate::scene::{
    AlphaMode, Filter, Material, Sampler, Texture, TextureRef, TextureTransform, Wrap,
};

/// The glTF extension that slides, turns and repeats a texture.
const TEXTURE_TRANSFORM: &str = "KHR_texture_transform";

/// Reads one material. With `textures` off its texture slots are left empty.
pub(crate) fn material(
    source: &gltf::Material<'_>,
    textures: bool,
    warnings: &mut Vec<String>,
) -> Material {
    let pbr = source.pbr_metallic_roughness();
    let name = source.name().map(str::to_owned);
    let mut slot = Slots {
        enabled: textures,
        material: name.as_deref().unwrap_or("(unnamed)"),
        warnings,
    };
    let normal = source.normal_texture();
    let occlusion = source.occlusion_texture();
    Material {
        base_color: pbr.base_color_factor(),
        base_color_texture: pbr.base_color_texture().and_then(|info| slot.info(&info)),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        metallic_roughness_texture: pbr
            .metallic_roughness_texture()
            .and_then(|info| slot.info(&info)),
        normal_scale: normal
            .as_ref()
            .map_or(1.0, gltf::material::NormalTexture::scale),
        normal_texture: normal.and_then(|normal| {
            slot.make(
                normal.texture().index(),
                normal.tex_coord(),
                normal
                    .extension_value(TEXTURE_TRANSFORM)
                    .and_then(transform),
            )
        }),
        occlusion_strength: occlusion
            .as_ref()
            .map_or(1.0, gltf::material::OcclusionTexture::strength),
        occlusion_texture: occlusion.and_then(|occlusion| {
            slot.make(
                occlusion.texture().index(),
                occlusion.tex_coord(),
                occlusion
                    .extension_value(TEXTURE_TRANSFORM)
                    .and_then(transform),
            )
        }),
        emissive: source.emissive_factor(),
        emissive_texture: source.emissive_texture().and_then(|info| slot.info(&info)),
        emissive_strength: source.emissive_strength().unwrap_or(1.0),
        alpha_mode: match source.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask {
                cutoff: source.alpha_cutoff().unwrap_or(0.5),
            },
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        },
        double_sided: source.double_sided(),
        extras: extras(source.extras()),
        name,
    }
}

/// Builds a material's texture slots.
struct Slots<'a> {
    /// Whether textures are being loaded at all.
    enabled: bool,
    /// The material's name, for warnings.
    material: &'a str,
    warnings: &'a mut Vec<String>,
}

impl Slots<'_> {
    fn info(&mut self, info: &gltf::texture::Info<'_>) -> Option<TextureRef> {
        let transform = info.texture_transform().map(|t| TextureTransform {
            offset: t.offset(),
            rotation: t.rotation(),
            scale: t.scale(),
        });
        self.make(info.texture().index(), info.tex_coord(), transform)
    }

    fn make(
        &mut self,
        texture: usize,
        uv_set: u32,
        transform: Option<TextureTransform>,
    ) -> Option<TextureRef> {
        if !self.enabled {
            return None;
        }
        let uv_set = match u8::try_from(uv_set) {
            Ok(set @ 0..=1) => set,
            _ => {
                self.warnings.push(format!(
                    "Material \"{}\" places a texture with UV map number {}, but only the \
                     first two UV maps of a mesh are read; the first is used instead.",
                    self.material,
                    u64::from(uv_set) + 1
                ));
                0
            }
        };
        Some(TextureRef {
            texture: number(texture),
            uv_set,
            transform,
        })
    }
}

/// Reads a texture transform that the glTF reader leaves as plain JSON.
fn transform(value: &Value) -> Option<TextureTransform> {
    let object = value.as_object()?;
    // Narrowing to f32 is intended: the engine's numbers are f32.
    #[allow(clippy::cast_possible_truncation)]
    let pair = |key: &str, default: f32| -> [f32; 2] {
        let number = |index: usize| {
            object
                .get(key)
                .and_then(|pair| pair.get(index))
                .and_then(Value::as_f64)
                .map_or(default, |n| n as f32)
        };
        [number(0), number(1)]
    };
    #[allow(clippy::cast_possible_truncation)]
    let rotation = object
        .get("rotation")
        .and_then(Value::as_f64)
        .map_or(0.0, |n| n as f32);
    Some(TextureTransform {
        offset: pair("offset", 0.0),
        rotation,
        scale: pair("scale", 1.0),
    })
}

/// Reads one texture: which image it shows and how.
pub(crate) fn texture(source: &gltf::Texture<'_>) -> Texture {
    Texture {
        name: source.name().map(str::to_owned),
        image: number(source.source().index()),
        sampler: sampler(&source.sampler()),
    }
}

fn sampler(source: &gltf::texture::Sampler<'_>) -> Sampler {
    let wrap = |mode| match mode {
        WrappingMode::Repeat => Wrap::Repeat,
        WrappingMode::MirroredRepeat => Wrap::MirroredRepeat,
        WrappingMode::ClampToEdge => Wrap::ClampToEdge,
    };
    let default = Sampler::default();
    let (minify, mipmaps) = match source.min_filter() {
        None => (default.minify, default.mipmaps),
        Some(MinFilter::Nearest) => (Filter::Nearest, None),
        Some(MinFilter::Linear) => (Filter::Linear, None),
        Some(MinFilter::NearestMipmapNearest) => (Filter::Nearest, Some(Filter::Nearest)),
        Some(MinFilter::LinearMipmapNearest) => (Filter::Linear, Some(Filter::Nearest)),
        Some(MinFilter::NearestMipmapLinear) => (Filter::Nearest, Some(Filter::Linear)),
        Some(MinFilter::LinearMipmapLinear) => (Filter::Linear, Some(Filter::Linear)),
    };
    Sampler {
        wrap_u: wrap(source.wrap_s()),
        wrap_v: wrap(source.wrap_t()),
        magnify: match source.mag_filter() {
            None => default.magnify,
            Some(MagFilter::Nearest) => Filter::Nearest,
            Some(MagFilter::Linear) => Filter::Linear,
        },
        minify,
        mipmaps,
    }
}

/// The custom properties on a glTF object, as JSON text.
pub(crate) fn extras(extras: &gltf::json::Extras) -> Option<String> {
    extras.as_ref().map(|raw| raw.get().to_owned())
}

/// Decodes a PNG or JPEG into four bytes per pixel.
#[cfg(feature = "textures")]
pub(crate) fn decode(name: Option<String>, bytes: &[u8]) -> Result<crate::scene::Image, String> {
    let format =
        image::guess_format(bytes).map_err(|_| "it is not a PNG or JPEG file".to_owned())?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg) {
        return Err(format!(
            "it is a {} file",
            format
                .extensions_str()
                .first()
                .copied()
                .unwrap_or("different kind of")
                .to_uppercase()
        ));
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
    // The default limits stop a small file that claims to be an enormous
    // picture from using up the computer's memory.
    reader.limits(image::Limits::default());
    let pixels = reader
        .decode()
        .map_err(|error| error.to_string())?
        .into_rgba8();
    Ok(crate::scene::Image {
        name,
        width: pixels.width(),
        height: pixels.height(),
        pixels: pixels.into_raw(),
    })
}
