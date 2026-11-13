//! The per-tick input path must not touch the heap.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use fossps_input::{ActionMap, GamepadButton, Input, Key, MouseWheel, Stick, StickAxis};

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
fn feeding_events_and_ticking_never_allocates() {
    let map = ActionMap::default_fps();
    let fire = map.action("fire").unwrap();
    let mut input = Input::new(map);
    let mut shots = 0_u32;

    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for step in 0..1_000_u16 {
        let phase = f32::from(step % 100) / 100.0;
        input.press(Key::W);
        input.press(MouseWheel::MouseWheelUp);
        input.move_mouse(phase * 10.0, -phase);
        input.set_stick(Stick::RightStick, StickAxis::X, phase);
        input.set_button_amount(GamepadButton::RightTrigger, phase);
        if step % 7 == 0 {
            input.release(Key::W);
        }
        if step % 50 == 0 {
            input.release_all();
        }
        shots += u32::from(input.tick().just_pressed(fire));
    }
    let after = ALLOCATIONS.load(Ordering::Relaxed);

    assert!(shots > 0, "the loop should have exercised the fire action");
    assert_eq!(after - before, 0, "the input tick path allocated");
}
