//! Builds the level `fossps new` puts in a game, `template/level.glb`, and
//! checks that the file in the repository is the one built here.
//!
//! The level is a small arena to block a first map out from: a chequered
//! floor 32 m on a side with a wall around it, a red end and a blue end,
//! cover of three heights and two platforms with ramps. Players start at
//! one of three spawn points at each end, looking at the other end. Its
//! middle is left clear.
//!
//! To change the level, change it here, then write the file again:
//! `FOSSPS_WRITE_STARTER_LEVEL=1 cargo test -p fossps starter_level`.

use std::fmt::Write as _;

use fossps_assets::{Instance, Marker, Material, Mesh, MeshPart, Scene, Transform};

use crate::standin::{block, face, tile};

/// How wide one floor tile is, in metres.
const TILE: f32 = 2.0;
/// How many tiles the floor has along each side.
const TILES: i16 = 16;
/// How far the floor reaches from the middle, in metres.
const HALF: f32 = 16.0;
/// How high the wall around the floor is, in metres: well over a player's 1.8.
const WALL_HEIGHT: f32 = 3.0;
/// How thick that wall is, in metres.
const WALL_THICK: f32 = 0.5;
/// How far from the middle each end's spawn points are, in metres: between
/// that end's low wall and the wall around the level.
const SPAWN_OUT: f32 = 14.0;
/// How far apart the spawn points at one end are, in metres.
const SPAWN_APART: f32 = 6.0;

const LIGHT_FLOOR: [f32; 4] = [0.46, 0.47, 0.49, 1.0];
const DARK_FLOOR: [f32; 4] = [0.34, 0.35, 0.37, 1.0];
const WALL: [f32; 4] = [0.72, 0.71, 0.68, 1.0];
const PLATFORM: [f32; 4] = [0.56, 0.58, 0.61, 1.0];
const COVER: [f32; 4] = [0.86, 0.48, 0.16, 1.0];
const RED: [f32; 4] = [0.78, 0.2, 0.18, 1.0];
const BLUE: [f32; 4] = [0.18, 0.4, 0.82, 1.0];

/// The part of a slope's surface direction along each of its two axes.
const SLOPE: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// A ramp one metre along each side: level with the ground at its +Z edge
/// and one metre high at its -Z edge.
fn ramp() -> MeshPart {
    let mut part = MeshPart::default();
    let (l, r, b, t, n, s) = (-0.5, 0.5, 0.0, 1.0, -0.5, 0.5);
    face(
        &mut part,
        [[l, b, s], [r, b, s], [r, t, n], [l, t, n]],
        [0.0, SLOPE, SLOPE],
    );
    face(
        &mut part,
        [[l, b, n], [r, b, n], [r, b, s], [l, b, s]],
        [0.0, -1.0, 0.0],
    );
    face(
        &mut part,
        [[r, b, n], [l, b, n], [l, t, n], [r, t, n]],
        [0.0, 0.0, -1.0],
    );
    for (corners, normal) in [
        ([[r, b, s], [r, b, n], [r, t, n]], [1.0, 0.0, 0.0]),
        ([[l, b, n], [l, b, s], [l, t, n]], [-1.0, 0.0, 0.0]),
    ] {
        // A ramp has eighteen points.
        #[allow(clippy::cast_possible_truncation)]
        let first = part.positions.len() as u32;
        part.positions.extend(corners);
        part.normals.extend([normal; 3]);
        part.indices.extend([first, first + 1, first + 2]);
    }
    part
}

/// The transform that puts a shape with its middle over `(x, z)` on the
/// ground, `size` times as large. `turned` turns it half way round, so a
/// ramp climbs towards +Z instead of -Z.
fn stand([x, z]: [f32; 2], [wide, high, deep]: [f32; 3], turned: bool) -> Transform {
    let way = if turned { -1.0 } else { 1.0 };
    [
        [wide * way, 0.0, 0.0, 0.0],
        [0.0, high, 0.0, 0.0],
        [0.0, 0.0, deep * way, 0.0],
        [x, 0.0, z, 1.0],
    ]
}

