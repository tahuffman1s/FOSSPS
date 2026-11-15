//! The benchmark behind `fossps bench`: one fixed scene, drawn along one
//! fixed path with no window, and how long its frames took.
//!
//! Every run draws exactly the same frames, so two runs differ only in how
//! fast the computer drew them. A [`Report`] can be held against a budget:
//! the engine's own automated checks do that on every change, so a change
//! that makes frames much slower is caught before it is merged.
//!
//! ```no_run
//! use fossps::bench::{Bench, run};
//!
//! # fn main() -> Result<(), fossps::render::RenderError> {
//! let report = run(&Bench::default())?;
//! print!("{}", report.text());
//! # Ok(())
//! # }
//! ```

use std::fmt::Write as _;
use std::time::Duration;

use fossps_assets::{Instance, Material, Mesh, MeshPart, Scene};
use fossps_render::{Camera, Headless, RenderError, Upscaling};

use crate::standin::{at, block, tile};

/// How many blocks stand along each side of the arena.
const BLOCKS: u8 = 40;
/// How far apart the blocks stand, in metres.
const SPACING: f32 = 4.0;
/// How far the floor reaches from the middle, in metres.
const FLOOR: f32 = 500.0;
/// How far from the middle the camera walks its circle, in metres.
const CIRCLE: f32 = 30.0;
/// How many frames one lap of the circle takes: 20 seconds at 60 a second.
const LAP_FRAMES: f32 = 1200.0;

/// The colours of the blocks.
const PAINTS: [[f32; 4]; 4] = [
    [0.80, 0.22, 0.18, 1.0],
    [0.20, 0.45, 0.85, 1.0],
    [0.90, 0.70, 0.15, 1.0],
    [0.25, 0.65, 0.35, 1.0],
];

/// What to draw and how much of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bench {
    /// How many frames are timed.
    pub frames: u32,
    /// How many frames are drawn first and not timed, while the graphics
    /// driver settles.
    pub warmup: u32,
    /// The width of the finished picture, in pixels.
    pub width: u32,
    /// The height of the finished picture, in pixels.
    pub height: u32,
    /// How much smaller the scene is drawn before FSR makes it full size.
    pub upscaling: Upscaling,
}

impl Default for Bench {
    /// 600 timed frames after 120 untimed ones, at 1280 by 720 with the
    /// engine's default upscaling.
    fn default() -> Self {
        Self {
            frames: 600,
            warmup: 120,
            width: 1280,
            height: 720,
            upscaling: Upscaling::default(),
        }
    }
}

/// The benchmark's scene: a floor with 1,600 blocks of four colours and
/// many heights standing on it in rows.
///
/// It is built in code, the same every time, so it needs no files.
#[must_use]
pub fn scene() -> Scene {
    let mut scene = Scene::default();
    let mut floor = MeshPart::default();
    tile(&mut floor, [-FLOOR, FLOOR], 0.0, [-FLOOR, FLOOR]);
    let shapes = std::iter::once(("floor", floor, [0.26, 0.28, 0.31, 1.0]))
        .chain(PAINTS.map(|paint| ("block", block(), paint)));
    for (number, (name, part, base_color)) in (0_u32..).zip(shapes) {
        scene.materials.push(Material {
            name: Some(name.to_owned()),
            base_color,
            ..Material::default()
        });
        scene.meshes.push(Mesh {
            name: Some(name.to_owned()),
            parts: vec![MeshPart {
                material: number,
                ..part
            }],
        });
    }

    let mut place = |mesh: u32, position: [f32; 3], size: [f32; 3]| {
        scene.instances.push(Instance {
            mesh,
            transform: at(position, size),
            mirrored: false,
            name: None,
            extras: None,
        });
    };
    place(0, [0.0; 3], [1.0; 3]);
    let half = f32::from(BLOCKS - 1) / 2.0;
    for row in 0..BLOCKS {
        for column in 0..BLOCKS {
            // Not random, but mixed enough that neighbours differ.
            let mix = u32::from(row) * 7 + u32::from(column) * 13;
            let height = 1.0 + f32::from(u8::try_from(mix % 5).unwrap_or(0));
            place(
                1 + mix % 4,
                [
                    (f32::from(column) - half) * SPACING,
                    0.0,
                    (f32::from(row) - half) * SPACING,
                ],
                [1.5, height, 1.5],
            );
        }
    }
    scene
}

