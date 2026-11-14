//! Sounds that come with the engine, so a new game is not silent.
//!
//! They are made from arithmetic when asked for, not read from files, and
//! come out the same every time. They are stand-ins: swap in
//! recorded sounds with [`Sound::load`] when the game has its own.

use std::f32::consts::TAU;

use crate::vary::scramble;
use crate::{Sound, SoundSet};

/// Samples a second of every starter sound.
const RATE: u32 = 48_000;

/// Hiss: a new unrelated number between -1 and 1 each time.
struct Hiss(u32);

impl Hiss {
    fn next(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9);
        (scramble(self.0) >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
    }
}

/// Builds a mono sound `seconds` long from `sample`, which is given the
/// time in seconds.
fn build(seconds: f32, mut sample: impl FnMut(f32) -> f32) -> Sound {
    let frames = (seconds * RATE as f32) as usize;
    let samples: Vec<f32> = (0..frames.max(1))
        .map(|frame| {
            let time = frame as f32 / RATE as f32;
            // Ease in and out of silence so neither end clicks.
            let starting = (time / 0.0005).min(1.0);
            let ending = ((seconds - time) / 0.004).clamp(0.0, 1.0);
            sample(time) * starting * ending
        })
        .collect();
    Sound::mono(RATE, samples)
}

/// Hiss with the high part taken off: `keep` is how much of each new
/// sample gets through, from 0 (nothing) to 1 (all of it).
fn soft_hiss(seed: u32, keep: f32) -> impl FnMut() -> f32 {
    let mut hiss = Hiss(seed);
    let mut low = 0.0;
    move || {
        low += (hiss.next() - low) * keep;
        low
    }
}

/// One footstep on a hard floor. Each `take` is a little different.
fn footstep(take: u32) -> Sound {
    let mut scuff = soft_hiss(0xF007 + take, 0.18 + 0.03 * take as f32);
    let thump = 85.0 + 9.0 * take as f32;
    build(0.11, move |time| {
        let heel = (TAU * thump * time).sin() * (-time * 60.0).exp();
        let sole = scuff() * (-time * 45.0).exp() * 1.6;
        (heel * 0.55 + sole) * (time / 0.002).min(1.0) * 0.7
    })
}

/// One rifle shot. Each `take` is a little different.
fn gunshot(take: u32) -> Sound {
    let mut crack = Hiss(0x6055 + take);
    let mut body = soft_hiss(0xB0D1 + take, 0.12);
    let boom = 70.0 + 6.0 * take as f32;
    build(0.42, move |time| {
        let snap = crack.next() * (-time * 140.0).exp();
        let roar = body() * (-time * 16.0).exp() * 2.2;
        // A low note that drops as it dies away.
        let kick = (TAU * (boom * time + 900.0 * (1.0 - (-time * 30.0).exp()) / 30.0)).sin()
            * (-time * 22.0).exp();
        (snap * 0.8 + roar * 0.6 + kick * 0.7).clamp(-1.0, 1.0) * 0.9
    })
}

/// A short metal tick, `pitch` hertz, starting at `start` seconds.
fn tick(time: f32, start: f32, pitch: f32) -> f32 {
    let since = time - start;
    if since < 0.0 {
        return 0.0;
    }
    (TAU * pitch * since).sin() * (-since * 220.0).exp()
}

/// Four takes of a footstep, quiet and short-ranged.
#[must_use]
pub fn footsteps() -> SoundSet {
    set((0..4).map(footstep).collect()).spread(0.07, 0.15)
}

/// Four takes of a rifle shot.
#[must_use]
pub fn gunfire() -> SoundSet {
    set((0..4).map(gunshot).collect()).spread(0.03, 0.05)
}

/// The thud of landing from a jump or a fall.
#[must_use]
pub fn landing() -> Sound {
    let mut scuff = soft_hiss(0x1A4D, 0.1);
    build(0.2, move |time| {
        let thud = (TAU * 60.0 * time).sin() * (-time * 28.0).exp();
        (thud * 0.8 + scuff() * (-time * 30.0).exp() * 1.5) * (time / 0.003).min(1.0) * 0.8
    })
}

/// The click of a trigger pulled on an empty magazine.
#[must_use]
pub fn dry_fire() -> Sound {
    build(0.06, |time| {
        (tick(time, 0.0, 2_600.0) + tick(time, 0.012, 1_700.0) * 0.6) * 0.5
    })
}

/// A magazine coming out and a new one going in.
#[must_use]
pub fn reload() -> Sound {
    let mut slide = soft_hiss(0x4E10, 0.35);
    build(0.75, move |time| {
        let out = tick(time, 0.0, 1_300.0) + tick(time, 0.03, 900.0) * 0.7;
        let seat = tick(time, 0.45, 1_100.0) + tick(time, 0.47, 2_000.0) * 0.5;
        // The new magazine sliding home, just before it seats.
        let sliding = if (0.36..0.45).contains(&time) {
            slide() * 0.25
        } else {
            0.0
        };
        let rack = tick(time, 0.66, 1_500.0) + tick(time, 0.69, 2_300.0) * 0.8;
        (out + seat + sliding + rack) * 0.5
    })
}

fn set(takes: Vec<Sound>) -> SoundSet {
    SoundSet::of(takes)
}
