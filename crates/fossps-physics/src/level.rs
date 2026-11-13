//! The level's triangles, and the few questions the player asks of them.

use fossps_assets::{Scene, Transform};
use parry3d::math::{Pose, Vector};
use parry3d::query::{self, Ray, RayCast, ShapeCastOptions};
use parry3d::shape::{Capsule, TriMesh};

/// A triangle thinner than this (twice its area, in square metres) is left
/// out: it has no direction to push the player in.
const THINNEST_TRIANGLE: f32 = 1.0e-10;

/// The solid shapes of a level, ready for the player to collide with.
///
/// Every triangle of every placed mesh is solid, from both sides. Build it
/// once when the level loads; it does not change afterwards.
#[derive(Clone)]
pub struct Level {
    /// `None` when there are no triangles, so nothing is ever hit.
    mesh: Option<TriMesh>,
    triangles: usize,
}

impl std::fmt::Debug for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Level")
            .field("triangles", &self.triangles)
            .finish()
    }
}

impl Level {
    /// The collision shapes of a loaded scene: every mesh, where it stands.
    ///
    /// A scene with no meshes gives an empty level, in which a player falls
    /// forever; check [`Level::triangle_count`] to warn about that.
    #[must_use]
    pub fn from_scene(scene: &Scene) -> Self {
        let mut triangles = Vec::new();
        for instance in &scene.instances {
            let Some(mesh) = scene.meshes.get(instance.mesh as usize) else {
                continue;
            };
            for part in &mesh.parts {
                // Both sides of a triangle are solid, so a mirrored
                // placement, which turns triangles inside out, needs no care.
                triangles.extend(
                    part.triangles()
                        .map(|corners| corners.map(|point| place(&instance.transform, point))),
                );
            }
        }
        Self::from_triangles(&triangles)
    }

    /// A level made of loose triangles, each three corner positions in metres.
    ///
    /// Triangles with no area, or with a corner that is not a number, are
    /// left out.
    #[must_use]
    pub fn from_triangles(triangles: &[[[f32; 3]; 3]]) -> Self {
        let mut points = Vec::with_capacity(triangles.len() * 3);
        let mut indices = Vec::with_capacity(triangles.len());
        for triangle in triangles {
            let [a, b, c] = triangle.map(Vector::from_array);
            let area_doubled = (b - a).cross(c - a).length();
            if !(area_doubled.is_finite() && area_doubled > THINNEST_TRIANGLE) {
                continue;
            }
            let Ok(first) = u32::try_from(points.len()) else {
                break;
            };
            points.extend([a, b, c]);
            indices.push([first, first + 1, first + 2]);
        }
        let count = indices.len();
        let mesh = TriMesh::new(points, indices).ok();
        Self {
            triangles: if mesh.is_some() { count } else { 0 },
            mesh,
        }
    }

    /// How many triangles the player can collide with.
    #[must_use]
    pub fn triangle_count(&self) -> usize {
        self.triangles
    }

    /// Moves `shape` from `centre` along `travel` and reports the first
    /// thing in its way, stopping `gap` short of it. Something less than
    /// `gap` past the end of `travel` is in the way too.
    ///
    /// Stopping short rather than touching matters: a shape that rests a
    /// hair away from a wall can slide along it without the wall being
    /// found again on every move, where its direction would be guesswork.
    pub(crate) fn sweep(
        &self,
        shape: &Capsule,
        centre: Vector,
        travel: Vector,
        gap: f32,
    ) -> Option<Touch> {
        let mesh = self.mesh.as_ref()?;
        let length = travel.length();
        let heading = travel.try_normalize()?;
        let options = ShapeCastOptions {
            max_time_of_impact: 1.0,
            target_distance: 0.0,
            // Something already touching is only in the way if we head into it.
            stop_at_penetration: false,
            compute_impact_geometry_on_penetration: true,
        };
        let hit = query::cast_shapes(
            &Pose::IDENTITY,
            Vector::ZERO,
            mesh,
            &Pose::from_translation(centre),
            heading * (length + gap),
            shape,
            options,
        )
        .ok()
        .flatten()?;
        let normal = hit.normal1.try_normalize()?;
        // Back along the path far enough to leave `gap` square to the surface.
        let head_on = (-heading.dot(normal)).max(1.0e-3);
        let reached = hit.time_of_impact.clamp(0.0, 1.0) * (length + gap);
        let fraction = ((reached - gap / head_on) / length).clamp(0.0, 1.0);
        (fraction.is_finite() && hit.witness1.is_finite()).then_some(Touch {
            fraction,
            normal,
            point: hit.witness1,
        })
    }

    /// Whether `shape` at `centre` reaches into anything solid.
    pub(crate) fn overlaps(&self, shape: &Capsule, centre: Vector) -> bool {
        self.mesh.as_ref().is_some_and(|mesh| {
            query::intersection_test(
                &Pose::IDENTITY,
                mesh,
                &Pose::from_translation(centre),
                shape,
            )
            .is_ok_and(|found| found.intersecting)
        })
    }

    /// The surface nearest to `shape` at `centre`, if it is closer than
    /// `gap`: which way it faces, and how far off it is (negative when the
    /// shape is inside it).
    pub(crate) fn nearest(
        &self,
        shape: &Capsule,
        centre: Vector,
        gap: f32,
    ) -> Option<(Vector, f32)> {
        let mesh = self.mesh.as_ref()?;
        let contact = query::contact(
            &Pose::IDENTITY,
            mesh,
            &Pose::from_translation(centre),
            shape,
            gap,
        )
        .ok()
        .flatten()?;
        let normal = contact.normal1.try_normalize()?;
        contact.dist.is_finite().then_some((normal, contact.dist))
    }

    /// Which way the surface straight below `from` faces, looking no
    /// further down than `reach`.
    pub(crate) fn surface_below(&self, from: Vector, reach: f32) -> Option<Vector> {
        let ray = Ray::new(from, Vector::NEG_Y);
        let hit = self
            .mesh
            .as_ref()?
            .cast_local_ray_and_get_normal(&ray, reach, false)?;
        // A triangle seen from behind reports its back; either side is floor.
        let normal = hit.normal.try_normalize()?;
        Some(if normal.y < 0.0 { -normal } else { normal })
    }
}

/// What a moving shape ran into.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Touch {
    /// How much of the move was made before touching, from 0 to 1.
    pub fraction: f32,
    /// Which way the surface faces, pointing at the shape.
    pub normal: Vector,
    /// The place on the surface that was touched.
    pub point: Vector,
}

/// `point` moved to where `transform` puts it.
fn place(transform: &Transform, point: [f32; 3]) -> [f32; 3] {
    let mut out = [transform[3][0], transform[3][1], transform[3][2]];
    for (column, value) in point.into_iter().enumerate() {
        for (row, cell) in out.iter_mut().enumerate() {
            *cell += transform[column][row] * value;
        }
    }
    out
}