/// Where frame number `frame` of the benchmark is seen from: eyes 1.7 m up,
/// walking a circle around the middle of the arena and looking ahead, with
/// a slow nod up and down.
#[must_use]
pub fn camera(frame: u32) -> Camera {
    // Frame numbers stay far below the point where f32 stops being exact.
    #[allow(clippy::cast_precision_loss)]
    let turn = frame as f32 / LAP_FRAMES * std::f32::consts::TAU;
    Camera {
        position: [turn.sin() * CIRCLE, 1.7, -turn.cos() * CIRCLE],
        // Facing along the circle, which turns right, so the yaw falls.
        yaw: -turn - std::f32::consts::FRAC_PI_2,
        pitch: (turn * 4.0).sin() * 0.15,
        ..Camera::default()
    }
}

/// How long the frames of one kind of work took, in milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Times {
    /// The usual frame: half were faster and half slower (the median).
    pub usual: f64,
    /// A slow frame: 99 of 100 frames were faster (the 99th percentile).
    pub slow: f64,
    /// The slowest frame of all.
    pub slowest: f64,
    /// All the frames' times added up and divided by how many there were.
    pub average: f64,
}

impl Times {
    /// Sums up `frames`. With no frames, every time is 0.
    #[must_use]
    pub fn of(frames: &[Duration]) -> Self {
        let mut sorted: Vec<f64> = frames
            .iter()
            .map(|time| time.as_secs_f64() * 1000.0)
            .collect();
        sorted.sort_by(f64::total_cmp);
        let Some(&slowest) = sorted.last() else {
            return Self::default();
        };
        // The smallest time that at least `percent` of the frames are within.
        let within = |percent: usize| {
            let rank = (sorted.len() * percent).div_ceil(100).max(1);
            sorted[rank - 1]
        };
        // No benchmark has anywhere near 2^52 frames.
        #[allow(clippy::cast_precision_loss)]
        let average = sorted.iter().sum::<f64>() / sorted.len() as f64;
        Self {
            usual: within(50),
            slow: within(99),
            slowest,
            average,
        }
    }
}

/// What a run of the benchmark found.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// What was asked for.
    pub bench: Bench,
    /// The name of the graphics card that drew the frames.
    pub gpu: String,
    /// The size the scene was drawn at before upscaling, in pixels.
    pub drawn: (u32, u32),
    /// How many meshes the scene places.
    pub placed: usize,
    /// How many triangles those meshes have between them.
    pub triangles: usize,
    /// Whole frames.
    pub frame: Times,
    /// The engine's own share of each frame: telling the graphics card what to draw.
    pub recording: Times,
    /// The graphics card's share of each frame.
    pub drawing: Times,
}

/// Draws the benchmark and times its frames.
///
/// # Errors
///
/// Fails if Vulkan 1.3 is not available, no graphics card can run the
/// renderer, or the graphics driver reports a failure. The error's message
/// says what to do about it.
pub fn run(bench: &Bench) -> Result<Report, RenderError> {
    let scene = scene();
    let mut headless = Headless::new(bench.width, bench.height, bench.upscaling)?;
    headless.set_scene(&scene)?;
    for frame in 0..bench.warmup {
        headless.render(&camera(frame))?;
    }
    let count = bench.frames as usize;
    let (mut whole, mut recording, mut drawing) = (
        Vec::with_capacity(count),
        Vec::with_capacity(count),
        Vec::with_capacity(count),
    );
    for frame in 0..bench.frames {
        let time = headless.render(&camera(bench.warmup + frame))?;
        whole.push(time.total());
        recording.push(time.recording);
        drawing.push(time.drawing);
    }
    Ok(Report {
        bench: *bench,
        gpu: headless.gpu().name().to_owned(),
        drawn: headless.drawn_size(),
        placed: scene.instances.len(),
        triangles: triangles(&scene),
        frame: Times::of(&whole),
        recording: Times::of(&recording),
        drawing: Times::of(&drawing),
    })
}

