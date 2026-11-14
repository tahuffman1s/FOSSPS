//! Choosing where a direction press goes.

use crate::input::Direction;
use crate::node::{Node, Rect};

/// A box as seen when travelling in one direction: how far along the
/// way it starts and ends, and where it sits from side to side.
struct Seen {
    near: f32,
    far: f32,
    side_low: f32,
    side_high: f32,
}

fn seen(rect: Rect, direction: Direction) -> Seen {
    let (near, far, side_low, side_high) = match direction {
        Direction::Down => (rect.y, rect.bottom(), rect.x, rect.right()),
        Direction::Up => (-rect.bottom(), -rect.y, rect.x, rect.right()),
        Direction::Right => (rect.x, rect.right(), rect.y, rect.bottom()),
        Direction::Left => (-rect.right(), -rect.x, rect.y, rect.bottom()),
    };
    Seen {
        near,
        far,
        side_low,
        side_high,
    }
}

/// How costly it is to go from one box to another: lower is better.
///
/// Distance sideways costs double and lining up earns a discount, so a
/// widget straight ahead beats a nearer one off to the side. `ahead` is
/// how far along the way the target is.
fn cost(from: &Seen, to: &Seen, ahead: f32) -> f32 {
    let sideways = ((to.side_low + to.side_high) - (from.side_low + from.side_high)).abs() / 2.0;
    let lined_up = (from.side_high.min(to.side_high) - from.side_low.max(to.side_low)).max(0.0);
    ahead + 2.0 * sideways - lined_up
}

/// The widget a press of `direction` moves to from `from`, among the
/// nodes `first..end`.
///
/// Only widgets whose middle is past the edge of the current one count.
/// When there are none and `wrap` is on, the search starts again from
/// the opposite side of the screen, among the widgets behind this one. Equal costs go to the widget made
/// first. `None` means stay put.
pub(crate) fn next(
    nodes: &[Node],
    first: usize,
    end: usize,
    from: usize,
    direction: Direction,
    wrap: bool,
) -> Option<usize> {
    let here = seen(nodes[from].rect, direction);
    let mut best: Option<(usize, f32)> = None;
    let consider = |best: &mut Option<(usize, f32)>, index: usize, cost: f32| {
        if best.is_none_or(|(_, lowest)| cost < lowest) {
            *best = Some((index, cost));
        }
    };
    let candidates = || (first..end).filter(|&index| nodes[index].focusable());

    for index in candidates() {
        let there = seen(nodes[index].rect, direction);
        if index != from && (there.near + there.far) / 2.0 > here.far {
            consider(
                &mut best,
                index,
                cost(&here, &there, (there.near - here.far).max(0.0)),
            );
        }
    }
    if best.is_none() && wrap {
        // Round to the other side: only widgets behind this one count, so
        // pressing sideways in a plain column goes nowhere.
        for index in candidates() {
            let there = seen(nodes[index].rect, direction);
            if index != from && (there.near + there.far) / 2.0 < here.near {
                consider(&mut best, index, cost(&here, &there, there.near));
            }
        }
    }
    best.map(|(index, _)| index)
}
