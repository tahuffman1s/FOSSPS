//! What a level or model file holds once it is read.

/// Where something stands: a 4 by 4 matrix stored column by column.
///
/// `transform[3]` is the position. This is the layout glTF and graphics cards use.
pub type Transform = [[f32; 4]; 4];

/// Everything read from one level or model file.
///
/// The lists refer to each other by position: an [`Instance`] names a mesh
/// by its index in `meshes`, a [`MeshPart`] names a material, and so on.
/// Every such index is valid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    /// The shapes, each stored once however many times it is placed.
    pub meshes: Vec<Mesh>,
    /// Where each shape stands. A shape placed twice has two entries.
    pub instances: Vec<Instance>,
    /// How surfaces look.
    pub materials: Vec<Material>,
    /// Pictures paired with how to sample them. Empty when textures were not loaded.
    pub textures: Vec<Texture>,
    /// The decoded pictures. Empty when textures were not loaded.
    pub images: Vec<Image>,
    /// The lights placed in the file.
    pub lights: Vec<Light>,
    /// Named places with no shape, such as spawn points.
    pub markers: Vec<Marker>,
    /// Things in the file that were skipped or changed, worded for its author.
    pub warnings: Vec<String>,
}

/// One shape, made of a part for each material it uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    /// The name given in the 3D tool.
    pub name: Option<String>,
    /// The pieces of the shape. Empty if the file only held points or lines.
    pub parts: Vec<MeshPart>,
}

/// A piece of a mesh drawn with one material.
///
/// Each list has one entry per corner point; `positions` and `normals` are
/// always filled and the optional lists, when present, are the same length.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshPart {
    /// Where each corner point is, in metres.
    pub positions: Vec<[f32; 3]>,
    /// Which way the surface faces at each point. Worked out from the
    /// triangles when the file has none.
    pub normals: Vec<[f32; 3]>,
    /// The direction textures run along the surface, needed for normal maps.
    /// The fourth number is 1 or -1. Worked out when the material has a
    /// normal map and the file has none.
    pub tangents: Option<Vec<[f32; 4]>>,
    /// Where each point is on its textures.
    pub uv0: Option<Vec<[f32; 2]>>,
    /// A second texture placement, usually for baked lighting.
    pub uv1: Option<Vec<[f32; 2]>>,
    /// A colour painted on each point (red, green, blue, alpha; linear).
    pub colors: Option<Vec<[f32; 4]>>,
    /// The triangles: every three numbers are three points, listed
    /// anticlockwise when seen from the front.
    pub indices: Vec<u32>,
    /// Which of the scene's materials this part is drawn with.
    pub material: u32,
    /// The box that just holds every point.
    pub bounds: Bounds,
}

impl MeshPart {
    /// The triangles as corner positions, which is all collision needs.
    pub fn triangles(&self) -> impl Iterator<Item = [[f32; 3]; 3]> + '_ {
        self.indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter_map(|corners| {
                let corner = |i: usize| self.positions.get(corners[i] as usize).copied();
                Some([corner(0)?, corner(1)?, corner(2)?])
            })
    }
}

/// A box lined up with the axes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Bounds {
    /// The corner with the smallest x, y and z.
    pub min: [f32; 3],
    /// The corner with the largest x, y and z.
    pub max: [f32; 3],
}

/// One placement of a mesh in the scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Which of the scene's meshes is placed here.
    pub mesh: u32,
    /// Where it stands, and its rotation and size.
    pub transform: Transform,
    /// The placement flips the shape (a negative scale), so its triangles
    /// run clockwise instead of anticlockwise.
    pub mirrored: bool,
    /// The name given in the 3D tool.
    pub name: Option<String>,
    /// The custom properties set in the 3D tool, as JSON text.
    pub extras: Option<String>,
}

/// A named place with no shape, such as a spawn point or an objective.
#[derive(Debug, Clone, PartialEq)]
pub struct Marker {
    /// The name given in the 3D tool.
    pub name: Option<String>,
    /// Where it stands and which way it faces.
    pub transform: Transform,
    /// The custom properties set in the 3D tool, as JSON text.
    pub extras: Option<String>,
}

/// A light placed in the file.
#[derive(Debug, Clone, PartialEq)]
pub struct Light {
    /// The name given in the 3D tool.
    pub name: Option<String>,
    /// What sort of light it is.
    pub kind: LightKind,
    /// Its colour (red, green, blue; linear).
    pub color: [f32; 3],
    /// How bright it is: lux for a sun, candela for the others.
    pub intensity: f32,
    /// How far it reaches in metres, if the file sets a limit.
    pub range: Option<f32>,
    /// Where it is. Means nothing for a sun.
    pub position: [f32; 3],
    /// Which way it shines. Means nothing for a point light.
    pub direction: [f32; 3],
}

/// The sorts of light.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LightKind {
    /// Shines the same way everywhere, like the sun.
    Sun,
    /// Shines in every direction from one point, like a bulb.
    Point,
    /// Shines in a cone, like a torch.
    Spot {
        /// The angle from the middle of the cone to where it starts to fade, in radians.
        inner_angle: f32,
        /// The angle from the middle of the cone to where it ends, in radians.
        outer_angle: f32,
    },
}

