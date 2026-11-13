//! Reads a glTF file into a [`Scene`].

use std::collections::VecDeque;
use std::fs;
use std::path::Path;

use gltf::Gltf;

use crate::error::AssetError;
use crate::files::Origin;
use crate::material::{extras, material, texture};
use crate::math::{IDENTITY, combine, forward, mirrors, position};
use crate::mesh::{PartContext, part};
use crate::scene::{Instance, Light, LightKind, Marker, Material, Mesh, Scene, Transform};

/// The glTF extensions a file may need and still be read.
const READABLE_EXTENSIONS: &[&str] = &[
    "KHR_lights_punctual",
    "KHR_materials_emissive_strength",
    "KHR_texture_transform",
];

/// What to read from a level or model file.
///
/// ```
/// use fossps_assets::LoadOptions;
///
/// // A dedicated server needs shapes, not pictures.
/// let options = LoadOptions::shapes_only();
/// assert!(!options.textures);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadOptions {
    /// Decode the file's pictures. Off, materials keep their colours and
    /// numbers but have no textures.
    pub textures: bool,
    /// Work out tangents for normal-mapped meshes that were exported without them.
    pub tangents: bool,
}

impl Default for LoadOptions {
    /// Everything a renderer needs.
    fn default() -> Self {
        Self {
            textures: true,
            tangents: true,
        }
    }
}

impl LoadOptions {
    /// Only what collision needs: no pictures are decoded and no tangents worked out.
    #[must_use]
    pub fn shapes_only() -> Self {
        Self {
            textures: false,
            tangents: false,
        }
    }
}

impl Scene {
    /// Reads the level or model at `path`, a `.glb` or `.gltf` file.
    ///
    /// # Errors
    ///
    /// Fails if the file or a file it points to cannot be read, is not glTF
    /// 2.0, or uses a feature FOSSPS does not read. The message says what to
    /// change in the 3D tool.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, AssetError> {
        Self::load_with(path, LoadOptions::default())
    }

    /// Reads the level or model at `path`, choosing what to read.
    ///
    /// # Errors
    ///
    /// As [`load`](Self::load).
    pub fn load_with(path: impl AsRef<Path>, options: LoadOptions) -> Result<Self, AssetError> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| AssetError::Read {
            path: path.to_owned(),
            source,
        })?;
        let name = path.display().to_string();
        // A bare file name has an empty parent, which means the current folder.
        Self::from_bytes(&name, &bytes, path.parent(), options)
    }

    /// Reads a level or model that is already in memory.
    ///
    /// `name` is what messages call the file. `folder` is where to look for
    /// the separate files a `.gltf` points to; with `None`, only files that
    /// hold everything themselves (`.glb`) can be read.
    ///
    /// # Errors
    ///
    /// As [`load`](Self::load).
    pub fn from_bytes(
        name: &str,
        bytes: &[u8],
        folder: Option<&Path>,
        options: LoadOptions,
    ) -> Result<Self, AssetError> {
        let origin = Origin { file: name, folder };
        let Gltf { document, blob } = parse(name, bytes)?;
        // Everything is numbered with a `u32`, which no real file comes near.
        let most = [
            document.meshes().len(),
            document.materials().len(),
            document.textures().len(),
            document.images().len(),
        ];
        if most.iter().any(|&length| length >= u32::MAX as usize) {
            return Err(AssetError::damaged(
                name,
                "it lists more items than fit in one scene",
            ));
        }
        let buffers = read_buffers(&origin, &document, blob)?;
        check_ranges(name, &document, &buffers)?;

        let mut scene = Self::default();
        let textures = options.textures && cfg!(feature = "textures");
        if textures {
            read_images(&origin, &document, &buffers, &mut scene)?;
            scene.textures = document.textures().map(|t| texture(&t)).collect();
        }
        scene.materials = document
            .materials()
            .map(|source| material(&source, textures, &mut scene.warnings))
            .collect();

        let tangents = options.tangents && textures;
        read_meshes(name, &document, &buffers, tangents, &mut scene)?;
        place_nodes(name, &document, &mut scene)?;

        if document.animations().len() > 0 {
            scene.warnings.push(
                "The file has animations, which are not played yet; everything is shown in its rest pose."
                    .to_owned(),
            );
        }
        for warning in &scene.warnings {
            log::warn!("{name}: {warning}");
        }
        Ok(scene)
    }
}

/// Parses the file, turning the reader's failures into ones an artist can act on.
fn parse(file: &str, bytes: &[u8]) -> Result<Gltf, AssetError> {
    let unreadable_extension = |gltf: &Gltf| {
        gltf.document
            .extensions_required()
            .find(|extension| !READABLE_EXTENSIONS.contains(extension))
            .map(|extension| AssetError::UnsupportedExtension {
                file: file.to_owned(),
                extension: extension.to_owned(),
            })
    };
    match Gltf::from_slice(bytes) {
        Ok(gltf) => match unreadable_extension(&gltf) {
            Some(error) => Err(error),
            None => Ok(gltf),
        },
        Err(error) => {
            // A needed extension is the likeliest reason a real file is
            // refused, and "export without Draco" helps more than a rule number.
            if let Ok(loose) = Gltf::from_slice_without_validation(bytes)
                && let Some(error) = unreadable_extension(&loose)
            {
                return Err(error);
            }
            Err(match error {
                gltf::Error::Validation(problems) => AssetError::Invalid {
                    file: file.to_owned(),
                    problems: problems
                        .into_iter()
                        .map(|(path, problem)| format!("{path}: {problem}"))
                        .collect(),
                },
                source => AssetError::NotGltf {
                    file: file.to_owned(),
                    source,
                },
            })
        }
    }
}