/// Builds the starter level.
pub(crate) fn scene() -> Scene {
    let mut scene = Scene::default();
    let mut add = |name: &str, part: MeshPart, base_color: [f32; 4]| -> u32 {
        // A handful of meshes are made here.
        #[allow(clippy::cast_possible_truncation)]
        let number = scene.meshes.len() as u32;
        scene.materials.push(Material {
            name: Some(name.to_owned()),
            base_color,
            ..Material::default()
        });
        scene.meshes.push(Mesh {
            name: Some(name.to_owned()),
            parts: vec![MeshPart {
                material: number,
                ..part
            }],
        });
        number
    };

    let mut light = MeshPart::default();
    let mut dark = MeshPart::default();
    for row in -TILES / 2..TILES / 2 {
        for column in -TILES / 2..TILES / 2 {
            let part = if (row + column) % 2 == 0 {
                &mut light
            } else {
                &mut dark
            };
            let (x, z) = (f32::from(column) * TILE, f32::from(row) * TILE);
            tile(part, [x, x + TILE], 0.0, [z, z + TILE]);
        }
    }
    let light = add("Floor Light", light, LIGHT_FLOOR);
    let dark = add("Floor Dark", dark, DARK_FLOOR);
    let wall = add("Wall", block(), WALL);
    let platform = add("Platform", block(), PLATFORM);
    let slope = add("Ramp", ramp(), PLATFORM);
    let cover = add("Cover", block(), COVER);
    let red = add("Red", block(), RED);
    let blue = add("Blue", block(), BLUE);

    let mut placed: Vec<(&str, u32, Transform)> = Vec::new();
    let unmoved = stand([0.0, 0.0], [1.0; 3], false);
    placed.push(("Floor Light", light, unmoved));
    placed.push(("Floor Dark", dark, unmoved));

    let edge = HALF + WALL_THICK / 2.0;
    let long = (HALF + WALL_THICK) * 2.0;
    for (name, [x, z], size) in [
        ("Wall North", [0.0, -edge], [long, WALL_HEIGHT, WALL_THICK]),
        ("Wall South", [0.0, edge], [long, WALL_HEIGHT, WALL_THICK]),
        (
            "Wall West",
            [-edge, 0.0],
            [WALL_THICK, WALL_HEIGHT, HALF * 2.0],
        ),
        (
            "Wall East",
            [edge, 0.0],
            [WALL_THICK, WALL_HEIGHT, HALF * 2.0],
        ),
    ] {
        placed.push((name, wall, stand([x, z], size, false)));
    }

    // Everything below is placed twice, the second time half a turn round
    // the middle, so neither end has the better ground. North (-Z) is red.
    for (north, south, mesh, [x, z], size) in [
        (
            "Red Wall",
            "Blue Wall",
            [red, blue],
            [0.0, -11.0],
            [6.0, 1.0, 0.5],
        ),
        (
            "Red Post West",
            "Blue Post East",
            [red, blue],
            [-13.0, -13.0],
            [1.0, 3.0, 1.0],
        ),
        (
            "Red Post East",
            "Blue Post West",
            [red, blue],
            [13.0, -13.0],
            [1.0, 3.0, 1.0],
        ),
        (
            "Cover Low North",
            "Cover Low South",
            [cover; 2],
            [-4.0, -5.0],
            [2.0, 1.0, 2.0],
        ),
        (
            "Cover High North",
            "Cover High South",
            [cover; 2],
            [5.0, -6.0],
            [1.0, 2.0, 3.0],
        ),
        (
            "Cover Tall North",
            "Cover Tall South",
            [cover; 2],
            [-9.0, -9.0],
            [2.0, 3.0, 2.0],
        ),
        (
            "Platform East",
            "Platform West",
            [platform; 2],
            [11.0, 0.0],
            [6.0, 1.5, 6.0],
        ),
        (
            "Ramp East",
            "Ramp West",
            [slope; 2],
            [11.0, 5.25],
            [3.0, 1.5, 4.5],
        ),
    ] {
        placed.push((north, mesh[0], stand([x, z], size, false)));
        placed.push((south, mesh[1], stand([-x, -z], size, true)));
    }

    scene.instances = placed
        .into_iter()
        .map(|(name, mesh, transform)| Instance {
            mesh,
            transform,
            mirrored: false,
            name: Some(name.to_owned()),
            extras: None,
        })
        .collect();

    // Where players start: three at each end, behind that end's low wall
    // and looking over it at the other end. Named as Blender names copies.
    for (number, x) in [0.0, -SPAWN_APART, SPAWN_APART].into_iter().enumerate() {
        let copy = if number == 0 {
            String::new()
        } else {
            format!(".{number:03}")
        };
        for (team, [x, z], turned) in [
            ("red", [x, -SPAWN_OUT], true),
            // Adding zero keeps the middle one from being written as "-0".
            ("blue", [-x + 0.0, SPAWN_OUT], false),
        ] {
            scene.markers.push(Marker {
                name: Some(format!("spawn.{team}{copy}")),
                transform: stand([x, z], [1.0; 3], turned),
                extras: None,
            });
        }
    }
    scene
}

