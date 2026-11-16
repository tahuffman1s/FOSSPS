//! Placing, moving and turning what is in the map, in the picture.
//!
//! With the mouse: a click on a mark chooses it, a drag moves it over the
//! ground (with Alt held, up and down), and a drag of the tip of its line
//! turns it. With "Click in the picture to place" on, a click on the ground
//! places a new one there.
//!
//! With keys or a gamepad, things go where the camera looks: one button
//! places, one picks up what the camera looks at (or what is chosen) to
//! carry it there and puts it down again, one puts it back where it was,
//! and two turn it.
//!
//! Everything lands on a grid and turns in steps unless snapping is off, or
//! Ctrl is held while dragging.

use super::{FOV_DEGREES, FlyCamera, MARK_FACING, PLACE_AHEAD, PLACE_WITHIN};
use crate::map::{self, Map};

/// The grids things can snap to, in metres between lines.
const GRIDS: [f32; 4] = [0.25, 0.5, 1.0, 2.0];
/// Which of them is used to begin with: half a metre.
const FIRST_GRID: usize = 1;
/// How far a turn snaps, in degrees.
const TURN: f32 = 15.0;
/// What positions and turns are rounded to with snapping off: a centimetre
/// and a degree.
const FINE: f32 = 0.01;
const FINE_TURN: f32 = 1.0;
/// A click this many `egui` points from a mark, or from the tip of the
/// chosen one's line, is a click on it.
const REACH: f32 = 12.0;
/// What the camera looks at, for picking up: the placed thing nearest the
/// middle of the view, within this many radians of it.
const AIM: f32 = 0.07;

/// What positions and turns snap to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Snap {
    /// Which of [`GRIDS`] is used; one past the last is off.
    grid: usize,
}

impl Default for Snap {
    fn default() -> Self {
        Self { grid: FIRST_GRID }
    }
}

impl Snap {
    /// The next choice: each grid from fine to coarse, then off, then round
    /// again.
    pub(crate) fn next(self) -> Self {
        Self {
            grid: (self.grid + 1) % (GRIDS.len() + 1),
        }
    }

    /// The grid in use, in metres, unless snapping is off.
    fn grid(self) -> Option<f32> {
        GRIDS.get(self.grid).copied()
    }

    /// `metres` on the grid; to the centimetre if snapping is off or `free`.
    pub(crate) fn metres(self, metres: f32, free: bool) -> f32 {
        if !metres.is_finite() {
            return 0.0;
        }
        let step = match self.grid() {
            Some(grid) if !free => grid,
            _ => FINE,
        };
        // Tidied to the millimetre, so 0.3 is not 0.30000001, and never -0.
        let snapped = (metres / step).round() * step;
        ((snapped * 1000.0).round() / 1000.0 + 0.0).clamp(-map::FURTHEST, map::FURTHEST)
    }

    /// How far one press turns something, in degrees.
    pub(crate) fn turn(self, free: bool) -> f32 {
        if self.grid().is_some() && !free {
            TURN
        } else {
            FINE_TURN
        }
    }

    /// `degrees` on a step of a turn, from -180 (not included) to 180.
    pub(crate) fn degrees(self, degrees: f32, free: bool) -> f32 {
        if !degrees.is_finite() {
            return 0.0;
        }
        let step = self.turn(free);
        map::wrapped((degrees / step).round() * step) + 0.0
    }

    /// The choice in words, as its button says it.
    pub(crate) fn words(self) -> String {
        match self.grid() {
            Some(grid) => format!("Snap: {grid} m, {TURN}°"),
            None => "Snap: off".to_owned(),
        }
    }
}

/// What a click or the start of a drag in the picture landed on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Hit {
    /// The mark of the placed thing with this ID.
    Mark(String),
    /// The tip of the chosen thing's line, which turns it.
    Tip(String),
}

/// What is being dragged in the picture.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Drag {
    /// The placed thing with this ID is moved. `offset` is how far it stood
    /// from the spot on the ground that was pressed, along X and Z, so that
    /// it does not jump to the pointer.
    Move { id: String, offset: [f32; 2] },
    /// The placed thing with this ID is turned to face the pointer.
    Turn { id: String },
}

/// Something picked up with a key or a gamepad, and where it was.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Held {
    pub(crate) id: String,
    pub(crate) from: [f32; 3],
}