/// How many triangles `scene` has, counting a mesh once for each place it stands.
fn triangles(scene: &Scene) -> usize {
    scene
        .instances
        .iter()
        .filter_map(|instance| scene.meshes.get(instance.mesh as usize))
        .flat_map(|mesh| &mesh.parts)
        .map(|part| part.indices.len() / 3)
        .sum()
}

/// The name `fossps bench --upscaling` takes for `upscaling`.
#[must_use]
pub fn upscaling_name(upscaling: Upscaling) -> &'static str {
    match upscaling {
        Upscaling::Off => "off",
        Upscaling::Native => "native",
        Upscaling::Quality => "quality",
        Upscaling::Balanced => "balanced",
        Upscaling::Performance => "performance",
        Upscaling::UltraPerformance => "ultra-performance",
    }
}

impl Report {
    /// The report for a person to read.
    #[must_use]
    pub fn text(&self) -> String {
        let Bench {
            frames,
            width,
            height,
            upscaling,
            ..
        } = self.bench;
        let mut text = format!(
            "Drew {frames} frames of the benchmark scene ({} meshes, {} triangles) at \
             {width}x{height}",
            self.placed, self.triangles
        );
        if self.drawn == (width, height) {
            text.push_str(".\n");
        } else {
            let _ = writeln!(
                text,
                ", upscaled from {}x{} (upscaling: {}).",
                self.drawn.0,
                self.drawn.1,
                upscaling_name(upscaling)
            );
        }
        let _ = writeln!(text, "Graphics card: {}\n", self.gpu);
        let per_second = if self.frame.usual > 0.0 {
            format!("{:.0} frames a second", 1000.0 / self.frame.usual)
        } else {
            "too fast to measure".to_owned()
        };
        let _ = writeln!(
            text,
            "  Usual frame    {:>8.2} ms   {per_second}",
            self.frame.usual
        );
        let _ = writeln!(
            text,
            "  Slow frame     {:>8.2} ms   99 of 100 frames were faster",
            self.frame.slow
        );
        let _ = writeln!(text, "  Slowest frame  {:>8.2} ms", self.frame.slowest);
        let _ = writeln!(
            text,
            "\nOf the usual frame, the engine's own work took {:.2} ms and the graphics \
             card's {:.2} ms.",
            self.recording.usual, self.drawing.usual
        );
        text
    }

    /// The report as JSON, for a program to read. `budget` is the most a
    /// slow frame may take, in milliseconds, if there is a limit.
    #[must_use]
    pub fn json(&self, budget: Option<f64>) -> String {
        let times = |times: &Times| {
            format!(
                "{{ \"usual_ms\": {:.4}, \"slow_ms\": {:.4}, \"slowest_ms\": {:.4}, \
                 \"average_ms\": {:.4} }}",
                times.usual, times.slow, times.slowest, times.average
            )
        };
        let gpu: String = self
            .gpu
            .chars()
            .filter(|letter| !letter.is_control())
            .flat_map(|letter| match letter {
                '"' | '\\' => vec!['\\', letter],
                _ => vec![letter],
            })
            .collect();
        format!(
            "{{\n  \"fossps\": \"{}\",\n  \"graphics_card\": \"{gpu}\",\n  \
             \"frames\": {},\n  \"warmup_frames\": {},\n  \"width\": {},\n  \"height\": {},\n  \
             \"drawn_width\": {},\n  \"drawn_height\": {},\n  \"upscaling\": \"{}\",\n  \
             \"meshes\": {},\n  \"triangles\": {},\n  \"frame\": {},\n  \"engine\": {},\n  \
             \"graphics_card_work\": {},\n  \"budget_ms\": {},\n  \"within_budget\": {}\n}}\n",
            env!("CARGO_PKG_VERSION"),
            self.bench.frames,
            self.bench.warmup,
            self.bench.width,
            self.bench.height,
            self.drawn.0,
            self.drawn.1,
            upscaling_name(self.bench.upscaling),
            self.placed,
            self.triangles,
            times(&self.frame),
            times(&self.recording),
            times(&self.drawing),
            budget.map_or_else(|| "null".to_owned(), |budget| format!("{budget}")),
            self.within(budget),
        )
    }

