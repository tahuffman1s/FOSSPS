//! Where the scene is seen from, and the arithmetic that puts it on screen.

/// A 4 by 4 matrix stored column by column, as `fossps_assets::Transform` is.
pub(crate) type Mat4 = [[f32; 4]; 4];

/// Where the scene is seen from.
///
/// Directions are glTF's: +Y is up, -Z is forward, +X is right, and one unit
/// is one metre.
///
/// ```
/// use fossps_render::Camera;
///
/// // Standing at the origin with eyes 1.7 m up, looking a little down.
/// let camera = Camera {
///     position: [0.0, 1.7, 0.0],
///     pitch: -0.2,
///     ..Camera::default()
/// };
/// assert_eq!(camera.fov_degrees, 74.0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Where the eye is, in metres.
    pub position: [f32; 3],
    /// Which way the eye faces, in radians: 0 looks along -Z, and it turns
    /// left as the number grows.
    pub yaw: f32,
    /// How far the eye looks up (positive) or down, in radians.
    pub pitch: f32,
    /// How much is seen from the bottom of the picture to the top, in
    /// degrees. Held between 1 and 179.
    pub fov_degrees: f32,
    /// Anything closer to the eye than this many metres is not drawn.
    pub near: f32,
}

impl Default for Camera {
    /// At the origin, looking along -Z, seeing 74 degrees from bottom to top.
    ///
    /// 74 degrees is what 90 degrees across a 4:3 display comes to, the usual
    /// default in shooters; on a 16:9 display it shows about 106 degrees across.
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            fov_degrees: 74.0,
            near: 0.05,
        }
    }
}

impl Camera {
    /// The matrix that takes a point in the world to where it is drawn, for
    /// a picture `width` by `height` pixels.
    ///
    /// In the result x runs right and y runs up, both from -1 to 1, which is
    /// what the WGSL shaders expect: the build script's compiler turns y
    /// over for Vulkan, where it runs down. Depth is 1 at `near` and falls
    /// towards 0 infinitely far away. That depth is what FSR's shaders are
    /// compiled for, and it keeps the most precision at long range.
    pub(crate) fn clip_from_world(&self, width: u32, height: u32) -> Mat4 {
        multiply(&self.projection(width, height), &self.view())
    }

    /// How much is seen from the bottom of the picture to the top, in radians.
    pub(crate) fn fov_radians(&self) -> f32 {
        finite_or(self.fov_degrees, 74.0)
            .clamp(1.0, 179.0)
            .to_radians()
    }

    /// The distance in metres closer than which nothing is drawn.
    pub(crate) fn near_metres(&self) -> f32 {
        finite_or(self.near, 0.05).max(0.001)
    }

    /// The matrix that takes a point in the world to where it is from the
    /// eye: x right, y up, and -z ahead.
    fn view(&self) -> Mat4 {
        let [yaw, pitch] = [self.yaw, self.pitch].map(|angle| finite_or(angle, 0.0));
        let position = self.position.map(|metres| finite_or(metres, 0.0));
        let (sin_yaw, cos_yaw) = yaw.sin_cos();
        let (sin_pitch, cos_pitch) = pitch.sin_cos();
        let right = [cos_yaw, 0.0, -sin_yaw];
        let forward = [-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch];
        let up = cross(right, forward);
        let mut view = [[0.0; 4]; 4];
        for axis in 0..3 {
            view[axis] = [right[axis], up[axis], -forward[axis], 0.0];
        }
        view[3] = [
            -dot(right, position),
            -dot(up, position),
            dot(forward, position),
            1.0,
        ];
        view
    }

    /// The matrix that takes a point as seen from the eye to where it is drawn.
    fn projection(&self, width: u32, height: u32) -> Mat4 {
        let near = self.near_metres();
        // Picture sizes are far below the point where f32 stops being exact.
        #[allow(clippy::cast_precision_loss)]
        let aspect = width.max(1) as f32 / height.max(1) as f32;
        let zoom = 1.0 / (self.fov_radians() / 2.0).tan();
        [
            [zoom / aspect, 0.0, 0.0, 0.0],
            [0.0, zoom, 0.0, 0.0],
            [0.0, 0.0, 0.0, -1.0],
            [0.0, 0.0, near, 0.0],
        ]
    }
}

