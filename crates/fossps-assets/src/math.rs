//! The little vector and matrix arithmetic the loader needs.

use crate::scene::Transform;

/// The transform that moves nothing.
pub(crate) const IDENTITY: Transform = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// The transform that applies `inner` first and `outer` second.
pub(crate) fn combine(outer: &Transform, inner: &Transform) -> Transform {
    let mut out = [[0.0; 4]; 4];
    for (column, out_column) in out.iter_mut().enumerate() {
        for (row, cell) in out_column.iter_mut().enumerate() {
            *cell = (0..4).map(|k| outer[k][row] * inner[column][k]).sum();
        }
    }
    out
}

/// Whether `transform` turns shapes inside out, as a negative scale does.
pub(crate) fn mirrors(transform: &Transform) -> bool {
    let [x, y, z] = [axis(transform, 0), axis(transform, 1), axis(transform, 2)];
    dot(cross(x, y), z) < 0.0
}

/// One of the three direction columns of `transform`.
fn axis(transform: &Transform, column: usize) -> [f32; 3] {
    let [x, y, z, _] = transform[column];
    [x, y, z]
}

/// Where `transform` puts the origin.
pub(crate) fn position(transform: &Transform) -> [f32; 3] {
    axis(transform, 3)
}

/// Which way `transform` points something that faces forward (-Z).
pub(crate) fn forward(transform: &Transform) -> [f32; 3] {
    let [x, y, z] = axis(transform, 2);
    normalized([-x, -y, -z]).unwrap_or([0.0, 0.0, -1.0])
}

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// `v` scaled to length one, or `None` if it has no direction.
pub(crate) fn normalized(v: [f32; 3]) -> Option<[f32; 3]> {
    let length = dot(v, v).sqrt();
    (length.is_finite() && length > 0.0).then(|| [v[0] / length, v[1] / length, v[2] / length])
}