/// Fetches the data of every buffer, from the file itself or beside it.
fn read_buffers(
    origin: &Origin<'_>,
    document: &gltf::Document,
    mut blob: Option<Vec<u8>>,
) -> Result<Vec<Vec<u8>>, AssetError> {
    document
        .buffers()
        .map(|buffer| {
            let data = match buffer.source() {
                gltf::buffer::Source::Bin => blob.take().ok_or_else(|| {
                    AssetError::damaged(origin.file, "its binary part is missing")
                })?,
                gltf::buffer::Source::Uri(uri) => origin.read(uri, false)?,
            };
            if data.len() < buffer.length() {
                return Err(AssetError::damaged(
                    origin.file,
                    format!(
                        "data block {} should hold {} bytes but only has {}",
                        buffer.index(),
                        buffer.length(),
                        data.len()
                    ),
                ));
            }
            Ok(data)
        })
        .collect()
}

/// Makes sure everything the file says about its data fits inside the data.
///
/// The glTF reader does not check this itself, so a cut-short or hostile
/// file is refused here instead of being read past its end.
fn check_ranges(
    file: &str,
    document: &gltf::Document,
    buffers: &[Vec<u8>],
) -> Result<(), AssetError> {
    let outside = |what: &str, index: usize| {
        AssetError::damaged(
            file,
            format!("{what} {index} reaches past the end of its data"),
        )
    };
    for view in document.views() {
        let available = buffers.get(view.buffer().index()).map_or(0, Vec::len);
        if !fits(view.offset(), 1, view.length(), view.length(), available) {
            return Err(outside("data view", view.index()));
        }
    }
    let total: usize = buffers.iter().map(Vec::len).sum();
    for accessor in document.accessors() {
        let size = accessor.size();
        let index = accessor.index();
        let count = accessor.count();
        if let Some(view) = accessor.view() {
            let stride = view.stride().unwrap_or(size);
            if !fits(accessor.offset(), count, stride, size, view.length()) {
                return Err(outside("data list", index));
            }
        } else if count > total {
            // A list with no data of its own is all zeros, so nothing above
            // limits its length; without this a tiny file could ask for a
            // list that fills the computer's memory.
            return Err(outside("data list", index));
        }
        if let Some(sparse) = accessor.sparse() {
            let indices = sparse.indices();
            let index_size = match indices.index_type() {
                gltf::accessor::sparse::IndexType::U8 => 1,
                gltf::accessor::sparse::IndexType::U16 => 2,
                gltf::accessor::sparse::IndexType::U32 => 4,
            };
            let index_view = indices.view();
            let values = sparse.values();
            let value_view = values.view();
            let replaced = sparse.count();
            if replaced == 0
                || replaced > count
                || !fits(
                    indices.offset(),
                    replaced,
                    index_view.stride().unwrap_or(index_size),
                    index_size,
                    index_view.length(),
                )
                || !fits(
                    values.offset(),
                    replaced,
                    value_view.stride().unwrap_or(size),
                    size,
                    value_view.length(),
                )
            {
                return Err(outside("data list", index));
            }
        }
    }
    Ok(())
}

/// A position in one of the scene's lists. `from_bytes` has made sure they all fit.
pub(crate) fn number(index: usize) -> u32 {
    u32::try_from(index).unwrap_or(u32::MAX)
}

/// Whether `count` items of `size` bytes, `stride` bytes apart and starting
/// at `offset`, fit in `available` bytes.
fn fits(offset: usize, count: usize, stride: usize, size: usize, available: usize) -> bool {
    let Some(last) = count.checked_sub(1) else {
        return offset <= available;
    };
    stride >= size
        && last
            .checked_mul(stride)
            .and_then(|span| span.checked_add(size))
            .and_then(|span| span.checked_add(offset))
            .is_some_and(|end| end <= available)
}

/// Decodes every picture in the file.
#[cfg(feature = "textures")]
fn read_images(
    origin: &Origin<'_>,
    document: &gltf::Document,
    buffers: &[Vec<u8>],
    scene: &mut Scene,
) -> Result<(), AssetError> {
    for image in document.images() {
        let fetched;
        let (bytes, label): (&[u8], _) = match image.source() {
            gltf::image::Source::View { view, .. } => {
                // `check_ranges` has made sure the view fits its buffer.
                let bytes = buffers
                    .get(view.buffer().index())
                    .and_then(|data| data.get(view.offset()..view.offset() + view.length()))
                    .unwrap_or_default();
                (bytes, None)
            }
            gltf::image::Source::Uri { uri, .. } => {
                fetched = origin.read(uri, true)?;
                // A picture stored inside the file has no file name worth showing.
                (
                    &fetched,
                    (!uri.starts_with("data:")).then(|| uri.to_owned()),
                )
            }
        };
        let name = image.name().map(str::to_owned).or(label);
        let shown = name
            .clone()
            .unwrap_or_else(|| format!("number {}", image.index() + 1));
        let decoded =
            crate::material::decode(name, bytes).map_err(|problem| AssetError::BadImage {
                file: origin.file.to_owned(),
                image: shown,
                problem,
            })?;
        scene.images.push(decoded);
    }
    Ok(())
}