/// Where the tip of the line showing which way `entity` faces is.
pub(crate) fn tip(entity: &map::Entity) -> [f32; 3] {
    let (sin, cos) = entity.facing.to_radians().sin_cos();
    [
        entity.position[0] - sin * MARK_FACING,
        entity.position[1],
        entity.position[2] - cos * MARK_FACING,
    ]
}

impl FlyCamera {
    /// Which way the point `at` in a picture `size` across and down is from
    /// the camera: the other way round from [`FlyCamera::pictured`]. Not of
    /// length one.
    fn ray(&self, at: [f32; 2], size: [f32; 2]) -> [f32; 3] {
        let [right, up, forward] = self.axes();
        let zoom = 1.0 / (FOV_DEGREES.to_radians() / 2.0).tan();
        let half = (size[1] / 2.0).max(1.0);
        let across = (at[0] - size[0] / 2.0) / (zoom * half);
        let above = (size[1] / 2.0 - at[1]) / (zoom * half);
        [0, 1, 2].map(|axis| forward[axis] + right[axis] * across + up[axis] * above)
    }

    /// Where a line from the camera along `ray` meets the level ground at
    /// `height`, if it does within reach.
    fn ground(&self, ray: [f32; 3], height: f32) -> Option<[f32; 3]> {
        let reach = (height - self.position[1]) / ray[1];
        let length = ray.iter().map(|part| part * part).sum::<f32>().sqrt();
        if !(reach.is_finite() && reach > 0.0 && reach * length <= PLACE_WITHIN) {
            return None;
        }
        Some([
            self.position[0] + ray[0] * reach,
            height,
            self.position[2] + ray[2] * reach,
        ])
    }

    /// Where on the ground at `height` the point `at` of the picture is.
    pub(crate) fn pointed(&self, at: [f32; 2], size: [f32; 2], height: f32) -> Option<[f32; 3]> {
        self.ground(self.ray(at, size), height)
    }

    /// How high the point `at` of the picture is above `place`: where the
    /// line through it meets the upright wall through `place` that faces
    /// the camera.
    fn pointed_height(&self, at: [f32; 2], size: [f32; 2], place: [f32; 3]) -> Option<f32> {
        let ray = self.ray(at, size);
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        // The way the camera faces, flattened.
        let flat = [-sin_yaw, -cos_yaw];
        let towards = ray[0] * flat[0] + ray[2] * flat[1];
        let away =
            (place[0] - self.position[0]) * flat[0] + (place[2] - self.position[2]) * flat[1];
        let reach = away / towards;
        (reach.is_finite() && reach > 0.0 && towards > 0.01)
            .then(|| self.position[1] + ray[1] * reach)
    }

    /// Where the next thing is placed and how far it is turned to the left,
    /// in degrees: on the ground at `height` where the camera looks, or a
    /// little ahead of the camera if it looks at the sky or far away;
    /// snapped, and facing the way the camera does.
    pub(crate) fn place_spot(&self, height: f32, snap: Snap) -> ([f32; 3], f32) {
        let [_, _, forward] = self.axes();
        let [x, _, z] = self.ground(forward, height).unwrap_or_else(|| {
            let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
            [
                self.position[0] - sin_yaw * PLACE_AHEAD,
                height,
                self.position[2] - cos_yaw * PLACE_AHEAD,
            ]
        });
        (
            [snap.metres(x, false), height, snap.metres(z, false)],
            snap.degrees(self.yaw.to_degrees(), false),
        )
    }

    /// The placed thing the camera looks most nearly straight at, if it
    /// looks at one.
    pub(crate) fn aimed_at<'map>(&self, map: &'map Map) -> Option<&'map str> {
        let [_, _, forward] = self.axes();
        let mut nearest = None;
        for entity in map.entities() {
            let away = [0, 1, 2].map(|axis| entity.position[axis] - self.position[axis]);
            let distance = away.iter().map(|part| part * part).sum::<f32>().sqrt();
            let ahead: f32 = (0..3).map(|axis| away[axis] * forward[axis]).sum();
            if !(distance.is_finite() && distance > 0.05) {
                continue;
            }
            let off = (ahead / distance).clamp(-1.0, 1.0).acos();
            if off <= AIM && nearest.is_none_or(|(least, _)| off < least) {
                nearest = Some((off, entity.id.as_str()));
            }
        }
        nearest.map(|(_, id)| id)
    }

    /// What is at the point `at` of a picture `size` across and down: the
    /// tip of the line of the `chosen` thing, or the nearest mark.
    pub(crate) fn hit(
        &self,
        at: [f32; 2],
        size: [f32; 2],
        map: &Map,
        chosen: Option<&str>,
    ) -> Option<Hit> {
        let off = |point: [f32; 3]| {
            self.pictured(point, size)
                .map(|(seen, _)| (seen[0] - at[0]).hypot(seen[1] - at[1]))
                .filter(|off| *off <= REACH)
        };
        let mut nearest: Option<(f32, Hit)> = None;
        for entity in map.entities() {
            // A mark comes before a tip as near: seen end on, the tip is
            // over its own mark, and the thing can still be moved.
            if let Some(off) = off(entity.position)
                && nearest.as_ref().is_none_or(|(least, _)| off < *least)
            {
                nearest = Some((off, Hit::Mark(entity.id.clone())));
            }
        }
        if let Some(entity) = chosen.and_then(|id| map.get(id))
            && let Some(off) = off(tip(entity))
            && nearest.as_ref().is_none_or(|(least, _)| off < *least)
        {
            nearest = Some((off, Hit::Tip(entity.id.clone())));
        }
        nearest.map(|(_, hit)| hit)
    }
}

