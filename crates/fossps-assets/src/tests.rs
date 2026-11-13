//! Whole files read end to end. The files are built here, so the repository
//! holds no binary test data.

use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use base64::Engine as _;

use crate::{AssetError, LightKind, LoadOptions, Scene};

/// A glTF file under construction: its data block and the lists describing it.
#[derive(Default)]
struct Builder {
    bin: Vec<u8>,
    views: Vec<String>,
    accessors: Vec<String>,
}

const FLOAT: u32 = 5126;
const U16: u32 = 5123;

impl Builder {
    /// Adds a data list and returns its accessor number.
    fn list(
        &mut self,
        bytes: &[u8],
        count: usize,
        component: u32,
        kind: &str,
        extra: &str,
    ) -> usize {
        self.views.push(format!(
            r#"{{"buffer":0,"byteOffset":{},"byteLength":{}}}"#,
            self.bin.len(),
            bytes.len()
        ));
        self.bin.extend_from_slice(bytes);
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        self.accessors.push(format!(
            r#"{{"bufferView":{},"componentType":{component},"count":{count},"type":"{kind}"{extra}}}"#,
            self.views.len() - 1
        ));
        self.accessors.len() - 1
    }

    fn floats<const N: usize>(&mut self, kind: &str, items: &[[f32; N]], extra: &str) -> usize {
        let bytes: Vec<u8> = items
            .iter()
            .flatten()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        self.list(&bytes, items.len(), FLOAT, kind, extra)
    }