    /// Whether a slow frame took no longer than `budget` milliseconds.
    /// With no budget, always true.
    #[must_use]
    pub fn within(&self, budget: Option<f64>) -> bool {
        budget.is_none_or(|budget| self.frame.slow <= budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn milliseconds(times: &[u64]) -> Vec<Duration> {
        times.iter().copied().map(Duration::from_millis).collect()
    }

    fn report(slow: f64) -> Report {
        Report {
            bench: Bench::default(),
            gpu: "Test \"card\"".to_owned(),
            drawn: (853, 480),
            placed: 1601,
            triangles: 19_202,
            frame: Times {
                usual: 8.0,
                slow,
                slowest: 30.0,
                average: 9.0,
            },
            recording: Times::default(),
            drawing: Times::default(),
        }
    }

    #[test]
    fn the_scene_and_the_path_are_the_same_every_time() {
        let scene = scene();
        assert_eq!(scene, super::scene());
        let blocks = usize::from(BLOCKS).pow(2);
        assert_eq!(scene.instances.len(), blocks + 1);
        assert_eq!(triangles(&scene), 2 + blocks * 12);
        assert!(
            scene
                .instances
                .iter()
                .all(|instance| (instance.mesh as usize) < scene.meshes.len())
        );
        assert_eq!(camera(77), camera(77));
        assert_ne!(camera(77), camera(78));
    }

    #[test]
    fn the_camera_walks_a_circle_looking_ahead() {
        for frame in [0, 150, 700, 1199] {
            let now = camera(frame);
            let next = camera(frame + 1);
            let moved = [
                next.position[0] - now.position[0],
                next.position[2] - now.position[2],
            ];
            // Yaw 0 looks along -Z and turns left as it grows.
            let facing = [-now.yaw.sin(), -now.yaw.cos()];
            let along = moved[0] * facing[0] + moved[1] * facing[1];
            let length = moved[0].hypot(moved[1]);
            assert!(along > 0.99 * length, "frame {frame}: {moved:?} {facing:?}");
            assert!((now.position[0].hypot(now.position[2]) - CIRCLE).abs() < 1e-3);
        }
    }

    #[test]
    fn times_are_the_middle_the_99th_in_100_and_the_worst() {
        let frames: Vec<u64> = (1..=200).collect();
        let times = Times::of(&milliseconds(&frames));
        assert_eq!(times.usual, 100.0);
        assert_eq!(times.slow, 198.0);
        assert_eq!(times.slowest, 200.0);
        assert_eq!(times.average, 100.5);

        let one = Times::of(&milliseconds(&[7]));
        assert_eq!((one.usual, one.slow, one.slowest), (7.0, 7.0, 7.0));
        assert_eq!(Times::of(&[]), Times::default());
        // The order the frames came in does not matter.
        assert_eq!(Times::of(&milliseconds(&[9, 1, 5])).usual, 5.0);
    }

    #[test]
    fn a_budget_is_held_against_the_slow_frame() {
        assert!(report(12.0).within(None));
        assert!(report(12.0).within(Some(12.0)));
        assert!(!report(12.1).within(Some(12.0)));
    }

    #[test]
    fn the_report_reads_as_text_and_as_json() {
        let text = report(12.0).text();
        assert!(text.contains("600 frames"), "{text}");
        assert!(text.contains("1280x720, upscaled from 853x480"), "{text}");
        assert!(text.contains("125 frames a second"), "{text}");
        assert!(text.contains("Slow frame        12.00 ms"), "{text}");

        let json = report(12.0).json(Some(10.0));
        assert!(
            json.contains("\"graphics_card\": \"Test \\\"card\\\"\""),
            "{json}"
        );
        assert!(json.contains("\"slow_ms\": 12.0000"), "{json}");
        assert!(json.contains("\"budget_ms\": 10,"), "{json}");
        assert!(json.contains("\"within_budget\": false"), "{json}");
        assert!(report(12.0).json(None).contains("\"budget_ms\": null"));
    }
}
