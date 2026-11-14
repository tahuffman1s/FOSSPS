//! Once built, a UI must not touch the heap: not to update, not to draw,
//! and not to change HUD text to something no longer than before.

#![allow(clippy::unwrap_used, reason = "tests")]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use fossps_ui::{Anchor, Color, Draw, Ui, UiInput};

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
fn a_built_ui_never_allocates() {
    let mut ui = Ui::default();
    let ammo = ui.hud_text(Anchor::BottomRight, [0.0, 0.0], 32.0, "30 / 90");
    let health = ui.hud_bar(
        Anchor::BottomLeft,
        [0.0, 0.0],
        [300.0, 20.0],
        Color::WHITE,
        1.0,
    );
    ui.hud_rect(Anchor::Center, [0.0, 0.0], [4.0, 4.0], Color::WHITE);

    let settings = ui
        .add_screen("Settings", |screen| {
            screen.tabs(&["Game", "Sound", "Controls"], |screen, tab| {
                screen.toggle("Hints", tab == 0);
                screen.slider("Volume", 0.0, 1.0, 0.05, 0.5);
                screen.choice("Quality", &["Low", "Medium", "High"], 1);
            });
            screen.row(|screen| {
                screen.button("Apply");
                screen.button("Cancel");
            });
        })
        .unwrap();
    let loadout = ui
        .add_screen("Loadout", |screen| {
            screen.grid(3, |screen| {
                for name in ["Rifle", "Pistol", "Shotgun", "Rocket", "Grenade", "Knife"] {
                    screen.button(name);
                }
            });
        })
        .unwrap();
    let pause = ui
        .add_screen("Paused", |screen| {
            screen.button("Resume");
            screen.button("Settings");
            screen.button("Quit");
        })
        .unwrap();

    let ammo_counts = ["30 / 90", "9 / 90", "0 / 0"];
    let mut drawn = 0_usize;
    let mut events = 0_usize;

    let before = ALLOCATIONS.load(Ordering::Relaxed);
    for step in 0..2_000_u16 {
        let phase = f32::from(step % 100) / 100.0;
        let input = UiInput {
            direction: [(phase * 9.0).sin(), (phase * 5.0).cos()],
            confirm: step % 7 < 2,
            back: step % 211 == 0,
            previous_tab: step % 13 == 0,
            next_tab: step % 5 == 0,
            pointer: (step % 3 != 0).then_some([phase * 1920.0, 1080.0 - phase * 1080.0]),
            pointer_down: step % 4 < 2,
        };
        match step % 50 {
            0 => ui.open(pause).unwrap(),
            10 => ui.open(settings).unwrap(),
            25 => ui.open(loadout).unwrap(),
            40 => ui.close_all(),
            _ => {}
        }
        ui.set_text(ammo, ammo_counts[usize::from(step % 3)])
            .unwrap();
        ui.set_bar(health, phase).unwrap();

        let size = if step % 2 == 0 {
            [1920, 1080]
        } else {
            [1280, 800]
        };
        events += ui.update(&input, 1.0 / 60.0, size).len();
        for draw in ui.draw() {
            drawn += usize::from(matches!(draw, Draw::Text { .. }));
        }
    }
    let after = ALLOCATIONS.load(Ordering::Relaxed);

    assert!(
        events > 100,
        "the loop should have pressed and changed things: {events}"
    );
    assert!(drawn > 10_000, "the loop should have drawn menus: {drawn}");
    assert_eq!(after - before, 0, "updating or drawing the UI allocated");
}