/// `value`, or `fallback` if it is infinite or not a number.
fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// The matrix that applies `inner` first and `outer` second.
pub(crate) fn multiply(outer: &Mat4, inner: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (column, out_column) in out.iter_mut().enumerate() {
        for (row, cell) in out_column.iter_mut().enumerate() {
            *cell = (0..4).map(|k| outer[k][row] * inner[column][k]).sum();
        }
    }
    out
}

/// `clip_from_world` with everything it draws moved `right` and `up`, where
/// the whole picture is 2 wide and 2 high.
///
/// The upscaler has the camera moved by a different part of a pixel each
/// frame, which is how it sees detail smaller than a pixel.
pub(crate) fn shifted(clip_from_world: &Mat4, right: f32, up: f32) -> Mat4 {
    let mut out = *clip_from_world;
    for column in &mut out {
        // A point is drawn at x / w, so moving it by `right` adds `right * w`.
        column[0] += right * column[3];
        column[1] += up * column[3];
    }
    out
}

/// The matrix that turns a shape's surface directions the way `transform`
/// turns the shape, as three columns padded to four numbers each.
///
/// A shape that is stretched more one way than another needs this rather
/// than `transform` itself, or its surfaces would lean. The directions come
/// out at any length; the shader makes them length one.
pub(crate) fn normal_matrix(transform: &Mat4) -> [[f32; 4]; 3] {
    let axis = |column: usize| {
        let [x, y, z, _] = transform[column];
        [x, y, z]
    };
    let [x, y, z] = [axis(0), axis(1), axis(2)];
    // A mirrored shape would otherwise have its surfaces facing inwards.
    let sign = if dot(cross(x, y), z) < 0.0 { -1.0 } else { 1.0 };
    [cross(y, z), cross(z, x), cross(x, y)].map(|[a, b, c]| [a * sign, b * sign, c * sign, 0.0])
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use std::f32::consts::FRAC_PI_2;

    use super::*;

    /// Where `point` is drawn by `camera` in a square picture: x and y from
    /// -1 to 1, then depth, then whether it is in front of the eye.
    fn drawn(camera: &Camera, point: [f32; 3]) -> ([f32; 3], bool) {
        let matrix = camera.clip_from_world(100, 100);
        let [x, y, z] = point;
        let input = [x, y, z, 1.0];
        let clip: [f32; 4] = std::array::from_fn(|row| {
            (0..4)
                .map(|column| matrix[column][row] * input[column])
                .sum()
        });
        (
            [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]],
            clip[3] > 0.0,
        )
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn straight_ahead_is_the_middle_of_the_picture() {
        let ([x, y, _], in_front) = drawn(&Camera::default(), [0.0, 0.0, -5.0]);
        assert!(in_front);
        assert!(close(x, 0.0) && close(y, 0.0));
        let (_, in_front) = drawn(&Camera::default(), [0.0, 0.0, 5.0]);
        assert!(!in_front, "a point behind the eye is not in front of it");
    }

    #[test]
    fn right_is_right_and_up_is_towards_the_top() {
        let ([x, y, _], _) = drawn(&Camera::default(), [1.0, 0.0, -5.0]);
        assert!(x > 0.0 && close(y, 0.0));
        let ([x, y, _], _) = drawn(&Camera::default(), [0.0, 1.0, -5.0]);
        assert!(close(x, 0.0) && y > 0.0);
    }

    #[test]
    fn depth_is_one_at_the_near_distance_and_falls_towards_zero() {
        let camera = Camera::default();
        let ([.., at_near], _) = drawn(&camera, [0.0, 0.0, -camera.near]);
        let ([.., close_by], _) = drawn(&camera, [0.0, 0.0, -2.0]);
        let ([.., far_off], _) = drawn(&camera, [0.0, 0.0, -5000.0]);
        assert!(close(at_near, 1.0));
        assert!(close_by < at_near && far_off < close_by && far_off > 0.0);
    }

    #[test]
    fn the_field_of_view_is_measured_from_bottom_to_top() {
        let camera = Camera {
            fov_degrees: 90.0,
            ..Camera::default()
        };
        // 45 degrees up from straight ahead is the top edge.
        let ([_, y, _], _) = drawn(&camera, [0.0, 3.0, -3.0]);
        assert!(close(y, 1.0));
        // A picture twice as wide as it is high shows more to the sides, not less up and down.
        let wide = camera.clip_from_world(200, 100);
        let square = camera.clip_from_world(100, 100);
        assert!(close(wide[1][1], square[1][1]));
        assert!(close(wide[0][0] * 2.0, square[0][0]));
    }

    #[test]
    fn yaw_turns_left_and_pitch_looks_up() {
        let left = Camera {
            yaw: FRAC_PI_2,
            ..Camera::default()
        };
        let ([x, y, _], in_front) = drawn(&left, [-5.0, 0.0, 0.0]);
        assert!(in_front && close(x, 0.0) && close(y, 0.0));

        let up = Camera {
            pitch: FRAC_PI_2,
            ..Camera::default()
        };
        let ([x, y, _], in_front) = drawn(&up, [0.0, 5.0, 0.0]);
        assert!(in_front && close(x, 0.0) && close(y, 0.0));

        // Turned and tilted together: the walking direction of the physics
        // (-sin yaw, 0, -cos yaw), raised by the pitch.
        let both = Camera {
            yaw: 0.7,
            pitch: 0.3,
            ..Camera::default()
        };
        let ahead = [
            -0.7_f32.sin() * 0.3_f32.cos(),
            0.3_f32.sin(),
            -0.7_f32.cos() * 0.3_f32.cos(),
        ];
        let ([x, y, _], in_front) = drawn(&both, ahead);
        assert!(in_front && close(x, 0.0) && close(y, 0.0));
    }

    #[test]
    fn a_shifted_picture_moves_every_point_by_the_same_amount() {
        let camera = Camera::default();
        let plain = camera.clip_from_world(100, 100);
        let moved = shifted(&plain, 0.02, -0.01);
        for point in [[0.0, 0.0, -5.0, 1.0], [3.0, -1.0, -40.0, 1.0_f32]] {
            let through = |matrix: &Mat4| -> [f32; 2] {
                let clip: [f32; 4] = std::array::from_fn(|row| {
                    (0..4)
                        .map(|column| matrix[column][row] * point[column])
                        .sum()
                });
                [clip[0] / clip[3], clip[1] / clip[3]]
            };
            let [x, y] = through(&plain);
            let [moved_x, moved_y] = through(&moved);
            assert!(close(moved_x - x, 0.02) && close(moved_y - y, -0.01));
        }
    }

    #[test]
    fn the_eye_moves_with_the_camera() {
        let camera = Camera {
            position: [10.0, 2.0, -4.0],
            ..Camera::default()
        };
        let ([x, y, _], in_front) = drawn(&camera, [10.0, 2.0, -9.0]);
        assert!(in_front && close(x, 0.0) && close(y, 0.0));
    }

    #[test]
    fn numbers_that_are_not_numbers_do_not_spoil_the_picture() {
        let camera = Camera {
            position: [f32::NAN, 0.0, 0.0],
            yaw: f32::INFINITY,
            pitch: f32::NAN,
            fov_degrees: f32::NAN,
            near: -1.0,
        };
        let matrix = camera.clip_from_world(0, 0);
        assert!(matrix.iter().flatten().all(|cell| cell.is_finite()));
    }

    #[test]
    fn surfaces_of_a_stretched_or_mirrored_shape_still_face_outwards() {
        // Ten times as wide: a surface leaning 45 degrees leans far less.
        let mut wide = [[0.0; 4]; 4];
        wide[0][0] = 10.0;
        wide[1][1] = 1.0;
        wide[2][2] = 1.0;
        wide[3][3] = 1.0;
        let columns = normal_matrix(&wide);
        let turned = |normal: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|row| (0..3).map(|c| columns[c][row] * normal[c]).sum())
        };
        let [x, y, _] = turned([1.0, 1.0, 0.0]);
        assert!(close(y / x, 10.0));

        let mut mirrored = wide;
        mirrored[0][0] = -1.0;
        let columns = normal_matrix(&mirrored);
        let turned = |normal: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|row| (0..3).map(|c| columns[c][row] * normal[c]).sum())
        };
        assert!(turned([1.0, 0.0, 0.0])[0] < 0.0, "the +X side now faces -X");
        assert!(turned([0.0, 1.0, 0.0])[1] > 0.0, "the top still faces up");
    }
}
