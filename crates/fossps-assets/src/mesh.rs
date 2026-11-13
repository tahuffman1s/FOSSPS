//! Turns one glTF primitive into a [`MeshPart`].

use gltf::Semantic;
use gltf::accessor::{DataType, Dimensions};
use gltf::mesh::Mode;

use crate::error::AssetError;
use crate::math::{cross, normalized, sub};
use crate::scene::{Bounds, MeshPart};

/// What reading a part needs to know from outside it.
pub(crate) struct PartContext<'a> {
    /// The file's name, for messages.
    pub(crate) file: &'a str,
    /// The mesh's name, for messages.
    pub(crate) mesh: &'a str,
    /// The data of each glTF buffer.
    pub(crate) buffers: &'a [Vec<u8>],
    /// The scene material this part is drawn with.
    pub(crate) material: u32,
    /// The UV map tangents must follow, if the material has a normal map
    /// and tangents are wanted.
    pub(crate) tangent_uv: Option<u8>,
}

/// Reads one part of a mesh, or `None` (with a warning) if it is not made of triangles.
pub(crate) fn part(
    context: &PartContext<'_>,
    primitive: &gltf::Primitive<'_>,
    warnings: &mut Vec<String>,
) -> Result<Option<MeshPart>, AssetError> {
    let PartContext { file, mesh, .. } = *context;
    let mode = primitive.mode();
    if !matches!(
        mode,
        Mode::Triangles | Mode::TriangleStrip | Mode::TriangleFan
    ) {
        warnings.push(format!(
            "Mesh \"{mesh}\" has loose points or lines, which are not drawn. \
             Give them thickness in your 3D tool (in Blender, convert the curve or edges to a mesh with faces)."
        ));
        return Ok(None);
    }
    let Some(position_accessor) = primitive.get(&Semantic::Positions) else {
        warnings.push(format!(
            "Mesh \"{mesh}\" has a part with no positions, which was skipped."
        ));
        return Ok(None);
    };
    let count = position_accessor.count();

    // The glTF reader trusts each accessor to hold what its role says, so
    // that is checked first; anything else would be read as garbage.
    let float = [DataType::F32];
    let unit = [DataType::F32, DataType::U8, DataType::U16];
    for (semantic, accessor) in primitive.attributes() {
        let (what, dimensions, types): (_, &[Dimensions], &[DataType]) = match semantic {
            Semantic::Positions => ("positions", &[Dimensions::Vec3], &float),
            Semantic::Normals => ("normals", &[Dimensions::Vec3], &float),
            Semantic::Tangents => ("tangents", &[Dimensions::Vec4], &float),
            Semantic::TexCoords(0 | 1) => ("UV map", &[Dimensions::Vec2], &unit),
            Semantic::Colors(0) => ("colours", &[Dimensions::Vec3, Dimensions::Vec4], &unit),
            // Not read, so not checked.
            _ => continue,
        };
        check(context, &accessor, what, dimensions, types)?;
        if accessor.count() != count {
            return Err(AssetError::damaged(
                file,
                format!(
                    "mesh \"{mesh}\" has {count} positions but {} {what} entries",
                    accessor.count()
                ),
            ));
        }
    }
    if let Some(accessor) = primitive.indices() {
        check(
            context,
            &accessor,
            "triangle list",
            &[Dimensions::Scalar],
            &[DataType::U8, DataType::U16, DataType::U32],
        )?;
    }
    let Ok(count) = u32::try_from(count) else {
        return Err(AssetError::damaged(
            file,
            format!("mesh \"{mesh}\" has more points than fit in one mesh"),
        ));
    };

    let reader = primitive.reader(|buffer| context.buffers.get(buffer.index()).map(Vec::as_slice));
    let unreadable = || {
        AssetError::damaged(
            file,
            format!("mesh \"{mesh}\" has data that cannot be read"),
        )
    };
    let mut positions: Vec<[f32; 3]> = reader.read_positions().ok_or_else(unreadable)?.collect();
    let listed: Vec<u32> = match reader.read_indices() {
        Some(indices) => indices.into_u32().collect(),
        None => (0..count).collect(),
    };
    if listed.iter().any(|&index| index >= count) {
        return Err(AssetError::damaged(
            file,
            format!("mesh \"{mesh}\" has a triangle that uses a point the mesh does not have"),
        ));
    }
    let mut indices = triangle_list(mode, &listed);

    let mut normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(Iterator::collect);
    let mut tangents: Option<Vec<[f32; 4]>> = reader.read_tangents().map(Iterator::collect);
    let mut uv0: Option<Vec<[f32; 2]>> =
        reader.read_tex_coords(0).map(|uv| uv.into_f32().collect());
    let mut uv1: Option<Vec<[f32; 2]>> =
        reader.read_tex_coords(1).map(|uv| uv.into_f32().collect());
    let mut colors: Option<Vec<[f32; 4]>> = reader
        .read_colors(0)
        .map(|colors| colors.into_rgba_f32().collect());

    let normals = if let Some(normals) = normals.take() {
        normals
    } else {
        // A flat face needs its own three points, so points shared between
        // faces are split and every list is rebuilt to match.
        positions = unweld(&positions, &indices);
        tangents = tangents.map(|list| unweld(&list, &indices));
        uv0 = uv0.map(|list| unweld(&list, &indices));
        uv1 = uv1.map(|list| unweld(&list, &indices));
        colors = colors.map(|list| unweld(&list, &indices));
        let Ok(corners) = u32::try_from(positions.len()) else {
            return Err(AssetError::damaged(
                file,
                format!("mesh \"{mesh}\" has more triangles than fit in one mesh"),
            ));
        };
        indices = (0..corners).collect();
        flat_normals(&positions)
    };

    if tangents.is_none()
        && let Some(set) = context.tangent_uv
        && let Some(uv) = if set == 0 { &uv0 } else { &uv1 }
    {
        tangents = Some(generate_tangents(&positions, &normals, uv, &indices));
    }

    Ok(Some(MeshPart {
        bounds: bounds(&positions),
        positions,
        normals,
        tangents,
        uv0,
        uv1,
        colors,
        indices,
        material: context.material,
    }))
}

