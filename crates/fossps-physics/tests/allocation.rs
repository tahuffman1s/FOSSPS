//! Moving a player must not touch the heap: it runs for every player on
//! every tick, and again for every tick a client replays.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use fossps_physics::{Level, MoveInput, MoveSettings, Player};

struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is passed straight to the system allocator, which
// upholds the contract; counting does not touch the memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller's layout is passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from `alloc` above with this layout.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

// The only test in this file, so nothing else allocates while it counts.
#[test]
fn stepping_a_player_does_not_allocate() {
    // A floor, a wall to slide along and a stair to climb.
    let quad = |a, b, c, d| [[a, b, c], [a, c, d]];
    let mut triangles = Vec::new();
    triangles.extend(quad(
        [-40.0, 0.0, 40.0],
        [40.0, 0.0, 40.0],
        [40.0, 0.0, -40.0],
        [-40.0, 0.0, -40.0],
    ));
    triangles.extend(quad(
        [2.0, 0.0, 40.0],
        [2.0, 0.0, -40.0],
        [2.0, 3.0, -40.0],
        [2.0, 3.0, 40.0],
    ));
    triangles.extend(quad(
        [-40.0, 0.0, -3.0],
        [40.0, 0.0, -3.0],
        [40.0, 0.3, -3.0],
        [-40.0, 0.3, -3.0],
    ));
    triangles.extend(quad(
        [-40.0, 0.3, -3.0],
        [40.0, 0.3, -3.0],
        [40.0, 0.3, -40.0],
        [-40.0, 0.3, -40.0],
    ));
    let level = Level::from_triangles(&triangles);
    let settings = MoveSettings::default();
    let mut player = Player::at([0.0, 0.5, 0.0], &settings);

    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for tick in 0..300_u32 {
        let input = MoveInput {
            forward: 1.0,
            right: 0.6,
            jump: tick.is_multiple_of(50),
            crouch: tick % 200 > 150,
            sprint: true,
            ..MoveInput::default()
        };
        player.step(&level, &settings, &input, 1.0 / 60.0);
    }
    let allocated = ALLOCATIONS.load(Ordering::Relaxed) - before;

    assert!(player.position[1] > 0.25, "climbed the stair: {player:?}");
    assert_eq!(allocated, 0, "allocations in 300 ticks");
}