/// Writes `scene` as a glTF Binary file: its shapes with their surface
/// directions, each shape's one colour, and where the shapes stand, by name.
/// Its markers follow, by name, with where they stand. That is all the
/// starter level has; textures and lights are not written.
pub(crate) fn glb(scene: &Scene) -> Vec<u8> {
    /// What glTF calls 32-bit floats and 32-bit whole numbers.
    const FLOAT: u32 = 5126;
    const UNSIGNED_INT: u32 = 5125;
    /// What glTF calls data for corner points and data for triangle corners.
    const POINTS: u32 = 34962;
    const CORNERS: u32 = 34963;

    let list = |numbers: &[f32]| {
        let mut text = String::from("[");
        for (place, number) in numbers.iter().enumerate() {
            let comma = if place == 0 { "" } else { "," };
            let _ = write!(text, "{comma}{number}");
        }
        text.push(']');
        text
    };

    let mut data: Vec<u8> = Vec::new();
    let (mut views, mut accessors, mut meshes) = (Vec::new(), Vec::new(), Vec::new());
    for mesh in &scene.meshes {
        let mut primitives = Vec::new();
        for part in &mesh.parts {
            let first = accessors.len();
            let mut least = [f32::INFINITY; 3];
            let mut most = [f32::NEG_INFINITY; 3];
            for point in &part.positions {
                for axis in 0..3 {
                    least[axis] = least[axis].min(point[axis]);
                    most[axis] = most[axis].max(point[axis]);
                }
            }
            for (points, limits) in [
                (
                    &part.positions,
                    format!(",\"min\":{},\"max\":{}", list(&least), list(&most)),
                ),
                (&part.normals, String::new()),
            ] {
                views.push(format!(
                    "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{},\"target\":{POINTS}}}",
                    data.len(),
                    points.len() * 12
                ));
                accessors.push(format!(
                    "{{\"bufferView\":{},\"componentType\":{FLOAT},\"count\":{},\
                     \"type\":\"VEC3\"{limits}}}",
                    views.len() - 1,
                    points.len()
                ));
                data.extend(points.iter().flatten().flat_map(|n| n.to_le_bytes()));
            }
            views.push(format!(
                "{{\"buffer\":0,\"byteOffset\":{},\"byteLength\":{},\"target\":{CORNERS}}}",
                data.len(),
                part.indices.len() * 4
            ));
            accessors.push(format!(
                "{{\"bufferView\":{},\"componentType\":{UNSIGNED_INT},\"count\":{},\
                 \"type\":\"SCALAR\"}}",
                views.len() - 1,
                part.indices.len()
            ));
            data.extend(part.indices.iter().flat_map(|n| n.to_le_bytes()));
            primitives.push(format!(
                "{{\"attributes\":{{\"POSITION\":{first},\"NORMAL\":{}}},\"indices\":{},\
                 \"material\":{}}}",
                first + 1,
                first + 2,
                part.material
            ));
        }
        meshes.push(format!(
            "{{\"name\":\"{}\",\"primitives\":[{}]}}",
            mesh.name.as_deref().unwrap_or_default(),
            primitives.join(",")
        ));
    }
    let materials: Vec<String> = scene
        .materials
        .iter()
        .map(|material| {
            format!(
                "{{\"name\":\"{}\",\"pbrMetallicRoughness\":{{\"baseColorFactor\":{},\
                 \"metallicFactor\":0,\"roughnessFactor\":1}}}}",
                material.name.as_deref().unwrap_or_default(),
                list(&material.base_color)
            )
        })
        .collect();
    let shapes = scene.instances.iter().map(|instance| {
        format!(
            "{{\"name\":\"{}\",\"mesh\":{},\"matrix\":{}}}",
            instance.name.as_deref().unwrap_or_default(),
            instance.mesh,
            list(instance.transform.as_flattened())
        )
    });
    let markers = scene.markers.iter().map(|marker| {
        format!(
            "{{\"name\":\"{}\",\"matrix\":{}}}",
            marker.name.as_deref().unwrap_or_default(),
            list(marker.transform.as_flattened())
        )
    });
    let nodes: Vec<String> = shapes.chain(markers).collect();
    let every_node: Vec<String> = (0..nodes.len()).map(|node| node.to_string()).collect();

    let mut json = format!(
        "{{\"asset\":{{\"version\":\"2.0\",\"generator\":\"FOSSPS starter level\"}},\
         \"scene\":0,\"scenes\":[{{\"name\":\"Level\",\"nodes\":[{}]}}],\"nodes\":[{}],\
         \"meshes\":[{}],\"materials\":[{}],\"accessors\":[{}],\"bufferViews\":[{}],\
         \"buffers\":[{{\"byteLength\":{}}}]}}",
        every_node.join(","),
        nodes.join(","),
        meshes.join(","),
        materials.join(","),
        accessors.join(","),
        views.join(","),
        data.len()
    );
    // Each of the file's two parts ends on a multiple of four bytes. The
    // data is whole 4-byte numbers already.
    while json.len() % 4 != 0 {
        json.push(' ');
    }

    let length = 12 + 8 + json.len() + 8 + data.len();
    let number = |n: usize| u32::try_from(n).unwrap_or(u32::MAX).to_le_bytes();
    let mut file = Vec::with_capacity(length);
    file.extend(b"glTF");
    file.extend(2_u32.to_le_bytes());
    file.extend(number(length));
    file.extend(number(json.len()));
    file.extend(b"JSON");
    file.extend(json.as_bytes());
    file.extend(number(data.len()));
    file.extend(b"BIN\0");
    file.extend(data);
    file
}

