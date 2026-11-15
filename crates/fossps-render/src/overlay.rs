//! What is drawn over the finished picture: the HUD's and menus' boxes and
//! text, worked out here and drawn by [`UiPass`](crate::ui_pass::UiPass).
//!
//! Nothing in this file touches the graphics card, so it is tested
//! without one.

use std::collections::HashMap;
use std::ops::Range;

use fossps_ui::{Color, Draw, DrawList};

use crate::font::Font;

/// The size of the picture all glyphs are kept in, in pixels.
///
/// Two megabytes holds a few thousand letters at the sizes a 1080p HUD
/// uses and several hundred at 4K. When it fills up it is emptied and
/// started again with what is on screen.
pub(crate) const ATLAS_WIDTH: u32 = 2048;
pub(crate) const ATLAS_HEIGHT: u32 = 1024;

/// The most boxes and letters drawn in one frame; the rest are left out.
///
/// A full settings screen is under two thousand. The limit is what a
/// frame's buffer on the graphics card is sized for.
pub(crate) const MOST_QUADS: usize = 8192;

/// The bytes one box or letter takes in the buffer the shader reads:
/// fourteen `f32`s, laid out as `shaders/ui.wgsl` expects.
pub(crate) const QUAD_BYTES: usize = 56;

/// The largest letters are drawn, in pixels. Larger text is drawn this
/// size, so that no text can ask for a picture of a letter the size of a
/// wall.
const LARGEST_TEXT: f32 = 256.0;
/// The smallest, below which letters are a smudge.
const SMALLEST_TEXT: f32 = 4.0;

/// How many sizes there are to a pixel: letters are drawn at quarter-pixel
/// sizes, close enough to what was asked that centred text stays centred,
/// and few enough that a window being dragged larger does not fill the
/// atlas with a new alphabet for every pixel of height.
const SIZES_PER_PIXEL: f32 = 4.0;

/// The most pictures of letters remembered before starting again, so
/// that letters that take no room in the atlas (spaces) are limited too.
const MOST_GLYPHS: usize = 16_384;

/// Where one letter's picture is in the atlas and how to place it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Glyph {
    /// The top-left corner in the atlas, and the size; zero for a space.
    at: [u16; 2],
    size: [u16; 2],
    /// From the pen to the picture's left edge.
    left: f32,
    /// From the line the text stands on down to the picture's top edge;
    /// negative, as letters stand above the line.
    top: f32,
}

/// The atlas is full; nothing was added.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Full;

/// One picture holding every letter drawn so far, each at each size used.
struct Atlas {
    /// One byte a pixel: how much of it a letter covers.
    pixels: Vec<u8>,
    glyphs: HashMap<(u16, u16), Glyph>,
    /// Where the next letter goes, and the height of the tallest letter
    /// in the row being filled.
    next: [u32; 2],
    row: u32,
    /// The rows changed since the graphics card last got a copy.
    changed: Option<Range<u32>>,
}

impl Atlas {
    fn new() -> Self {
        Self {
            pixels: vec![0; (ATLAS_WIDTH * ATLAS_HEIGHT) as usize],
            glyphs: HashMap::new(),
            // One empty pixel is kept around every letter, so that a
            // letter never picks up its neighbour's edge.
            next: [1, 1],
            row: 0,
            changed: None,
        }
    }

    /// Forgets every letter.
    fn empty(&mut self) {
        let used = self.used_rows();
        self.pixels[..(used * ATLAS_WIDTH) as usize].fill(0);
        self.glyphs.clear();
        self.next = [1, 1];
        self.row = 0;
        self.changed = Some(0..used);
    }

    /// How many rows from the top hold letters, or did before `empty`.
    fn used_rows(&self) -> u32 {
        (self.next[1] + self.row + 1).min(ATLAS_HEIGHT)
    }

    /// Finds room for a picture `width` by `height` pixels.
    fn place(&mut self, width: u32, height: u32) -> Result<[u32; 2], Full> {
        if self.next[0] + width + 1 > ATLAS_WIDTH {
            self.next = [1, self.next[1] + self.row + 1];
            self.row = 0;
        }
        if self.next[1] + height + 1 > ATLAS_HEIGHT {
            return Err(Full);
        }
        let at = self.next;
        self.next[0] += width + 1;
        self.row = self.row.max(height);
        Ok(at)
    }