    fn positions(&mut self, items: &[[f32; 3]]) -> usize {
        self.floats("VEC3", items, r#","min":[-10,-10,-10],"max":[10,10,10]"#)
    }

    fn vec3(&mut self, items: &[[f32; 3]]) -> usize {
        self.floats("VEC3", items, "")
    }

    fn vec2(&mut self, items: &[[f32; 2]]) -> usize {
        self.floats("VEC2", items, "")
    }

    fn indices(&mut self, items: &[u16]) -> usize {
        let bytes: Vec<u8> = items.iter().flat_map(|i| i.to_le_bytes()).collect();
        self.list(&bytes, items.len(), U16, "SCALAR", "")
    }

    /// The file's JSON, with its data block described by `buffer` and the
    /// rest of the file (meshes, nodes and so on) in `rest`.
    fn json(&self, buffer: &str, rest: &str) -> String {
        let mut json = String::from(r#"{"asset":{"version":"2.0"}"#);
        if !self.bin.is_empty() {
            write!(
                json,
                r#","buffers":[{{"byteLength":{}{buffer}}}],"bufferViews":[{}],"accessors":[{}]"#,
                self.bin.len(),
                self.views.join(","),
                self.accessors.join(",")
            )
            .unwrap();
        }
        if !rest.is_empty() {
            write!(json, ",{rest}").unwrap();
        }
        json.push('}');
        json
    }

    /// A `.gltf` holding its data inside itself.
    fn gltf(&self, rest: &str) -> Vec<u8> {
        self.json(
            &format!(
                r#","uri":"{}""#,
                data_uri("application/octet-stream", &self.bin)
            ),
            rest,
        )
        .into_bytes()
    }

    /// A `.gltf` whose data is in the separate file `uri`.
    fn gltf_beside(&self, uri: &str, rest: &str) -> Vec<u8> {
        self.json(&format!(r#","uri":"{uri}""#), rest).into_bytes()
    }

    /// A `.glb`.
    fn glb(&self, rest: &str) -> Vec<u8> {
        glb(&self.json("", rest), &self.bin)
    }
}

fn data_uri(kind: &str, bytes: &[u8]) -> String {
    format!(
        "data:{kind};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
    let mut json = json.as_bytes().to_vec();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let length = 12 + 8 + json.len() + 8 + bin.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&u32::try_from(length).unwrap().to_le_bytes());
    out.extend_from_slice(&u32::try_from(json.len()).unwrap().to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out.extend_from_slice(&u32::try_from(bin.len()).unwrap().to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(bin);
    out
}

const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
const QUAD: [[f32; 3]; 4] = [
    [0.0, 0.0, 0.0],
    [1.0, 0.0, 0.0],
    [1.0, 1.0, 0.0],
    [0.0, 1.0, 0.0],
];

/// One mesh (accessor 0 as positions, accessor 1 as indices) placed once.
const ONE_MESH: &str = r#""meshes":[{"name":"Crate","primitives":[{"attributes":{"POSITION":0},"indices":1}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}],"scene":0"#;

/// A builder holding a triangle's positions and indices.
fn triangle() -> Builder {
    let mut file = Builder::default();
    file.positions(&TRIANGLE);
    file.indices(&[0, 1, 2]);
    file
}

fn read(bytes: &[u8]) -> Result<Scene, AssetError> {
    Scene::from_bytes("level.glb", bytes, None, LoadOptions::default())
}

/// A fresh folder for one test, removed when dropped.
struct Folder(PathBuf);

impl Folder {
    fn new(test: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("fossps-assets-{test}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_triangle_reads_the_same_from_gltf_and_glb() {
    let file = triangle();
    let from_gltf = read(&file.gltf(ONE_MESH)).unwrap();
    let from_glb = read(&file.glb(ONE_MESH)).unwrap();
    assert_eq!(from_gltf, from_glb);

    let scene = from_glb;
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.meshes[0].name.as_deref(), Some("Crate"));
    let part = &scene.meshes[0].parts[0];
    assert_eq!(part.positions, TRIANGLE);
    assert_eq!(part.indices, [0, 1, 2]);
    // No normals in the file: they face out of the triangle.
    assert_eq!(part.normals, [[0.0, 0.0, 1.0]; 3]);
    assert_eq!(part.bounds.min, [0.0, 0.0, 0.0]);
    assert_eq!(part.bounds.max, [1.0, 1.0, 0.0]);
    assert_eq!(part.triangles().collect::<Vec<_>>(), [TRIANGLE]);
    assert_eq!(
        (&part.tangents, &part.uv0, &part.colors),
        (&None, &None, &None)
    );
    // No material in the file: matte white.
    assert_eq!(scene.materials.len(), 1);
    assert_eq!(
        scene.materials[part.material as usize],
        crate::Material::default()
    );
    assert_eq!(scene.instances.len(), 1);
    assert_eq!(scene.instances[0].mesh, 0);
    assert!(!scene.instances[0].mirrored);
}

#[test]
fn separate_files_are_found_beside_the_level() {
    let folder = Folder::new("beside");
    let file = triangle();
    fs::write(folder.0.join("my level.bin"), &file.bin).unwrap();
    let path = folder.0.join("level.gltf");
    fs::write(&path, file.gltf_beside("my%20level.bin", ONE_MESH)).unwrap();
    let scene = Scene::load(&path).unwrap();
    assert_eq!(scene.meshes[0].parts[0].positions, TRIANGLE);

    // The same file with its data missing says which file to put where.
    fs::remove_file(folder.0.join("my level.bin")).unwrap();
    let error = Scene::load(&path).unwrap_err();
    assert!(
        matches!(error, AssetError::MissingFile { texture: false, .. }),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("level.gltf") && message.contains("my%20level.bin"),
        "{message}"
    );
    assert!(message.contains(".glb"), "{message}");
}

#[test]
fn a_level_cannot_read_files_outside_its_folder() {
    let folder = Folder::new("outside");
    let file = triangle();
    for uri in ["../secret.bin", "/etc/passwd", "https://example.com/a.bin"] {
        let bytes = file.gltf_beside(uri, ONE_MESH);
        let error = Scene::from_bytes(
            "level.gltf",
            &bytes,
            Some(&folder.0),
            LoadOptions::default(),
        )
        .unwrap_err();
        assert!(
            matches!(error, AssetError::OutsideFolder { .. }),
            "{uri}: {error}"
        );
    }
    // Read from memory with no folder, a separate file cannot be found at all.
    let error = read(&file.gltf_beside("level.bin", ONE_MESH)).unwrap_err();
    assert!(matches!(error, AssetError::NoFolder { .. }), "{error}");
}

#[test]
fn a_missing_file_says_where_it_looked() {
    let error = Scene::load("no-such-folder/level.glb").unwrap_err();
    assert!(matches!(error, AssetError::Read { .. }));
    assert!(
        error
            .to_string()
            .starts_with("Could not find no-such-folder"),
        "{error}"
    );
}

#[test]
fn shared_points_are_split_when_normals_are_worked_out() {
    let mut file = Builder::default();
    file.positions(&QUAD);
    file.indices(&[0, 1, 2, 0, 2, 3]);
    let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    file.vec2(&uv);
    let scene = read(&file.glb(
        r#""meshes":[{"primitives":[{"attributes":{"POSITION":0,"TEXCOORD_0":2},"indices":1}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]"#,
    ))
    .unwrap();
    let part = &scene.meshes[0].parts[0];
    assert_eq!(part.indices, [0, 1, 2, 3, 4, 5]);
    assert_eq!(
        part.positions,
        [QUAD[0], QUAD[1], QUAD[2], QUAD[0], QUAD[2], QUAD[3]]
    );
    assert_eq!(part.normals, [[0.0, 0.0, 1.0]; 6]);
    // The other lists follow the split.
    assert_eq!(
        part.uv0.as_deref().unwrap(),
        [uv[0], uv[1], uv[2], uv[0], uv[2], uv[3]]
    );
}

#[test]
fn normals_in_the_file_are_kept_and_points_stay_shared() {
    let mut file = Builder::default();
    file.positions(&QUAD);
    file.vec3(&[[0.0, 1.0, 0.0]; 4]);
    // No triangle list, drawn as a fan: the list is made up.
    let scene = read(&file.glb(
        r#""meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1},"mode":6}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]"#,
    ))
    .unwrap();
    let part = &scene.meshes[0].parts[0];
    assert_eq!(part.positions, QUAD);
    assert_eq!(part.normals, [[0.0, 1.0, 0.0]; 4]);
    assert_eq!(part.indices, [0, 1, 2, 0, 2, 3]);
}

#[test]
fn points_and_lines_are_skipped_with_a_warning() {
    let scene = read(&triangle().glb(
        r#""meshes":[{"name":"Wire","primitives":[{"attributes":{"POSITION":0},"indices":1,"mode":1}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]"#,
    ))
    .unwrap();
    assert!(scene.meshes[0].parts.is_empty());
    assert_eq!(scene.warnings.len(), 1);
    assert!(scene.warnings[0].contains("Wire"), "{:?}", scene.warnings);
}

#[test]
fn objects_are_placed_through_their_parents() {
    let scene = read(&triangle().glb(
        r#""meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],
        "nodes":[
            {"name":"Building","translation":[1,2,3],"children":[1,2,3]},
            {"name":"Door","mesh":0,"translation":[0,0,-5],"scale":[-1,1,1]},
            {"name":"Window","mesh":0,"extras":{"breakable":true}},
            {"name":"spawn_red","rotation":[0,1,0,0],"extras":{"team":"red"}},
            {"name":"Lamp","translation":[0,4,0],"rotation":[-0.70710678,0,0,0.70710678],
             "extensions":{"KHR_lights_punctual":{"light":0}}}
        ],
        "extensionsUsed":["KHR_lights_punctual"],
        "extensions":{"KHR_lights_punctual":{"lights":[
            {"type":"spot","color":[1,0.5,0.25],"intensity":800,"range":12,
             "spot":{"innerConeAngle":0.25,"outerConeAngle":0.5}}]}},
        "scenes":[{"nodes":[0,4]}]"#,
    ))
    .unwrap();
    assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);

    // One mesh, placed twice.
    assert_eq!(scene.meshes.len(), 1);
    let door = scene
        .instances
        .iter()
        .find(|i| i.name.as_deref() == Some("Door"))
        .unwrap();
    assert_eq!(door.transform[3], [1.0, 2.0, -2.0, 1.0]);
    assert_eq!(door.transform[0], [-1.0, 0.0, 0.0, 0.0]);
    assert!(door.mirrored);
    let window = scene
        .instances
        .iter()
        .find(|i| i.name.as_deref() == Some("Window"))
        .unwrap();
    assert_eq!(window.transform[3], [1.0, 2.0, 3.0, 1.0]);
    assert!(!window.mirrored);
    assert_eq!(window.extras.as_deref(), Some(r#"{"breakable":true}"#));

    // Objects with no shape are markers; a turn of half a circle faces +Z.
    let names: Vec<_> = scene
        .markers
        .iter()
        .map(|m| m.name.as_deref().unwrap())
        .collect();
    assert_eq!(names, ["Building", "spawn_red"]);
    let spawn = &scene.markers[1];
    assert_eq!(spawn.extras.as_deref(), Some(r#"{"team":"red"}"#));
    assert_eq!(spawn.transform[3], [1.0, 2.0, 3.0, 1.0]);
    assert!((crate::math::forward(&spawn.transform)[2] - 1.0).abs() < 1e-6);

    // The lamp is tipped a quarter turn about X, so it shines straight down.
    assert_eq!(scene.lights.len(), 1);
    let lamp = &scene.lights[0];
    assert_eq!(lamp.name.as_deref(), Some("Lamp"));
    assert_eq!(
        lamp.kind,
        LightKind::Spot {
            inner_angle: 0.25,
            outer_angle: 0.5
        }
    );
    assert_eq!(
        (lamp.color, lamp.intensity, lamp.range),
        ([1.0, 0.5, 0.25], 800.0, Some(12.0))
    );
    assert_eq!(lamp.position, [0.0, 4.0, 0.0]);
    assert!(
        (lamp.direction[1] + 1.0).abs() < 1e-5,
        "{:?}",
        lamp.direction
    );
}

#[test]
fn an_extension_the_file_needs_says_what_to_untick() {
    let file = triangle();
    let needs = |extension: &str| {
        read(&file.glb(&format!(
            r#"{ONE_MESH},"extensionsUsed":["{extension}"],"extensionsRequired":["{extension}"]"#
        )))
    };
    let error = needs("KHR_draco_mesh_compression").unwrap_err();
    assert!(
        matches!(error, AssetError::UnsupportedExtension { .. }),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("level.glb") && message.contains("untick"),
        "{message}"
    );
    for extension in [
        "KHR_mesh_quantization",
        "KHR_texture_basisu",
        "VENDOR_made_up",
    ] {
        let error = needs(extension).unwrap_err();
        assert!(
            matches!(error, AssetError::UnsupportedExtension { .. }),
            "{extension}: {error}"
        );
    }
    // Extensions that are read may be needed.
    needs("KHR_texture_transform").unwrap();
    // One that is used but not needed is simply not used.
    read(&file.glb(&format!(
        r#"{ONE_MESH},"extensionsUsed":["VENDOR_made_up"]"#
    )))
    .unwrap();
}

#[test]
fn damaged_files_are_refused_without_crashing() {
    let file = triangle();
    let whole = file.glb(ONE_MESH);

    // Cut short at every length: never a panic, never a scene made of garbage.
    for length in 0..whole.len() {
        assert!(read(&whole[..length]).is_err(), "cut to {length} bytes");
    }
    assert!(matches!(
        read(b"not a model").unwrap_err(),
        AssetError::NotGltf { .. }
    ));
    assert!(
        read(b"not a model")
            .unwrap_err()
            .to_string()
            .contains("Blender")
    );

    // The file promises more data than it holds.
    let mut short = triangle();
    short.bin.truncate(8);
    let json = file.json("", ONE_MESH);
    let error = read(&glb(&json, &short.bin)).unwrap_err();
    assert!(matches!(error, AssetError::Damaged { .. }), "{error}");

    // A list that reaches past the end of its data.
    let mut long = triangle();
    long.accessors[0] = long.accessors[0].replace(r#""count":3"#, r#""count":300"#);
    let error = read(&long.glb(ONE_MESH)).unwrap_err();
    assert!(matches!(error, AssetError::Damaged { .. }), "{error}");

    // A triangle that uses a point the mesh does not have.
    let mut wild = Builder::default();
    wild.positions(&TRIANGLE);
    wild.indices(&[0, 1, 7]);
    let error = read(&wild.glb(ONE_MESH)).unwrap_err();
    assert!(error.to_string().contains("Crate"), "{error}");

    // Positions stored as the wrong kind of data.
    let mut wrong = Builder::default();
    wrong.vec2(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
    wrong.indices(&[0, 1, 2]);
    assert!(read(&wrong.glb(ONE_MESH)).is_err());

    // Lists of different lengths.
    let mut uneven = triangle();
    uneven.vec3(&[[0.0, 0.0, 1.0]; 2]);
    let error = read(&uneven.glb(
        r#""meshes":[{"primitives":[{"attributes":{"POSITION":0,"NORMAL":2},"indices":1}]}],"nodes":[{"mesh":0}],"scenes":[{"nodes":[0]}]"#,
    ))
    .unwrap_err();
    assert!(matches!(error, AssetError::Damaged { .. }), "{error}");

    // Lists that patch a few entries ("sparse") get the same checks.
    let sparse = |accessor: &str, stride: &str| {
        let mut file = triangle();
        file.indices(&[1]);
        file.positions(&[[5.0, 5.0, 5.0]]);
        file.views[3] = file.views[3].replace('}', stride);
        file.accessors[0] = accessor.to_owned();
        read(&file.glb(ONE_MESH))
    };
    let patch = |count: u32| {
        format!(
            r#""sparse":{{"count":{count},"indices":{{"bufferView":2,"componentType":5123}},"values":{{"bufferView":3}}}}"#
        )
    };
    let patched = |base: &str, count: u32| {
        format!(
            r#"{{{base}"componentType":5126,"type":"VEC3","min":[0,0,0],"max":[5,5,5],{}}}"#,
            patch(count)
        )
    };
    let scene = sparse(&patched(r#""bufferView":0,"count":3,"#, 1), "}").unwrap();
    assert_eq!(scene.meshes[0].parts[0].positions[1], [5.0, 5.0, 5.0]);
    // Patch entries packed closer together than their size.
    assert!(
        sparse(
            &patched(r#""bufferView":0,"count":3,"#, 1),
            r#","byteStride":4}"#
        )
        .is_err()
    );
    // No patches, or more patches than entries.
    assert!(sparse(&patched(r#""bufferView":0,"count":3,"#, 0), "}").is_err());
    assert!(sparse(&patched(r#""bufferView":0,"count":3,"#, 4), "}").is_err());
    // A list of zeros far longer than the file itself.
    assert!(sparse(&patched(r#""count":4000000000,"#, 1), "}").is_err());

    // An object that is its own child would never finish.
    assert!(
        read(&file.glb(
            r#""meshes":[{"primitives":[{"attributes":{"POSITION":0}}]}],"nodes":[{"mesh":0,"children":[0]}],"scenes":[{"nodes":[0]}]"#,
        ))
        .is_err()
    );
}

#[test]
fn a_file_with_nothing_placed_says_so() {
    let error = read(&Builder::default().gltf("")).unwrap_err();
    assert!(matches!(error, AssetError::NoScene { .. }), "{error}");
}

/// A material with every setting changed from its default, on a quad with
/// normals and a UV map but no tangents. `{image}` is the picture's address.
fn textured(image: &str) -> Vec<u8> {
    let mut file = Builder::default();
    file.positions(&QUAD);
    file.indices(&[0, 1, 2, 0, 2, 3]);
    file.vec3(&[[0.0, 0.0, 1.0]; 4]);
    file.vec2(&[[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]);
    file.glb(&format!(
        r#""meshes":[{{"primitives":[{{"attributes":{{"POSITION":0,"NORMAL":2,"TEXCOORD_0":3}},"indices":1,"material":0}}]}}],
        "nodes":[{{"mesh":0}}],"scenes":[{{"nodes":[0]}}],
        "images":[{{"uri":"{image}"}}],
        "samplers":[{{"magFilter":9728,"minFilter":9986,"wrapS":33071,"wrapT":33648}}],
        "textures":[{{"source":0,"sampler":0}},{{"source":0,"name":"Bumps"}}],
        "materials":[{{"name":"Wall",
            "pbrMetallicRoughness":{{"baseColorFactor":[0.5,0.25,1,1],"metallicFactor":0.25,"roughnessFactor":0.75,
                "baseColorTexture":{{"index":0,"extensions":{{"KHR_texture_transform":{{"offset":[0.5,0],"rotation":1.5,"scale":[2,3]}}}}}}}},
            "normalTexture":{{"index":1,"scale":2,"extensions":{{"KHR_texture_transform":{{"scale":[4,4]}}}}}},
            "emissiveFactor":[1,1,1],
            "extensions":{{"KHR_materials_emissive_strength":{{"emissiveStrength":5}}}},
            "alphaMode":"MASK","alphaCutoff":0.25,"doubleSided":true,"extras":{{"footstep":"metal"}}}}],
        "extensionsUsed":["KHR_texture_transform","KHR_materials_emissive_strength"]"#
    ))
}

#[test]
fn shapes_only_reads_no_pictures() {
    // The picture is not even looked at, so a server never trips over one.
    let scene = Scene::from_bytes(
        "level.glb",
        &textured("missing.png"),
        None,
        LoadOptions::shapes_only(),
    )
    .unwrap();
    assert!(scene.images.is_empty() && scene.textures.is_empty());
    let wall = &scene.materials[0];
    assert_eq!(wall.base_color, [0.5, 0.25, 1.0, 1.0]);
    assert_eq!((wall.base_color_texture, wall.normal_texture), (None, None));
    let part = &scene.meshes[0].parts[0];
    assert_eq!(part.tangents, None);
    assert_eq!(part.positions, QUAD);
}

#[cfg(feature = "textures")]
mod textures {
    use super::*;
    use crate::{AlphaMode, Filter, TextureRef, TextureTransform, Wrap};

    fn png() -> Vec<u8> {
        let picture =
            image::RgbaImage::from_raw(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
        let mut bytes = std::io::Cursor::new(Vec::new());
        picture
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn materials_and_textures_are_read() {
        let scene = read(&textured(&data_uri("image/png", &png()))).unwrap();
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);

        assert_eq!(scene.images.len(), 1);
        let image = &scene.images[0];
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, [255, 0, 0, 255, 0, 0, 255, 128]);

        assert_eq!(scene.textures.len(), 2);
        let sampler = scene.textures[0].sampler;
        assert_eq!(
            (sampler.wrap_u, sampler.wrap_v),
            (Wrap::ClampToEdge, Wrap::MirroredRepeat)
        );
        assert_eq!(sampler.magnify, Filter::Nearest);
        assert_eq!(
            (sampler.minify, sampler.mipmaps),
            (Filter::Nearest, Some(Filter::Linear))
        );
        // No sampler in the file: repeat, and smooth.
        assert_eq!(scene.textures[1].sampler, crate::Sampler::default());
        assert_eq!(scene.textures[1].name.as_deref(), Some("Bumps"));

        let wall = &scene.materials[0];
        assert_eq!(wall.name.as_deref(), Some("Wall"));
        assert_eq!(wall.base_color, [0.5, 0.25, 1.0, 1.0]);
        assert_eq!((wall.metallic, wall.roughness), (0.25, 0.75));
        assert_eq!(
            wall.base_color_texture,
            Some(TextureRef {
                texture: 0,
                uv_set: 0,
                transform: Some(TextureTransform {
                    offset: [0.5, 0.0],
                    rotation: 1.5,
                    scale: [2.0, 3.0],
                }),
            })
        );
        let normal = wall.normal_texture.unwrap();
        assert_eq!(normal.texture, 1);
        assert_eq!(normal.transform.unwrap().scale, [4.0, 4.0]);
        assert_eq!(normal.transform.unwrap().offset, [0.0, 0.0]);
        assert_eq!(wall.normal_scale, 2.0);
        assert_eq!((wall.emissive, wall.emissive_strength), ([1.0; 3], 5.0));
        assert_eq!(wall.alpha_mode, AlphaMode::Mask { cutoff: 0.25 });
        assert!(wall.double_sided);
        assert_eq!(wall.extras.as_deref(), Some(r#"{"footstep":"metal"}"#));
        assert_eq!(
            (wall.occlusion_texture, wall.emissive_texture),
            (None, None)
        );

        // A normal map and no tangents in the file: they are worked out,
        // along the texture's left-to-right direction.
        let part = &scene.meshes[0].parts[0];
        let tangents = part.tangents.as_ref().unwrap();
        assert_eq!(tangents.len(), 4);
        for tangent in tangents {
            assert!((tangent[0] - 1.0).abs() < 1e-5, "{tangent:?}");
        }
        // Normals came from the file, so the points are still shared.
        assert_eq!(part.positions.len(), 4);
    }

    #[test]
    fn tangents_can_be_turned_off() {
        let options = LoadOptions {
            tangents: false,
            ..LoadOptions::default()
        };
        let bytes = textured(&data_uri("image/png", &png()));
        let scene = Scene::from_bytes("level.glb", &bytes, None, options).unwrap();
        assert_eq!(scene.meshes[0].parts[0].tangents, None);
        assert!(scene.materials[0].normal_texture.is_some());
    }

    #[test]
    fn a_texture_beside_the_level_is_found_or_named() {
        let folder = Folder::new("texture");
        fs::create_dir(folder.0.join("textures")).unwrap();
        let bytes = textured("textures/wall.png");
        let load =
            || Scene::from_bytes("level.glb", &bytes, Some(&folder.0), LoadOptions::default());

        let error = load().unwrap_err();
        assert!(
            matches!(error, AssetError::MissingFile { texture: true, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("textures/wall.png"), "{error}");

        fs::write(folder.0.join("textures").join("wall.png"), png()).unwrap();
        let scene = load().unwrap();
        assert_eq!(scene.images[0].name.as_deref(), Some("textures/wall.png"));
    }

    #[test]
    fn a_picture_that_is_not_png_or_jpeg_says_so() {
        for bytes in [
            &b"just some text"[..],
            b"RIFF\x10\0\0\0WEBPVP8 ",
            &png()[..20],
        ] {
            let error = read(&textured(&data_uri("image/png", bytes))).unwrap_err();
            assert!(matches!(error, AssetError::BadImage { .. }), "{error}");
            let message = error.to_string();
            assert!(
                message.contains("PNG and JPEG") && message.contains("level.glb"),
                "{message}"
            );
        }
    }
}
