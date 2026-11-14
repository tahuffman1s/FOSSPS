//! Once made, a mixer must not touch the heap: not to start sounds, not to
//! cut them off, and not to mix. Footsteps and sound sets must not either.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use fossps_audio::{
    Feet, FootstepSettings, Footsteps, Importance, Listener, Mixer, Play, Range, starter,
};

/// Counts every allocation made by the process.
struct Counting;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to `System` unchanged, so its contract is upheld.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract for `layout`.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer` came from `System.alloc` with this same `layout`.
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// This file holds a single test on purpose: a second test running on another
// thread would add its own allocations to the count.
#[test]
fn playing_and_mixing_never_allocate() {
    let mut mixer = Mixer::with_voices(48_000, 8);
    let mut steps = starter::footsteps();
    let mut gunfire = starter::gunfire();
    let reload = starter::reload();
    let settings = FootstepSettings::default();
    let mut footsteps = Footsteps::default();
    let mut buffer = vec![0.0_f32; 800 * 2];
    let feet = Feet {
        speed: 6.5,
        grounded: true,
        sprinting: true,
        ..Feet::default()
    };

    let before = ALLOCATIONS.load(Ordering::Relaxed);
    let mut heard = 0.0_f32;
    for tick in 0..2_000_u32 {
        let turn = tick as f32 * 0.01;
        mixer.set_listener(Listener {
            position: [0.0, 1.6, 0.0],
            yaw: turn,
        });
        let enemy = [10.0 * turn.sin(), 0.0, -10.0 * turn.cos()];
        if footsteps.step(&settings, &feet, 1.0 / 60.0).is_some() {
            mixer.play(steps.at(enemy, tick).range(Range::FOOTSTEP));
        }
        // More gunfire than there are voices, so sounds are cut off too.
        if tick % 3 == 0 {
            mixer.play(gunfire.at(enemy, tick).range(Range::GUNFIRE));
            mixer.play(gunfire.everywhere(tick).importance(Importance::High));
        }
        if tick % 200 == 0 {
            let playing = mixer.play(Play::everywhere(reload.clone()).looping());
            mixer.move_to(playing, enemy);
            mixer.stop(playing);
        }
        mixer.mix(&mut buffer);
        heard = heard.max(buffer[0].abs());
    }
    let allocations = ALLOCATIONS.load(Ordering::Relaxed) - before;

    assert!(heard > 0.0, "the test should have mixed something");
    assert_eq!(allocations, 0);
}