    /// The picture of `glyph` at `size` quarter pixels, drawn now if this
    /// is the first time it is asked for.
    fn glyph(&mut self, font: &Font, glyph: u16, size: u16) -> Result<Glyph, Full> {
        if let Some(found) = self.glyphs.get(&(glyph, size)) {
            return Ok(*found);
        }
        if self.glyphs.len() >= MOST_GLYPHS {
            return Err(Full);
        }
        let nothing = Glyph {
            at: [0, 0],
            size: [0, 0],
            left: 0.0,
            top: 0.0,
        };
        let px = f32::from(size) / SIZES_PER_PIXEL;
        let drawn = font.rasterize(glyph, px).and_then(|(metrics, coverage)| {
            let width = u16::try_from(metrics.width).ok()?;
            let height = u16::try_from(metrics.height).ok()?;
            // A picture that could never fit is left out, not waited for.
            let fits = u32::from(width) + 2 <= ATLAS_WIDTH && u32::from(height) + 2 <= ATLAS_HEIGHT;
            (width > 0 && height > 0 && fits && coverage.len() == metrics.width * metrics.height)
                .then_some((metrics, coverage, width, height))
        });
        let Some((metrics, coverage, width, height)) = drawn else {
            self.glyphs.insert((glyph, size), nothing);
            return Ok(nothing);
        };
        let [x, y] = self.place(u32::from(width), u32::from(height))?;
        for (row, line) in coverage.chunks_exact(usize::from(width)).enumerate() {
            let start = (y as usize + row) * ATLAS_WIDTH as usize + x as usize;
            if let Some(into) = self.pixels.get_mut(start..start + line.len()) {
                into.copy_from_slice(line);
            }
        }
        let rows = y..y + u32::from(height);
        self.changed = Some(match self.changed.take() {
            Some(before) => before.start.min(rows.start)..before.end.max(rows.end),
            None => rows,
        });
        // Glyph pictures are far smaller than the numbers an f32 or a u16 holds exactly.
        #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
        let found = Glyph {
            at: [x as u16, y as u16],
            size: [width, height],
            left: metrics.xmin as f32,
            top: -((metrics.ymin + i32::from(height)) as f32),
        };
        self.glyphs.insert((glyph, size), found);
        Ok(found)
    }
}

/// The boxes and letters to draw over the picture, ready for the shader.
pub(crate) struct Overlay {
    font: Font,
    atlas: Atlas,
    /// `QUAD_BYTES` for each box and letter, back to front.
    quads: Vec<u8>,
    /// Whether leaving something out has been logged, so it is said once.
    warned: bool,
}

impl Overlay {
    pub(crate) fn new(font: Font) -> Self {
        Self {
            font,
            atlas: Atlas::new(),
            quads: Vec::new(),
            warned: false,
        }
    }

    /// The font the text is drawn in.
    pub(crate) fn font(&self) -> &Font {
        &self.font
    }

    /// Replaces what is drawn with `list`.
    pub(crate) fn set(&mut self, list: DrawList<'_>) {
        if self.build(list, false).is_ok() {
            return;
        }
        // The atlas is full of letters from earlier frames, sizes or
        // menus. Start it again with only what is drawn now.
        self.atlas.empty();
        if self.build(list, true).is_err() {
            // `build` leaves letters out instead of stopping when told to.
            self.quads.clear();
        }
    }

    /// Turns `list` into quads. With `skip`, letters the atlas has no room
    /// for are left out; without, the first one stops the work.
    fn build(&mut self, list: DrawList<'_>, skip: bool) -> Result<(), Full> {
        self.quads.clear();
        let mut left_out = false;
        for draw in list {
            match draw {
                Draw::Rect {
                    position,
                    size,
                    color,
                    corner,
                } => {
                    let visible = position.iter().chain(&size).all(|value| value.is_finite())
                        && size[0] > 0.0
                        && size[1] > 0.0;
                    if !visible {
                        continue;
                    }
                    // Edges stand on whole pixels, so a thin crosshair is
                    // a sharp line and not two faint ones; nothing is made
                    // thinner than a pixel.
                    let left = position[0].round();
                    let top = position[1].round();
                    let width = ((position[0] + size[0]).round() - left).max(1.0);
                    let height = ((position[1] + size[1]).round() - top).max(1.0);
                    let corner = if corner.is_finite() {
                        corner.clamp(0.0, width.min(height) / 2.0)
                    } else {
                        0.0
                    };
                    left_out |=
                        !self.push([left, top, width, height], [0.0; 4], color, [corner, 0.0]);
                }
                Draw::Text {
                    position,
                    size,
                    color,
                    text,
                } => {
                    let visible =
                        position.iter().all(|value| value.is_finite()) && size.is_finite();
                    if !visible || size <= 0.0 {
                        continue;
                    }
                    left_out |= self.text(position, size, color, text, skip)?;
                }
            }
        }
        if left_out && !self.warned {
            self.warned = true;
            log::warn!(
                "There is more HUD and menu to draw than fits in one frame ({MOST_QUADS} boxes \
                 and letters, or more large text than the engine keeps pictures of), so some \
                 of it is left out. Show less text at once, or make it smaller."
            );
        }
        Ok(())
    }

