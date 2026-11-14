//! Choosing where a player comes into the world.

use crate::rounds::TeamId;

/// Closer than this to anyone, in metres, a spawn point is taken: the new
/// player would appear inside them.
const TAKEN_WITHIN: f32 = 1.0;

/// An enemy this far away, in metres, is far enough. Points beyond it all
/// count the same, so players are not always sent to the far corner.
const FAR_ENOUGH: f32 = 40.0;

/// The choice is made by chance among the points at least this good compared
/// with the best one, so someone watching one point cannot be sure who
/// appears there.
const NEARLY_AS_GOOD: f32 = 0.75;

/// A place in the map where players can come into the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpawnPoint {
    /// Where the player's feet go.
    pub position: [f32; 3],
    /// The team it belongs to, or `None` if anyone may use it.
    pub team: Option<TeamId>,
}

/// A living player, as the one about to spawn sees them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stander {
    /// Where they are standing.
    pub position: [f32; 3],
    /// Whether they are an enemy of the player about to spawn. With no
    /// teams, everyone is.
    pub enemy: bool,
}

/// Picks the spawn point for a player on `team`, as an index into `points`.
/// `None` only if there are no points at all.
///
/// The player gets one of their team's points (or a point for anyone); if the
/// map has none for them, any point will do. Points somebody is standing on
/// are left out while others are free. Of the rest, the further the nearest
/// enemy the better, up to 40 m, and the pick is made with `seed` among the
/// points at least three quarters as good as the best: pass something that differs each time, such as the player's
/// number mixed with the tick. The same arguments always give the same point.
///
/// Walls are not looked at: a point in plain view of an enemy 30 m away
/// counts as well as a hidden one.
#[must_use]
pub fn choose_spawn(
    points: &[SpawnPoint],
    team: Option<TeamId>,
    standers: &[Stander],
    seed: u32,
) -> Option<usize> {
    let allowed = |point: &SpawnPoint| point.team.is_none() || point.team == team;
    let any_allowed = points.iter().any(allowed);
    let free = |point: &SpawnPoint| {
        standers
            .iter()
            .all(|stander| distance(point.position, stander.position) >= TAKEN_WITHIN)
    };
    let any_free = points
        .iter()
        .any(|point| (allowed(point) || !any_allowed) && free(point));
    let usable =
        |point: &SpawnPoint| (allowed(point) || !any_allowed) && (free(point) || !any_free);
    let safety = |point: &SpawnPoint| {
        standers
            .iter()
            .filter(|stander| stander.enemy)
            .map(|stander| distance(point.position, stander.position))
            .fold(FAR_ENOUGH, f32::min)
    };

    let safest = points
        .iter()
        .filter(|point| usable(point))
        .map(safety)
        .fold(f32::NEG_INFINITY, f32::max);
    let good = |point: &SpawnPoint| usable(point) && safety(point) >= safest * NEARLY_AS_GOOD;
    let count = points.iter().filter(|point| good(point)).count();
    if count == 0 {
        return None;
    }
    let pick = mix(seed) as usize % count;
    points
        .iter()
        .enumerate()
        .filter(|(_, point)| good(point))
        .nth(pick)
        .map(|(index, _)| index)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let [x, y, z] = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (x * x + y * y + z * z).sqrt()
}

/// Scrambles a number so that close seeds give unrelated picks.
fn mix(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    x
}