/// Without the `textures` feature there is nothing to decode with.
#[cfg(not(feature = "textures"))]
#[allow(clippy::unnecessary_wraps)]
fn read_images(
    _origin: &Origin<'_>,
    _document: &gltf::Document,
    _buffers: &[Vec<u8>],
    _scene: &mut Scene,
) -> Result<(), AssetError> {
    Ok(())
}

/// Reads every mesh. `tangents` says whether to work out missing ones.
fn read_meshes(
    file: &str,
    document: &gltf::Document,
    buffers: &[Vec<u8>],
    tangents: bool,
    scene: &mut Scene,
) -> Result<(), AssetError> {
    // Shapes with no material share one, added the first time it is needed.
    let mut fallback: Option<u32> = None;
    for source in document.meshes() {
        let name = source.name().map(str::to_owned);
        let shown = name
            .clone()
            .unwrap_or_else(|| format!("number {}", source.index() + 1));
        let mut parts = Vec::new();
        for primitive in source.primitives() {
            let gltf_material = primitive.material();
            let material = match gltf_material.index() {
                Some(index) => number(index),
                None => *fallback.get_or_insert_with(|| {
                    scene.materials.push(Material::default());
                    number(scene.materials.len() - 1)
                }),
            };
            let tangent_uv = gltf_material
                .normal_texture()
                .filter(|_| tangents)
                .map(|normal| u8::from(normal.tex_coord() == 1));
            let context = PartContext {
                file,
                mesh: &shown,
                buffers,
                material,
                tangent_uv,
            };
            if let Some(part) = part(&context, &primitive, &mut scene.warnings)? {
                parts.push(part);
            }
        }
        scene.meshes.push(Mesh { name, parts });
    }
    Ok(())
}

/// Walks the scene's objects and records where each mesh, light and marker stands.
fn place_nodes(file: &str, document: &gltf::Document, scene: &mut Scene) -> Result<(), AssetError> {
    let Some(root) = document
        .default_scene()
        .or_else(|| document.scenes().next())
    else {
        return Err(AssetError::NoScene {
            file: file.to_owned(),
        });
    };
    // glTF asks for a tree; a file that reuses an object could otherwise loop for ever.
    let mut visited = vec![false; document.nodes().len()];
    let mut skinned = false;
    let mut queue: VecDeque<(gltf::Node<'_>, Transform)> =
        root.nodes().map(|node| (node, IDENTITY)).collect();
    while let Some((node, parent)) = queue.pop_front() {
        match visited.get_mut(node.index()) {
            Some(seen) if !*seen => *seen = true,
            _ => {
                return Err(AssetError::damaged(
                    file,
                    format!(
                        "object {} is placed under more than one parent",
                        node.name().map_or_else(
                            || format!("number {}", node.index() + 1),
                            |name| format!("\"{name}\"")
                        )
                    ),
                ));
            }
        }
        let transform = combine(&parent, &node.transform().matrix());
        let name = node.name().map(str::to_owned);
        let extras = extras(node.extras());
        skinned |= node.skin().is_some();

        if let Some(light) = node.light() {
            scene.lights.push(Light {
                name: light.name().or(node.name()).map(str::to_owned),
                kind: match light.kind() {
                    gltf::khr_lights_punctual::Kind::Directional => LightKind::Sun,
                    gltf::khr_lights_punctual::Kind::Point => LightKind::Point,
                    gltf::khr_lights_punctual::Kind::Spot {
                        inner_cone_angle,
                        outer_cone_angle,
                    } => LightKind::Spot {
                        inner_angle: inner_cone_angle,
                        outer_angle: outer_cone_angle,
                    },
                },
                color: light.color(),
                intensity: light.intensity(),
                range: light.range(),
                position: position(&transform),
                direction: forward(&transform),
            });
        }
        let mesh = node
            .mesh()
            .and_then(|mesh| u32::try_from(mesh.index()).ok());
        if let Some(mesh) = mesh {
            scene.instances.push(Instance {
                mesh,
                transform,
                mirrored: mirrors(&transform),
                name,
                extras,
            });
        } else if node.light().is_none() && (name.is_some() || extras.is_some()) {
            scene.markers.push(Marker {
                name,
                transform,
                extras,
            });
        }
        queue.extend(node.children().map(|child| (child, transform)));
    }
    if skinned {
        scene.warnings.push(
            "The file has skinned (rigged) meshes, which are not animated yet; they are shown in their rest pose."
                .to_owned(),
        );
    }
    Ok(())
}