/// Makes sure `accessor` holds the kind of data its role needs.
fn check(
    context: &PartContext<'_>,
    accessor: &gltf::Accessor<'_>,
    what: &str,
    dimensions: &[Dimensions],
    types: &[DataType],
) -> Result<(), AssetError> {
    let PartContext { file, mesh, .. } = *context;
    if !dimensions.contains(&accessor.dimensions()) || !types.contains(&accessor.data_type()) {
        return Err(AssetError::damaged(
            file,
            format!(
                "mesh \"{mesh}\" stores its {what} as {:?} {:?}, which glTF does not allow",
                accessor.data_type(),
                accessor.dimensions()
            ),
        ));
    }
    if accessor.count() == 0 {
        return Err(AssetError::damaged(
            file,
            format!("mesh \"{mesh}\" has an empty {what}"),
        ));
    }
    Ok(())
}

/// Rewrites strips and fans as a plain list of triangles.
fn triangle_list(mode: Mode, listed: &[u32]) -> Vec<u32> {
    match mode {
        Mode::TriangleStrip => listed
            .windows(3)
            .enumerate()
            // Every second triangle of a strip runs the other way round.
            .map(|(n, w)| {
                if n % 2 == 0 {
                    [w[0], w[1], w[2]]
                } else {
                    [w[1], w[0], w[2]]
                }
            })
            // Strips repeat a point to jump; those triangles have no area.
            .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
            .flatten()
            .collect(),
        Mode::TriangleFan => listed
            .get(1..)
            .unwrap_or_default()
            .windows(2)
            .flat_map(|w| [listed[0], w[0], w[1]])
            .collect(),
        // A list with one or two points left over ignores them.
        _ => listed[..listed.len() - listed.len() % 3].to_vec(),
    }
}

/// Gives every triangle corner its own entry.
fn unweld<T: Copy + Default>(list: &[T], indices: &[u32]) -> Vec<T> {
    indices
        .iter()
        .map(|&index| list.get(index as usize).copied().unwrap_or_default())
        .collect()
}

/// One normal per corner, facing straight out of its triangle.
fn flat_normals(positions: &[[f32; 3]]) -> Vec<[f32; 3]> {
    positions
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|corner| {
            let normal = normalized(cross(sub(corner[1], corner[0]), sub(corner[2], corner[0])))
                // A triangle with no area faces nowhere; up is as good as any.
                .unwrap_or([0.0, 1.0, 0.0]);
            [normal; 3]
        })
        .collect()
}

