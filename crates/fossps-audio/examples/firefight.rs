//! Eight seconds of a firefight, heard from where the player stands.
//!
//! An enemy sprints in a circle around the player, firing bursts. The
//! player fires back, runs dry, reloads, and lands from a jump.
//!
//! ```sh
//! cargo run -p fossps-audio --example firefight                 # through the speakers
//! cargo run -p fossps-audio --example firefight -- out.wav      # into a file
//! ```

use std::error::Error;

use fossps_audio::{
    Feet, Footfall, FootstepSettings, Footsteps, Importance, Listener, Mixer, Play, Range, Sound,
    SoundSet, starter,
};

const TICKS_PER_SECOND: u32 = 60;
const SECONDS: u32 = 8;
const RATE: u32 = 48_000;

/// Everything the scene plays, and who is where.
struct Scene {
    steps: SoundSet,
    gunfire: SoundSet,
    dry_fire: Sound,
    reload: Sound,
    landing: Sound,
    settings: FootstepSettings,
    enemy_feet: Footsteps,
}

impl Scene {
    fn new() -> Self {
        Self {
            steps: starter::footsteps(),
            gunfire: starter::gunfire(),
            dry_fire: starter::dry_fire(),
            reload: starter::reload(),
            landing: starter::landing(),
            settings: FootstepSettings::default(),
            enemy_feet: Footsteps::default(),
        }
    }

    /// Runs one tick and hands every sound it starts to `play`.
    fn tick(&mut self, tick: u32, mut play: impl FnMut(Play)) -> Listener {
        let step = 1.0 / TICKS_PER_SECOND as f32;
        let time = tick as f32 * step;
        let every = |ticks: u32, from: f32, to: f32| {
            (from..to).contains(&time) && tick.is_multiple_of(ticks)
        };
        let at = |seconds: f32| tick == (seconds * TICKS_PER_SECOND as f32) as u32;

        // The enemy runs a circle eight metres out, starting ahead.
        let angle = time * 0.8;
        let enemy = [8.0 * angle.sin(), 0.0, -8.0 * angle.cos()];
        let feet = Feet {
            speed: 6.4,
            grounded: true,
            sprinting: true,
            ..Feet::default()
        };
        if let Some(Footfall::Step { volume, .. }) =
            self.enemy_feet.step(&self.settings, &feet, step)
        {
            let sound = self.steps.at(enemy, tick);
            let volume = sound.volume * volume;
            play(
                sound
                    .volume(volume)
                    .range(Range::FOOTSTEP)
                    .importance(Importance::High),
            );
        }
        if every(6, 2.0, 2.6) || every(6, 5.5, 6.1) {
            let muzzle = [enemy[0], 1.4, enemy[2]];
            play(self.gunfire.at(muzzle, tick).range(Range::GUNFIRE));
        }

        // The player's own gun is not in the world: it is in both ears.
        if every(6, 3.4, 4.0) {
            play(self.gunfire.everywhere(tick).importance(Importance::High));
        }
        if at(4.2) {
            play(Play::everywhere(self.dry_fire.clone()));
        }
        if at(4.5) {
            play(Play::everywhere(self.reload.clone()));
        }
        if at(7.0) {
            play(Play::everywhere(self.landing.clone()));
        }

        Listener {
            position: [0.0, 1.6, 0.0],
            yaw: 0.0,
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    match std::env::args().nth(1) {
        Some(path) => into_file(&path),
        None => through_speakers(),
    }
}

/// Mixes the scene into a WAV file, with no sound card involved.
fn into_file(path: &str) -> Result<(), Box<dyn Error>> {
    let mut scene = Scene::new();
    let mut mixer = Mixer::new(RATE);
    let mut writer = hound::WavWriter::create(
        path,
        hound::WavSpec {
            channels: 2,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    let mut buffer = vec![0.0_f32; (RATE / TICKS_PER_SECOND) as usize * 2];
    let mut loudest = 0.0_f32;
    for tick in 0..SECONDS * TICKS_PER_SECOND {
        let listener = scene.tick(tick, |play| {
            mixer.play(play);
        });
        mixer.set_listener(listener);
        mixer.mix(&mut buffer);
        for sample in &buffer {
            loudest = loudest.max(sample.abs());
            writer.write_sample((sample * f32::from(i16::MAX)) as i16)?;
        }
    }
    writer.finalize()?;
    println!("Wrote {SECONDS} seconds to {path}. Loudest sample: {loudest:.2} of 1.");
    Ok(())
}

#[cfg(feature = "device")]
fn through_speakers() -> Result<(), Box<dyn Error>> {
    use std::time::{Duration, Instant};

    let mut speakers = fossps_audio::Speakers::open()?;
    println!("Playing {SECONDS} seconds: an enemy circles you, firing. Then your own gun.");
    let mut scene = Scene::new();
    let start = Instant::now();
    for tick in 0..SECONDS * TICKS_PER_SECOND {
        let mut plays = Vec::new();
        let listener = scene.tick(tick, |play| plays.push(play));
        speakers.set_listener(listener);
        for play in plays {
            speakers.play(play);
        }
        let next = Duration::from_secs_f64(f64::from(tick + 1) / f64::from(TICKS_PER_SECOND));
        std::thread::sleep(next.saturating_sub(start.elapsed()));
    }
    std::thread::sleep(Duration::from_millis(500));
    if !speakers.is_working() {
        println!("The sound device stopped partway through.");
    }
    Ok(())
}

#[cfg(not(feature = "device"))]
fn through_speakers() -> Result<(), Box<dyn Error>> {
    Err("Built without the `device` feature: give a file name to write a WAV instead.".into())
}
