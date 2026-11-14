//! A tick of netcode must not touch the heap: it runs for every player on
//! every tick, on the server and on every client.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use fossps_net::{Client, ClientSettings, PlayerInput, Server, ServerSettings};
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
fn a_tick_of_netcode_does_not_allocate() {
    let s = 100.0;
    let level = Level::from_triangles(&[
        [[-s, 0.0, s], [s, 0.0, s], [s, 0.0, -s]],
        [[-s, 0.0, s], [s, 0.0, -s], [-s, 0.0, -s]],
    ]);
    let rules = MoveSettings::default();
    let step = 1.0 / 60.0;
    let mut server = Server::new(step, ServerSettings::default());
    let mut clients = Vec::new();
    for seat in 0..8_u8 {
        let start = Player::at([f32::from(seat) * 2.0, 0.5, 0.0], &rules);
        let id = server.join(start).unwrap();
        clients.push((id, Client::new(start, step, ClientSettings::default())));
    }
    let mut commands = Vec::new();
    let mut snapshot = Vec::new();

    let mut corrected = 0;
    let mut round = |tick: u32, server: &mut Server, corrected: &mut u32| {
        for (id, client) in &mut clients {
            let input = PlayerInput {
                movement: MoveInput {
                    forward: 1.0,
                    right: 0.3,
                    jump: tick.is_multiple_of(40),
                    ..MoveInput::default()
                },
                ..PlayerInput::default()
            };
            client.predict(&level, &rules, &input, &mut commands);
            server.receive(*id, &commands).unwrap();
        }
        // Now and then the server moves someone, so clients replay.
        if tick.is_multiple_of(25) {
            server.player_mut(3).unwrap().position[0] += 0.2;
        }
        server.tick(&level, &rules);
        for (id, client) in &mut clients {
            server.write_snapshot(*id, &mut snapshot).unwrap();
            let received = client.receive(&level, &rules, &snapshot).unwrap();
            if matches!(received, fossps_net::Received::Corrected { .. }) {
                *corrected += 1;
            }
            assert_eq!(client.others().count(), 7);
        }
    };
    // The first packets size the two buffers.
    for tick in 0..10 {
        round(tick, &mut server, &mut corrected);
    }

    let before = ALLOCATIONS.load(Ordering::Relaxed);
    corrected = 0;
    for tick in 10..310 {
        round(tick, &mut server, &mut corrected);
    }
    let allocated = ALLOCATIONS.load(Ordering::Relaxed) - before;

    assert!(corrected >= 12, "{corrected} corrections replayed");
    assert_eq!(allocated, 0, "allocations in 300 ticks");
}