fn bounds(positions: &[[f32; 3]]) -> Bounds {
    let Some(&first) = positions.first() else {
        return Bounds::default();
    };
    positions.iter().fold(
        Bounds {
            min: first,
            max: first,
        },
        |mut bounds, point| {
            for ((min, max), &value) in bounds.min.iter_mut().zip(&mut bounds.max).zip(point) {
                *min = min.min(value);
                *max = max.max(value);
            }
            bounds
        },
    )
}

/// The mesh as the tangent generator wants to see it.
struct TangentInput<'a> {
    positions: &'a [[f32; 3]],
    normals: &'a [[f32; 3]],
    uv: &'a [[f32; 2]],
    indices: &'a [u32],
    tangents: Vec<[f32; 4]>,
}

impl TangentInput<'_> {
    fn point(&self, face: usize, corner: usize) -> usize {
        self.indices
            .get(face * 3 + corner)
            .map_or(0, |&index| index as usize)
    }
}

impl bevy_mikktspace::Geometry for TangentInput<'_> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }

    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }

    fn position(&self, face: usize, corner: usize) -> [f32; 3] {
        self.positions
            .get(self.point(face, corner))
            .copied()
            .unwrap_or_default()
    }

    fn normal(&self, face: usize, corner: usize) -> [f32; 3] {
        self.normals
            .get(self.point(face, corner))
            .copied()
            .unwrap_or_default()
    }

    fn tex_coord(&self, face: usize, corner: usize) -> [f32; 2] {
        self.uv
            .get(self.point(face, corner))
            .copied()
            .unwrap_or_default()
    }

    fn set_tangent(
        &mut self,
        tangent: Option<bevy_mikktspace::TangentSpace>,
        face: usize,
        corner: usize,
    ) {
        let point = self.point(face, corner);
        if let (Some(tangent), Some(slot)) = (tangent, self.tangents.get_mut(point)) {
            *slot = tangent.tangent_encoded();
        }
    }
}

/// Works out tangents the standard (MikkTSpace) way, as glTF asks for.
///
/// Points stay shared: one that sits on a seam where the texture is mirrored
/// keeps the tangent of the last triangle to use it. Exporters split points
/// along UV seams, so this is rare, and it keeps meshes small.
fn generate_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uv: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    let mut input = TangentInput {
        positions,
        normals,
        uv,
        indices,
        // What a point keeps if no triangle gives it a tangent.
        tangents: vec![[1.0, 0.0, 0.0, 1.0]; positions.len()],
    };
    if !indices.is_empty() {
        // The generator has no way to fail (its error type has no values).
        let _: Result<(), _> = bevy_mikktspace::generate_tangents(&mut input);
    }
    input.tangents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_and_fans_become_triangle_lists() {
        assert_eq!(
            triangle_list(Mode::TriangleStrip, &[0, 1, 2, 3, 4]),
            [0, 1, 2, 2, 1, 3, 2, 3, 4]
        );
        // A repeated point is a jump, not a triangle.
        assert_eq!(
            triangle_list(Mode::TriangleStrip, &[0, 1, 2, 2, 3]),
            [0, 1, 2]
        );
        assert_eq!(
            triangle_list(Mode::TriangleFan, &[0, 1, 2, 3]),
            [0, 1, 2, 0, 2, 3]
        );
        assert_eq!(triangle_list(Mode::TriangleFan, &[]), [0u32; 0]);
        assert_eq!(triangle_list(Mode::Triangles, &[0, 1, 2, 3]), [0, 1, 2]);
    }

    #[test]
    fn flat_normals_face_out_of_an_anticlockwise_triangle() {
        let triangle = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        assert_eq!(flat_normals(&triangle), [[0.0, 0.0, 1.0]; 3]);
        // No area: no panic, and still a usable direction.
        assert_eq!(flat_normals(&[[1.0; 3]; 3]), [[0.0, 1.0, 0.0]; 3]);
    }

    #[test]
    fn tangents_follow_the_texture() {
        let positions = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let normals = [[0.0, 0.0, 1.0]; 3];
        let uv = [[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]];
        let tangents = generate_tangents(&positions, &normals, &uv, &[0, 1, 2]);
        for tangent in tangents {
            assert!((tangent[0] - 1.0).abs() < 1e-5, "{tangent:?}");
            assert!(tangent[1].abs() < 1e-5 && tangent[2].abs() < 1e-5);
            assert!((tangent[3].abs() - 1.0).abs() < 1e-5);
        }
        // Nothing to work from: nothing to do, and no panic.
        assert!(generate_tangents(&[], &[], &[], &[]).is_empty());
    }
}