/// How far the placed thing with this `id` stands from the spot of the
/// picture that was pressed to drag it, along X and Z.
pub(crate) fn drag_offset(
    camera: &FlyCamera,
    at: [f32; 2],
    size: [f32; 2],
    map: &Map,
    id: &str,
) -> [f32; 2] {
    map.get(id)
        .and_then(|entity| {
            let [x, _, z] = camera.pointed(at, size, entity.position[1])?;
            Some([entity.position[0] - x, entity.position[2] - z])
        })
        .unwrap_or_default()
}

/// How a drag is to be read: what the keys held with it ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Keys {
    /// Off the grid, while Ctrl is held.
    pub(crate) free: bool,
    /// Up and down and not over the ground, while Alt is held.
    pub(crate) lift: bool,
}

/// Carries on `drag` with the pointer at `at` in a picture `size` across
/// and down. Says whether the map was changed.
pub(crate) fn drag_to(
    drag: &Drag,
    camera: &FlyCamera,
    at: [f32; 2],
    size: [f32; 2],
    map: &mut Map,
    snap: Snap,
    keys: Keys,
) -> bool {
    match drag {
        Drag::Move { id, offset } => {
            let Some(entity) = map.get_mut(id) else {
                return false;
            };
            let mut to = entity.position;
            if keys.lift {
                if let Some(height) = camera.pointed_height(at, size, entity.position) {
                    to[1] = snap.metres(height, keys.free);
                }
            } else if let Some([x, _, z]) = camera.pointed(at, size, entity.position[1]) {
                to[0] = snap.metres(x + offset[0], keys.free);
                to[2] = snap.metres(z + offset[1], keys.free);
            }
            let changed = to != entity.position;
            entity.position = to;
            changed
        }
        Drag::Turn { id } => {
            let Some(entity) = map.get_mut(id) else {
                return false;
            };
            let Some([x, _, z]) = camera.pointed(at, size, entity.position[1]) else {
                return false;
            };
            let (across, along) = (x - entity.position[0], z - entity.position[2]);
            // Pointing right at where it stands is no direction.
            if across.hypot(along) < 0.05 {
                return false;
            }
            // Facing 0 looks along -Z, and a quarter turn left along -X.
            let facing = snap.degrees((-across).atan2(-along).to_degrees(), keys.free);
            let changed = facing != entity.facing;
            entity.facing = facing;
            changed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> FlyCamera {
        FlyCamera {
            position: [0.0, 5.0, 22.0],
            yaw: 0.0,
            pitch: -0.2,
            speed: 8.0,
        }
    }

    const SIZE: [f32; 2] = [1280.0, 720.0];

    #[test]
    fn snapping_goes_round_the_grids_and_off() {
        let mut snap = Snap::default();
        assert_eq!(snap.words(), "Snap: 0.5 m, 15°");
        assert_eq!(snap.metres(1.26, false), 1.5);
        assert_eq!(snap.metres(-0.2, false), 0.0);
        assert!(snap.metres(-0.2, false).is_sign_positive(), "never -0");
        assert_eq!(snap.degrees(100.0, false), 105.0);
        assert_eq!(snap.degrees(-176.0, false), 180.0);
        assert_eq!(snap.degrees(200.0, false), -165.0);
        // Ctrl held: to the centimetre and the degree.
        assert_eq!(snap.metres(1.257, true), 1.26);
        assert_eq!(snap.degrees(100.4, true), 100.0);

        let mut said = Vec::new();
        for _ in 0..5 {
            snap = snap.next();
            said.push(snap.words());
        }
        assert_eq!(
            said,
            [
                "Snap: 1 m, 15°",
                "Snap: 2 m, 15°",
                "Snap: off",
                "Snap: 0.25 m, 15°",
                "Snap: 0.5 m, 15°"
            ]
        );
        let off = Snap::default().next().next().next();
        assert_eq!(off.metres(0.3, false), 0.3);
        assert_eq!(off.metres(12.3456, false), 12.35);
        assert_eq!(off.turn(false), 1.0);
        assert_eq!(Snap::default().turn(false), 15.0);
        // What is not a number goes nowhere, and nothing leaves the map.
        assert_eq!(snap.metres(f32::NAN, false), 0.0);
        assert_eq!(snap.degrees(f32::INFINITY, false), 0.0);
        assert_eq!(snap.metres(1e9, false), map::FURTHEST);
    }

    #[test]
    fn a_point_of_the_picture_is_found_on_the_ground_and_back() {
        let camera = camera();
        // The middle of the picture is where the camera looks.
        let [x, y, z] = camera.pointed([640.0, 360.0], SIZE, 0.0).unwrap();
        assert!(x.abs() < 1e-3 && y == 0.0, "{x} {y}");
        assert!((z - (22.0 - 5.0 / 0.2_f32.tan())).abs() < 1e-3, "{z}");
        // Any spot on the ground is pictured where pointing finds it again.
        for spot in [[3.0, 0.0, 4.0], [-6.5, 0.0, -9.0], [0.5, 2.0, 10.0]] {
            let (at, _) = camera.pictured(spot, SIZE).unwrap();
            let found = camera.pointed(at, SIZE, spot[1]).unwrap();
            for axis in 0..3 {
                assert!((found[axis] - spot[axis]).abs() < 1e-3, "{found:?}");
            }
            // And as high above the ground below it as it is.
            let below = [spot[0], 0.0, spot[2]];
            let height = camera.pointed_height(at, SIZE, below).unwrap();
            assert!((height - spot[1]).abs() < 1e-3, "{height}");
        }
        // The sky is not on the ground, nor is ground too far to see well.
        assert_eq!(camera.pointed([640.0, 10.0], SIZE, 0.0), None);
        let level = FlyCamera {
            pitch: -0.01,
            ..camera
        };
        assert_eq!(level.pointed([640.0, 360.0], SIZE, 0.0), None);
    }

    #[test]
    fn things_are_placed_on_the_grid_where_the_camera_looks() {
        let camera = camera();
        // It looks at 0, 0, -2.67.
        assert_eq!(
            camera.place_spot(0.0, Snap::default()),
            ([0.0, 0.0, -2.5], 0.0)
        );
        let off = Snap::default().next().next().next();
        assert_eq!(camera.place_spot(0.0, off), ([0.0, 0.0, -2.67], 0.0));
        // Looking at the sky, things go a little ahead of the camera,
        // facing the way it does.
        let up = FlyCamera {
            position: [1.23, 3.0, 4.0],
            yaw: std::f32::consts::TAU / 4.0,
            pitch: 0.4,
            speed: 8.0,
        };
        assert_eq!(
            up.place_spot(0.0, Snap::default()),
            ([-4.0, 0.0, 4.0], 90.0)
        );
        // Looking down at the ground far away is the same.
        let far = FlyCamera { pitch: -0.02, ..up };
        assert_eq!(
            far.place_spot(0.0, Snap::default()),
            ([-4.0, 0.0, 4.0], 90.0)
        );
        // Something carried at a height stays at it.
        let ([_, y, z], _) = camera.place_spot(2.0, Snap::default());
        assert_eq!(y, 2.0);
        assert!(z > -2.5, "the ground is nearer up there: {z}");
    }

    fn map() -> Map {
        Map::parse(
            "map.toml",
            "version = 1\n\
             [[entity]]\nid = \"a\"\ntype = \"spawn\"\nposition = [0, 0, 0]\n\
             [[entity]]\nid = \"b\"\ntype = \"zone\"\nposition = [4, 0, 2]\nfacing = 90\n",
        )
        .unwrap()
    }

    #[test]
    fn a_click_finds_the_nearest_mark_and_the_tip_of_the_chosen_one() {
        let (camera, map) = (camera(), map());
        let seen = |point| camera.pictured(point, SIZE).unwrap().0;
        let a = seen([0.0; 3]);
        assert_eq!(
            camera.hit(a, SIZE, &map, None),
            Some(Hit::Mark("a".to_owned()))
        );
        assert_eq!(
            camera.hit([a[0] + 8.0, a[1] - 5.0], SIZE, &map, None),
            Some(Hit::Mark("a".to_owned()))
        );
        assert_eq!(camera.hit([a[0] + 30.0, a[1]], SIZE, &map, None), None);
        // The tip of b's line is a metre along -X from it, and counts only
        // while b is chosen.
        let tip_of_b = seen(tip(map.get("b").unwrap()));
        assert_eq!(seen([3.0, 0.0, 2.0]), tip_of_b);
        assert_eq!(camera.hit(tip_of_b, SIZE, &map, Some("a")), None);
        assert_eq!(
            camera.hit(tip_of_b, SIZE, &map, Some("b")),
            Some(Hit::Tip("b".to_owned()))
        );
        // The camera looks at the ground a little beyond a, and well to
        // the side of b; turned away, at neither.
        assert_eq!(camera.aimed_at(&map), Some("a"));
        let mut away = camera;
        away.yaw = 0.5;
        assert_eq!(away.aimed_at(&map), None);
    }

    #[test]
    fn a_drag_moves_lifts_and_turns_on_the_grid() {
        let (camera, mut map) = (camera(), map());
        let snap = Snap::default();
        let seen = |point| camera.pictured(point, SIZE).unwrap().0;
        let keys = Keys::default();

        // Pressed a little off its middle, it keeps that much off the
        // pointer: dragged to where 3.2, 0, 1.2 is, it lands on 3, 0, 1.
        let pressed = seen([0.2, 0.0, 0.2]);
        let offset = drag_offset(&camera, pressed, SIZE, &map, "a");
        assert!((offset[0] + 0.2).abs() < 1e-3 && (offset[1] + 0.2).abs() < 1e-3);
        let drag = Drag::Move {
            id: "a".to_owned(),
            offset,
        };
        let to = seen([3.2, 0.0, 1.2]);
        assert!(drag_to(&drag, &camera, to, SIZE, &mut map, snap, keys));
        assert_eq!(map.get("a").unwrap().position, [3.0, 0.0, 1.0]);
        // The same again changes nothing, so nothing is saved.
        assert!(!drag_to(&drag, &camera, to, SIZE, &mut map, snap, keys));
        // With Ctrl, off the grid.
        let free = Keys { free: true, ..keys };
        let to = seen([3.33, 0.0, 1.2]);
        assert!(drag_to(&drag, &camera, to, SIZE, &mut map, snap, free));
        assert_eq!(map.get("a").unwrap().position, [3.13, 0.0, 1.0]);
        // Dragged into the sky, it stays where it was.
        assert!(!drag_to(
            &drag,
            &camera,
            [640.0, 5.0],
            SIZE,
            &mut map,
            snap,
            keys
        ));

        // With Alt it goes up and down, and nowhere else.
        let lift = Keys { lift: true, ..keys };
        let to = seen([3.13, 1.6, 1.0]);
        assert!(drag_to(&drag, &camera, to, SIZE, &mut map, snap, lift));
        assert_eq!(map.get("a").unwrap().position, [3.13, 1.5, 1.0]);

        // The tip of b's line dragged to the far side of it (-Z) faces it
        // forwards; to +X of it, a quarter to the right.
        let turn = Drag::Turn { id: "b".to_owned() };
        let to = seen([4.1, 0.0, -1.0]);
        assert!(drag_to(&turn, &camera, to, SIZE, &mut map, snap, keys));
        assert_eq!(map.get("b").unwrap().facing, 0.0);
        let to = seen([7.0, 0.0, 2.3]);
        assert!(drag_to(&turn, &camera, to, SIZE, &mut map, snap, keys));
        assert_eq!(map.get("b").unwrap().facing, -90.0);
        assert!(drag_to(&turn, &camera, to, SIZE, &mut map, snap, free));
        assert_eq!(map.get("b").unwrap().facing, -96.0);
        // Pointing right at it is no direction, and something that was
        // removed meanwhile is not dragged.
        let to = seen([4.0, 0.0, 2.0]);
        assert!(!drag_to(&turn, &camera, to, SIZE, &mut map, snap, keys));
        map.remove("b");
        assert!(!drag_to(&turn, &camera, to, SIZE, &mut map, snap, keys));
    }
}