mod tests {
    use fossps_assets::LoadOptions;

    use super::*;
    use crate::scaffold::STARTER_LEVEL;

    fn loaded() -> Scene {
        Scene::from_bytes("level.glb", STARTER_LEVEL, None, LoadOptions::default()).unwrap()
    }

    /// Where `instance`'s shape reaches in the level: its lowest and highest
    /// corner.
    fn reach(scene: &Scene, instance: &Instance) -> ([f32; 3], [f32; 3]) {
        let mut least = [f32::INFINITY; 3];
        let mut most = [f32::NEG_INFINITY; 3];
        for part in &scene.meshes[instance.mesh as usize].parts {
            for point in &part.positions {
                for axis in 0..3 {
                    let moved: f32 = (0..3)
                        .map(|from| instance.transform[from][axis] * point[from])
                        .sum::<f32>()
                        + instance.transform[3][axis];
                    least[axis] = least[axis].min(moved);
                    most[axis] = most[axis].max(moved);
                }
            }
        }
        (least, most)
    }

    #[test]
    fn the_file_in_the_repository_is_the_level_built_here() {
        let built = glb(&scene());
        if std::env::var_os("FOSSPS_WRITE_STARTER_LEVEL").is_some() {
            let file = concat!(env!("CARGO_MANIFEST_DIR"), "/template/level.glb");
            std::fs::write(file, &built).unwrap();
            return;
        }
        assert!(
            built == STARTER_LEVEL,
            "template/level.glb is not what starter_level.rs builds. Write it again with \
             FOSSPS_WRITE_STARTER_LEVEL=1 cargo test -p fossps starter_level"
        );
        assert!(built.len() < 64 * 1024, "{} bytes", built.len());
    }

