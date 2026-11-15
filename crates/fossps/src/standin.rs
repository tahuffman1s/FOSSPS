//! What a game with no level of its own is shown: a chequered floor and a
//! few coloured blocks to walk between.
//!
//! The floor is where the physics has it, at height 0. The blocks are only
//! drawn; players walk through them.

use fossps_assets::{Instance, Material, Mesh, MeshPart, Scene, Transform};

/// How wide one floor tile is, in metres.
const TILE: f32 = 2.0;
/// How many tiles the chequered part of the floor has along each side.
const TILES: i16 = 64;
/// How far the plain floor around the tiles reaches from the middle, in metres.
const FLOOR: f32 = 500.0;

/// The colours of the blocks.
const PAINTS: [[f32; 4]; 4] = [
    [0.80, 0.22, 0.18, 1.0],
    [0.20, 0.45, 0.85, 1.0],
    [0.90, 0.70, 0.15, 1.0],
    [0.25, 0.65, 0.35, 1.0],
];

/// Builds the stand-in level.
pub(crate) fn scene() -> Scene {
    let mut scene = Scene::default();
    let mut add = |name: &str, part: MeshPart, base_color: [f32; 4]| -> u32 {
        // Far fewer than `u32::MAX` meshes are made here.
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
    let mut around = MeshPart::default();
    // Just under the tiles, so the two never fight over a pixel.
    tile(&mut around, [-FLOOR, FLOOR], -0.02, [-FLOOR, FLOOR]);

    let mut placed = Vec::new();
    let here = |mesh: u32| (mesh, at([0.0; 3], [1.0; 3]));
    placed.push(here(add("light tiles", light, [0.42, 0.44, 0.47, 1.0])));
    placed.push(here(add("dark tiles", dark, [0.26, 0.28, 0.31, 1.0])));
    placed.push(here(add("ground", around, [0.20, 0.22, 0.24, 1.0])));

    let blocks = PAINTS.map(|paint| add("block", block(), paint));
    // A ring of twelve blocks around the starting point, then four tall
    // ones further out as landmarks.
    for step in 0_u8..12 {
        let angle = f32::from(step) * std::f32::consts::TAU / 12.0;
        let (sin, cos) = angle.sin_cos();
        let height = 1.0 + f32::from(step % 3);
        placed.push((
            blocks[usize::from(step % 4)],
            at([sin * 12.0, 0.0, -cos * 12.0], [2.0, height, 2.0]),
        ));
    }
    for (corner, &mesh) in blocks.iter().enumerate() {
        let x = if corner % 2 == 0 { -40.0 } else { 40.0 };
        let z = if corner < 2 { -40.0 } else { 40.0 };
        placed.push((mesh, at([x, 0.0, z], [4.0, 14.0, 4.0])));
    }

    scene.instances = placed
        .into_iter()
        .map(|(mesh, transform)| Instance {
            mesh,
            transform,
            mirrored: false,
            name: None,
            extras: None,
        })
        .collect();
    scene
}

/// The transform that puts a shape at `position`, `size` times as large
/// along each axis.
pub(crate) fn at([x, y, z]: [f32; 3], [wide, high, deep]: [f32; 3]) -> Transform {
    [
        [wide, 0.0, 0.0, 0.0],
        [0.0, high, 0.0, 0.0],
        [0.0, 0.0, deep, 0.0],
        [x, y, z, 1.0],
    ]
}

/// Adds four points and the two triangles between them to `part`. The
/// points must run anticlockwise seen from the side `normal` points to.
pub(crate) fn face(part: &mut MeshPart, corners: [[f32; 3]; 4], normal: [f32; 3]) {
    // A part here holds a few thousand points at most.
    #[allow(clippy::cast_possible_truncation)]
    let first = part.positions.len() as u32;
    part.positions.extend(corners);
    part.normals.extend([normal; 4]);
    part.indices
        .extend([0, 1, 2, 0, 2, 3].map(|corner| first + corner));
}

/// Adds a flat rectangle facing up, at height `y`, to `part`.
pub(crate) fn tile(part: &mut MeshPart, [west, east]: [f32; 2], y: f32, [north, south]: [f32; 2]) {
    face(
        part,
        [
            [west, y, south],
            [east, y, south],
            [east, y, north],
            [west, y, north],
        ],
        [0.0, 1.0, 0.0],
    );
}

/// A block one metre along each side, standing on the origin.
pub(crate) fn block() -> MeshPart {
    let mut part = MeshPart::default();
    let (l, r, b, t, n, s) = (-0.5, 0.5, 0.0, 1.0, -0.5, 0.5);
    face(
        &mut part,
        [[l, t, s], [r, t, s], [r, t, n], [l, t, n]],
        [0.0, 1.0, 0.0],
    );
    face(
        &mut part,
        [[l, b, n], [r, b, n], [r, b, s], [l, b, s]],
        [0.0, -1.0, 0.0],
    );
    face(
        &mut part,
        [[l, b, s], [r, b, s], [r, t, s], [l, t, s]],
        [0.0, 0.0, 1.0],
    );
    face(
        &mut part,
        [[r, b, n], [l, b, n], [l, t, n], [r, t, n]],
        [0.0, 0.0, -1.0],
    );
    face(
        &mut part,
        [[r, b, s], [r, b, n], [r, t, n], [r, t, s]],
        [1.0, 0.0, 0.0],
    );
    face(
        &mut part,
        [[l, b, n], [l, b, s], [l, t, s], [l, t, n]],
        [-1.0, 0.0, 0.0],
    );
    part
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    #[test]
    fn every_triangle_faces_the_way_its_surface_does() {
        let scene = scene();
        assert!(!scene.instances.is_empty());
        for mesh in &scene.meshes {
            for part in &mesh.parts {
                assert_eq!(part.positions.len(), part.normals.len());
                assert!((part.material as usize) < scene.materials.len());
                for (number, [a, b, c]) in part.triangles().enumerate() {
                    let (u, v) = (sub(b, a), sub(c, a));
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
        assert_eq!(
            scene
                .meshes
                .iter()
                .map(|mesh| mesh.parts[0].indices.len())
                .sum::<usize>(),
            (usize::try_from(TILES).unwrap().pow(2) + 1 + 6 * PAINTS.len()) * 6
        );
    }

    #[test]
    fn the_floor_is_where_the_physics_has_it() {
        let scene = scene();
        let tiles = &scene.meshes[0].parts[0];
        assert!(tiles.positions.iter().all(|point| point[1] == 0.0));
    }
}