/// How a surface looks: glTF's metallic-roughness material.
///
/// The base colour and emissive textures hold sRGB colours; the others hold
/// plain numbers and must not be colour-converted.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    /// The name given in the 3D tool.
    pub name: Option<String>,
    /// The surface colour (red, green, blue, alpha; linear), multiplied with the texture.
    pub base_color: [f32; 4],
    /// The surface colour picture.
    pub base_color_texture: Option<TextureRef>,
    /// 0 for plastic, stone and wood; 1 for bare metal.
    pub metallic: f32,
    /// 0 for a mirror finish, 1 for fully matte.
    pub roughness: f32,
    /// Roughness in the green channel and metallic in the blue, multiplied
    /// with the two numbers above.
    pub metallic_roughness_texture: Option<TextureRef>,
    /// Fine surface detail as a normal map.
    pub normal_texture: Option<TextureRef>,
    /// How strongly the normal map bends the surface.
    pub normal_scale: f32,
    /// Baked shadowing in crevices, in the red channel.
    pub occlusion_texture: Option<TextureRef>,
    /// How strongly the occlusion texture darkens, from 0 to 1.
    pub occlusion_strength: f32,
    /// The light the surface gives off (red, green, blue; linear),
    /// multiplied with the texture and with `emissive_strength`.
    pub emissive: [f32; 3],
    /// The picture of the light the surface gives off.
    pub emissive_texture: Option<TextureRef>,
    /// Brightens `emissive` beyond 1 for surfaces that glow.
    pub emissive_strength: f32,
    /// What the alpha of the base colour does.
    pub alpha_mode: AlphaMode,
    /// The surface is drawn from behind as well.
    pub double_sided: bool,
    /// The custom properties set in the 3D tool, as JSON text.
    pub extras: Option<String>,
}

impl Default for Material {
    /// Matte white: what a shape with no material is drawn with.
    fn default() -> Self {
        Self {
            name: None,
            base_color: [1.0; 4],
            base_color_texture: None,
            metallic: 0.0,
            roughness: 1.0,
            metallic_roughness_texture: None,
            normal_texture: None,
            normal_scale: 1.0,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive: [0.0; 3],
            emissive_texture: None,
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            double_sided: false,
            extras: None,
        }
    }
}

/// What the alpha of a material's base colour does.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode {
    /// Nothing: the surface is solid.
    Opaque,
    /// Cuts holes: anything with alpha below `cutoff` is not drawn.
    Mask {
        /// The alpha below which the surface is not drawn.
        cutoff: f32,
    },
    /// Makes the surface see-through.
    Blend,
}

/// A material's use of one texture.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureRef {
    /// Which of the scene's textures is used.
    pub texture: u32,
    /// Which placement to use: 0 for a part's `uv0`, 1 for `uv1`.
    pub uv_set: u8,
    /// Slides, turns or repeats the texture, if the file asks for it.
    pub transform: Option<TextureTransform>,
}

/// A change made to texture coordinates before the texture is read:
/// scaled, then turned, then moved.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureTransform {
    /// How far to move.
    pub offset: [f32; 2],
    /// How far to turn anticlockwise, in radians.
    pub rotation: f32,
    /// How many times to repeat.
    pub scale: [f32; 2],
}

/// A picture together with how to sample it.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    /// The name given in the 3D tool.
    pub name: Option<String>,
    /// Which of the scene's images is used.
    pub image: u32,
    /// How the picture repeats and is smoothed.
    pub sampler: Sampler,
}

/// A decoded picture.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    /// The name given in the 3D tool, or its file name.
    pub name: Option<String>,
    /// How many pixels wide it is.
    pub width: u32,
    /// How many pixels tall it is.
    pub height: u32,
    /// Four bytes per pixel (red, green, blue, alpha), row by row from the top.
    pub pixels: Vec<u8>,
}

/// How a texture repeats and is smoothed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sampler {
    /// What happens past the left and right edges.
    pub wrap_u: Wrap,
    /// What happens past the top and bottom edges.
    pub wrap_v: Wrap,
    /// How the texture looks when shown larger than it is.
    pub magnify: Filter,
    /// How the texture looks when shown smaller than it is.
    pub minify: Filter,
    /// How smaller copies of the texture are mixed, or `None` to not use them.
    pub mipmaps: Option<Filter>,
}

impl Default for Sampler {
    /// Repeats, and is smoothed in every way.
    fn default() -> Self {
        Self {
            wrap_u: Wrap::Repeat,
            wrap_v: Wrap::Repeat,
            magnify: Filter::Linear,
            minify: Filter::Linear,
            mipmaps: Some(Filter::Linear),
        }
    }
}

/// What a texture shows past its edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wrap {
    /// The texture again.
    Repeat,
    /// The texture again, flipped each time.
    MirroredRepeat,
    /// The colour at the edge.
    ClampToEdge,
}

/// How neighbouring pixels are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// The nearest pixel is used: sharp and blocky.
    Nearest,
    /// Neighbours are blended: smooth.
    Linear,
}