    #[test]
    fn the_file_loads_as_the_level_that_was_built() {
        let (built, loaded) = (scene(), loaded());
        assert_eq!(loaded.warnings, Vec::<String>::new());
        assert_eq!(loaded.meshes.len(), built.meshes.len());
        assert_eq!(loaded.instances.len(), built.instances.len());
        for (read, made) in loaded.instances.iter().zip(&built.instances) {
            assert_eq!(read.name, made.name);
            assert_eq!(read.transform, made.transform, "{:?}", made.name);
            assert!(!read.mirrored, "{:?}", made.name);
            let (read, made) = (
                &loaded.meshes[read.mesh as usize].parts[0],
                &built.meshes[made.mesh as usize].parts[0],
            );
            assert_eq!(read.positions, made.positions);
            assert_eq!(read.normals, made.normals);
            assert_eq!(read.indices, made.indices);
            assert_eq!(
                loaded.materials[read.material as usize].base_color,
                built.materials[made.material as usize].base_color
            );
        }
        // Every placed thing has a name of its own, for the editor's list.
        let mut names: Vec<_> = loaded.instances.iter().map(|i| i.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), loaded.instances.len());
    }

    #[test]
    fn the_floor_is_at_height_zero_and_the_middle_is_clear() {
        let scene = loaded();
        for instance in &scene.instances {
            let name = instance.name.as_deref().unwrap();
            let (least, most) = reach(&scene, instance);
            if name.starts_with("Floor") {
                // Height zero, where a game with no level has its floor.
                assert_eq!((least[1], most[1]), (0.0, 0.0));
                assert_eq!((least[0], most[2]), (-HALF, HALF));
                continue;
            }
            assert_eq!(least[1], 0.0, "{name} stands on the floor");
            assert!(most[1] <= WALL_HEIGHT, "{name}");
            // The middle is open ground, where the two ends meet.
            let clear = 3.0;
            let over_middle =
                least[0] < clear && most[0] > -clear && least[2] < clear && most[2] > -clear;
            assert!(!over_middle, "{name} is in the middle");
            let inside = HALF + WALL_THICK;
            assert!(least[0] >= -inside && most[0] <= inside, "{name}");
            assert!(least[2] >= -inside && most[2] <= inside, "{name}");
        }
    }

    #[test]
    fn players_start_at_either_end_with_room_to_stand_and_a_floor_to_land_on() {
        let (built, scene) = (scene(), loaded());
        assert_eq!(scene.markers, built.markers);
        let names: Vec<_> = scene
            .markers
            .iter()
            .map(|marker| marker.name.as_deref().unwrap())
            .collect();
        assert_eq!(
            names,
            [
                "spawn.red",
                "spawn.blue",
                "spawn.red.001",
                "spawn.blue.001",
                "spawn.red.002",
                "spawn.blue.002"
            ]
        );
        // Nothing stands within a metre of a spawn point, the floor aside.
        for marker in &scene.markers {
            let [x, y, z, _] = marker.transform[3];
            assert_eq!(y, 0.0);
            assert_eq!(z.abs(), SPAWN_OUT);
            for instance in &scene.instances {
                let name = instance.name.as_deref().unwrap();
                let (least, most) = reach(&scene, instance);
                let near = least[0] < x + 1.0
                    && most[0] > x - 1.0
                    && least[2] < z + 1.0
                    && most[2] > z - 1.0;
                assert!(name.starts_with("Floor") || !near, "{name} is at {x}, {z}");
            }
        }
        // And a player started at each one lands.
        let walked = crate::online::try_walking(&scene, &crate::map::Map::default());
        assert_eq!(walked.spawn_points, 6);
        assert_eq!(walked.problems, Vec::<String>::new());
    }

    #[test]
    fn every_triangle_faces_the_way_its_surface_does() {
        let scene = scene();
        for mesh in &scene.meshes {
            let part = &mesh.parts[0];
            assert_eq!(part.positions.len(), part.normals.len());
            for (number, [a, b, c]) in part.triangles().enumerate() {
                let (u, v) = (
                    [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                    [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                );
                let facing = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let normal = part.normals[part.indices[number * 3] as usize];
                let agreement: f32 = facing.iter().zip(normal).map(|(f, n)| f * n).sum();
                assert!(agreement > 0.0, "{:?} triangle {number}", mesh.name);
            }
        }
    }

    /// The level is drawn for real, with no window, from where a player
    /// starts: the red end ahead, under the wall, on the floor.
    #[test]
    fn the_level_is_drawn_with_the_red_end_ahead_of_where_players_start() {
        use fossps_render::{Camera, Headless, RenderError, Upscaling};

        const SCREEN: [u32; 2] = [1280, 720];
        let mut headless = match Headless::new(SCREEN[0], SCREEN[1], Upscaling::Off) {
            Ok(headless) => headless,
            Err(
                error @ (RenderError::NoVulkan(_)
                | RenderError::OldVulkan { .. }
                | RenderError::NoSuitableGpu { .. }),
            ) => {
                let required =
                    std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
                assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {error}");
                eprintln!("Skipping the drawing test: {error}");
                return;
            }
            Err(error) => panic!("{error}"),
        };
        let sky = headless.screenshot(&Camera::default()).unwrap();
        headless.set_scene(&loaded()).unwrap();
        let eyes = Camera {
            position: [0.0, 1.6, 0.0],
            ..Camera::default()
        };
        let seen = headless.screenshot(&eyes).unwrap();
        // From a corner, looking down across the level.
        let above = Camera {
            position: [-22.0, 16.0, 22.0],
            yaw: -std::f32::consts::FRAC_PI_4,
            pitch: -0.5,
            ..Camera::default()
        };
        let overview = headless.screenshot(&above).unwrap();
        assert_eq!(fossps_render::validation_errors(), 0);
        // Set `FOSSPS_SAVE_PICTURES` to a folder to look at the pictures:
        // 1280 by 720 pixels of four bytes each.
        if let Some(folder) = std::env::var_os("FOSSPS_SAVE_PICTURES") {
            let folder = std::path::Path::new(&folder);
            std::fs::write(folder.join("starter-level.rgba"), &seen).unwrap();
            std::fs::write(folder.join("starter-level-above.rgba"), &overview).unwrap();
        }

        let pixel = |picture: &[u8], x: u32, y: u32| {
            let start = ((y * SCREEN[0] + x) * 4) as usize;
            [picture[start], picture[start + 1], picture[start + 2]].map(i32::from)
        };
        // The top of the picture is sky, above the wall.
        assert_eq!(pixel(&seen, 640, 10), pixel(&sky, 640, 10));
        // Straight ahead, just under the horizon, is the low red wall.
        let [red, green, blue] = pixel(&seen, 640, 410);
        assert!(red > green + 60 && red > blue + 60, "{red} {green} {blue}");
        // Above it is the wall around the level: light and without a colour.
        let [red, green, blue] = pixel(&seen, 640, 350);
        assert!(red > 120 && (red - blue).abs() < 40, "{red} {green} {blue}");
        // The bottom is the chequered floor: grey, in two shades.
        let shades: std::collections::BTreeSet<_> = (0..SCREEN[0])
            .map(|x| pixel(&seen, x, 700))
            .inspect(|[red, _, blue]| assert!((red - blue).abs() < 30))
            .collect();
        assert!(shades.len() >= 2, "{shades:?}");
        assert_ne!(pixel(&seen, 640, 700), pixel(&sky, 640, 700));
    }
}