    /// Adds the letters of one line. Returns whether any were left out.
    fn text(
        &mut self,
        position: [f32; 2],
        size: f32,
        color: Color,
        text: &str,
        skip: bool,
    ) -> Result<bool, Full> {
        // Clamped first, so the numbers fit a u16 with room to spare.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let quarters = (size.clamp(SMALLEST_TEXT, LARGEST_TEXT) * SIZES_PER_PIXEL).round() as u16;
        let px = f32::from(quarters) / SIZES_PER_PIXEL;
        // Letters stand on whole pixels, or their flat edges blur.
        let baseline = (position[1] + self.font.baseline(size, px)).round();
        let mut left_out = false;
        let mut stopped = false;
        // The font is a cheap shared handle; taking a copy lets the letters
        // be added to `self` while it is being read.
        let font = self.font.clone();
        font.each_glyph(text, px, |placed| {
            if stopped {
                return;
            }
            let glyph = match self.atlas.glyph(&font, placed.glyph, quarters) {
                Ok(glyph) => glyph,
                Err(Full) if skip => {
                    left_out = true;
                    return;
                }
                Err(Full) => {
                    stopped = true;
                    return;
                }
            };
            if glyph.size[0] == 0 {
                return;
            }
            let rect = [
                (position[0] + placed.x + glyph.left).round(),
                baseline + glyph.top,
                f32::from(glyph.size[0]),
                f32::from(glyph.size[1]),
            ];
            let from = [
                f32::from(glyph.at[0]),
                f32::from(glyph.at[1]),
                f32::from(glyph.size[0]),
                f32::from(glyph.size[1]),
            ];
            left_out |= !self.push(rect, from, color, [0.0, 1.0]);
        });
        if stopped {
            return Err(Full);
        }
        Ok(left_out)
    }

    /// Adds one quad, unless the frame is full. Returns whether it was added.
    fn push(&mut self, rect: [f32; 4], from: [f32; 4], color: Color, shape: [f32; 2]) -> bool {
        if self.quads.len() >= MOST_QUADS * QUAD_BYTES {
            return false;
        }
        let colour = [color.r, color.g, color.b, color.a].map(|part| {
            if part.is_finite() {
                part.clamp(0.0, 1.0)
            } else {
                0.0
            }
        });
        for value in rect.iter().chain(&from).chain(&colour).chain(&shape) {
            self.quads.extend_from_slice(&value.to_ne_bytes());
        }
        true
    }

    /// The quads to draw, `QUAD_BYTES` each, back to front.
    pub(crate) fn quads(&self) -> &[u8] {
        &self.quads
    }

    /// How many quads there are to draw.
    pub(crate) fn count(&self) -> u32 {
        u32::try_from(self.quads.len() / QUAD_BYTES).unwrap_or(0)
    }

    /// The rows of the atlas that changed since [`sent`](Self::sent) was
    /// last called, with their pixels: `ATLAS_WIDTH` bytes a row.
    pub(crate) fn changed(&self) -> Option<(Range<u32>, &[u8])> {
        let rows = self.atlas.changed.clone()?;
        self.rows(rows)
    }

    /// The rows of the atlas that hold anything, with their pixels, for a
    /// graphics card that has no copy yet. Never empty.
    pub(crate) fn everything(&self) -> Option<(Range<u32>, &[u8])> {
        self.rows(0..self.atlas.used_rows().max(1))
    }

    fn rows(&self, rows: Range<u32>) -> Option<(Range<u32>, &[u8])> {
        let bytes = (rows.start * ATLAS_WIDTH) as usize..(rows.end * ATLAS_WIDTH) as usize;
        Some((rows, self.atlas.pixels.get(bytes)?))
    }

    /// Notes that the graphics card has been sent every change so far.
    pub(crate) fn sent(&mut self) {
        self.atlas.changed = None;
    }
}

#[cfg(test)]
mod tests {
    use fossps_ui::{Anchor, DrawBuffer, Ui, UiInput};

    use super::*;

    /// One quad, read back out of the bytes the shader gets.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct Quad {
        rect: [f32; 4],
        from: [f32; 4],
        colour: [f32; 4],
        corner: f32,
        letter: bool,
    }

    fn quads(overlay: &Overlay) -> Vec<Quad> {
        overlay
            .quads()
            .as_chunks::<QUAD_BYTES>()
            .0
            .iter()
            .map(|bytes| {
                let number = |index: usize| {
                    f32::from_ne_bytes(bytes[index * 4..index * 4 + 4].try_into().unwrap())
                };
                Quad {
                    rect: [number(0), number(1), number(2), number(3)],
                    from: [number(4), number(5), number(6), number(7)],
                    colour: [number(8), number(9), number(10), number(11)],
                    corner: number(12),
                    letter: number(13) == 1.0,
                }
            })
            .collect()
    }

    /// A list holding one line of text at `position`.
    fn line(text: &str, size: f32, position: [f32; 2]) -> DrawBuffer {
        let mut ui = Ui::default();
        ui.set_text_measure(Box::new(Font::bundled()));
        ui.hud_text(Anchor::TopLeft, position, size, text);
        // On a screen 1080 high, sizes are pixels.
        ui.update(&UiInput::default(), 1.0 / 60.0, [1920, 1080]);
        let mut buffer = DrawBuffer::default();
        buffer.add(ui.draw());
        buffer
    }

    #[test]
    fn letters_become_quads_that_read_their_own_part_of_the_atlas() {
        let mut overlay = Overlay::new(Font::bundled());
        let list = line("30 / 90", 40.0, [0.0, 0.0]);
        overlay.set(list.draw());
        let drawn = quads(&overlay);
        // Seven characters, two of them spaces.
        assert_eq!(drawn.len(), 5);
        assert_eq!(overlay.count(), 5);
        for (index, quad) in drawn.iter().enumerate() {
            assert!(quad.letter);
            // The theme's text colour.
            assert_eq!(quad.colour, [0.92, 0.93, 0.95, 1.0]);
            // Whole pixels, the size of the picture read.
            assert_eq!(quad.rect.map(f32::round), quad.rect);
            assert_eq!(quad.rect[2..], quad.from[2..]);
            assert!(quad.rect[3] > 20.0 && quad.rect[3] < 40.0, "{quad:?}");
            if index > 0 {
                assert!(quad.rect[0] > drawn[index - 1].rect[0]);
            }
        }
        // "0" is drawn twice from one picture; "3", "9" and "/" have their own.
        assert_eq!(drawn[1].from, drawn[4].from);
        assert_ne!(drawn[0].from, drawn[1].from);
        assert_eq!(overlay.atlas.glyphs.len(), 5);

        // The picture of the "3" has ink in it, and an empty edge around it.
        let [x, y, width, height] = drawn[0].from.map(|value| value as usize);
        let at = |x: usize, y: usize| overlay.atlas.pixels[y * ATLAS_WIDTH as usize + x];
        let ink: u32 = (y..y + height)
            .flat_map(|y| (x..x + width).map(move |x| (x, y)))
            .map(|(x, y)| u32::from(at(x, y)))
            .sum();
        assert!(ink > 255 * 20, "{ink}");
        assert!((x - 1..=x + width).all(|x| at(x, y - 1) == 0 && at(x, y + height) == 0));
        assert!((y - 1..=y + height).all(|y| at(x - 1, y) == 0 && at(x + width, y) == 0));
    }

    #[test]
    fn only_new_letters_are_sent_to_the_graphics_card() {
        let mut overlay = Overlay::new(Font::bundled());
        assert!(overlay.changed().is_none());
        let (rows, pixels) = overlay.everything().unwrap();
        assert_eq!((rows, pixels.len()), (0..2, 2 * ATLAS_WIDTH as usize));

        overlay.set(line("abc", 32.0, [0.0, 0.0]).draw());
        let (rows, pixels) = overlay.changed().unwrap();
        assert!(rows.start >= 1 && rows.end < 40, "{rows:?}");
        assert_eq!(pixels.len(), rows.len() * ATLAS_WIDTH as usize);
        assert!(pixels.iter().any(|&pixel| pixel > 0));
        overlay.sent();

        // The same letters again: nothing to send.
        overlay.set(line("cab", 32.0, [50.0, 9.0]).draw());
        assert!(overlay.changed().is_none());
        // A new one, or an old one at a new size, is sent.
        overlay.set(line("cabd", 32.0, [0.0, 0.0]).draw());
        assert!(overlay.changed().is_some());
        overlay.sent();
        overlay.set(line("a", 48.0, [0.0, 0.0]).draw());
        assert!(overlay.changed().is_some());
    }

    #[test]
    fn text_is_centred_in_the_line_the_ui_gave_it() {
        let mut overlay = Overlay::new(Font::bundled());
        overlay.set(line("H", 40.0, [100.0, 200.0]).draw());
        let quad = quads(&overlay)[0];
        // The UI keeps the HUD inside the safe area, so read where it put the line.
        let list = line("H", 40.0, [100.0, 200.0]);
        let Some(Draw::Text { position, size, .. }) = list.draw().iter().next() else {
            panic!("no text");
        };
        let middle = quad.rect[1] + quad.rect[3] / 2.0;
        assert!(
            (middle - (position[1] + size / 2.0)).abs() <= 2.0,
            "a capital's middle is at {middle}, the line's at {}",
            position[1] + size / 2.0
        );
        assert!(quad.rect[0] >= position[0] && quad.rect[0] < position[0] + 6.0);
    }

    #[test]
    fn boxes_keep_their_place_and_corners_never_exceed_half_a_side() {
        let mut ui = Ui::default();
        let screen = ui.add_screen("Paused", |screen| {
            screen.button("Resume");
        });
        ui.open(screen.unwrap()).unwrap();
        ui.update(&UiInput::default(), 1.0 / 60.0, [1920, 1080]);
        let mut overlay = Overlay::new(Font::bundled());
        overlay.set(ui.draw());
        let drawn = quads(&overlay);
        let expected: Vec<_> = ui
            .draw()
            .iter()
            .filter_map(|draw| match draw {
                Draw::Rect { position, size, .. } => {
                    // Each edge on the nearest whole pixel.
                    let (left, top) = (position[0].round(), position[1].round());
                    Some([
                        left,
                        top,
                        (position[0] + size[0]).round() - left,
                        (position[1] + size[1]).round() - top,
                    ])
                }
                Draw::Text { .. } => None,
            })
            .collect();
        let boxes: Vec<_> = drawn.iter().filter(|quad| !quad.letter).collect();
        assert_eq!(
            boxes.iter().map(|quad| quad.rect).collect::<Vec<_>>(),
            expected
        );
        assert!(boxes.len() >= 3 && drawn.len() > boxes.len());
        for quad in boxes {
            assert!(quad.corner >= 0.0 && quad.corner <= quad.rect[2].min(quad.rect[3]) / 2.0);
        }
        // Back to front: the darkening over the game comes first.
        assert!(!drawn[0].letter);
        assert_eq!(drawn[0].rect, [0.0, 0.0, 1920.0, 1080.0]);
    }

    #[test]
    fn a_full_atlas_starts_again_with_what_is_on_screen() {
        let mut overlay = Overlay::new(Font::bundled());
        let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
        // Large letters at many sizes, as a window dragged taller would ask for.
        let mut size = 120.0;
        let mut emptied = false;
        for _ in 0..200 {
            let before = overlay.atlas.glyphs.len();
            overlay.set(line(alphabet, size, [0.0, 0.0]).draw());
            // Every letter is drawn, also in the frame that starts again.
            assert_eq!(overlay.count(), 62);
            size += 0.25;
            if overlay.atlas.glyphs.len() < before {
                emptied = true;
                assert_eq!(overlay.atlas.glyphs.len(), 62);
                let (rows, _) = overlay.changed().unwrap();
                assert_eq!(rows.start, 0);
                break;
            }
        }
        assert!(emptied, "the atlas was never started again");
    }

    #[test]
    fn too_much_for_one_frame_is_left_out_and_huge_text_is_held_to_a_size() {
        let mut overlay = Overlay::new(Font::bundled());
        let mut buffer = DrawBuffer::default();
        let long = line(&"x".repeat(1000), 24.0, [0.0, 0.0]);
        for _ in 0..10 {
            buffer.add(long.draw());
        }
        overlay.set(buffer.draw());
        assert_eq!(overlay.count() as usize, MOST_QUADS);
        assert_eq!(overlay.quads().len(), MOST_QUADS * QUAD_BYTES);

        // Text far larger than a screen is drawn at the largest size kept.
        overlay.set(line("W", 100_000.0, [0.0, 0.0]).draw());
        let huge = quads(&overlay);
        assert_eq!(huge.len(), 1);
        assert!(huge[0].rect[3] <= LARGEST_TEXT);
    }

    #[test]
    fn a_character_the_font_lacks_is_a_box_not_a_gap() {
        let mut overlay = Overlay::new(Font::bundled());
        assert!(!Font::bundled().has('語'));
        overlay.set(line("語", 40.0, [0.0, 0.0]).draw());
        assert_eq!(overlay.count(), 1);
    }
}
