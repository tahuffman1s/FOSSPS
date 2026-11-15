//! The font HUD and menu text is drawn in.

use std::fmt;
use std::sync::{Arc, OnceLock};

use fossps_ui::TextMeasure;

/// Atkinson Hyperlegible Next, made by the Braille Institute so that letters
/// and digits are hard to mistake for each other: `I`, `l` and `1`, `O` and
/// `0`. It is under the SIL Open Font License 1.1, which is in
/// `fonts/OFL.txt` beside it.
const BUNDLED: &[u8] = include_bytes!("../fonts/AtkinsonHyperlegibleNext-Regular.ttf");

/// The widest a character is taken to be when there is no font to ask, as
/// a share of the text's height. The same guess as `fossps_ui::EstimatedText`.
const GUESSED_WIDTH: f32 = 0.55;

/// The font text is drawn in, and the widths of words in it.
///
/// Every [`Renderer`](crate::Renderer) draws text in the font that comes
/// with the engine. Give the same font to a `fossps_ui::Ui` and it centres
/// text and sizes buttons by the real widths instead of an estimate:
///
/// ```
/// use fossps_render::Font;
/// use fossps_ui::{TextMeasure, Ui};
///
/// let mut ui = Ui::default();
/// ui.set_text_measure(Box::new(Font::bundled()));
///
/// // An "i" is narrower than an "m", which the estimate does not know.
/// let font = Font::bundled();
/// assert!(font.width("i", 28.0) < font.width("m", 28.0));
/// ```
///
/// It holds no graphics card memory and needs no window, so a server or a
/// test can measure text too. Copies are cheap: they share one font.
#[derive(Clone)]
pub struct Font {
    /// `None` if the font could not be read, which a test rules out for the
    /// bundled one. Text is then not drawn and its width is estimated.
    inner: Option<Arc<Loaded>>,
}

struct Loaded {
    font: fontdue::Font,
    /// How far the tallest letters reach above the line they stand on, as
    /// a share of the text's size.
    ascent: f32,
    /// How far the lowest reach below it; a negative number.
    descent: f32,
}

impl fmt::Debug for Font {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = self.inner.as_ref().and_then(|loaded| loaded.font.name());
        f.debug_struct("Font").field("name", &name).finish()
    }
}

/// One character of a line of text, as [`Font::each_glyph`] hands them out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Placed {
    /// Which of the font's shapes this is.
    pub(crate) glyph: u16,
    /// How far from the start of the line the character begins.
    pub(crate) x: f32,
}

impl Font {
    /// The font that comes with the engine: Atkinson Hyperlegible Next.
    ///
    /// It covers the Latin alphabet with its accents, digits and common
    /// symbols. A character it does not have is drawn as an empty box.
    #[must_use]
    pub fn bundled() -> Self {
        static FONT: OnceLock<Font> = OnceLock::new();
        FONT.get_or_init(|| Self::read(BUNDLED)).clone()
    }

    /// Reads a TrueType or OpenType font.
    fn read(bytes: &[u8]) -> Self {
        let settings = fontdue::FontSettings {
            // The largest text is rarely over a hundred pixels; outlines
            // are prepared for sizes up to about this one.
            scale: 64.0,
            ..fontdue::FontSettings::default()
        };
        let inner = match fontdue::Font::from_bytes(bytes, settings) {
            Ok(font) => {
                let (ascent, descent) = font
                    .horizontal_line_metrics(1.0)
                    .map_or((0.95, -0.25), |line| (line.ascent, line.descent));
                Some(Arc::new(Loaded {
                    font,
                    ascent,
                    descent,
                }))
            }
            Err(reason) => {
                log::error!(
                    "The engine's own font could not be read ({reason}), so no text will be \
                     drawn. This is a fault in FOSSPS itself; please report it at \
                     https://github.com/tahuffman1s/FOSSPS/issues."
                );
                None
            }
        };
        Self { inner }
    }

    /// Whether the font has a shape for `character`.
    ///
    /// ```
    /// let font = fossps_render::Font::bundled();
    /// assert!(font.has('é'));
    /// assert!(!font.has('語'));
    /// ```
    #[must_use]
    pub fn has(&self, character: char) -> bool {
        self.inner
            .as_ref()
            .is_some_and(|loaded| loaded.font.has_glyph(character))
    }

    /// Where the line text stands on is, measured down from the top of a
    /// line `size` high whose letters are `px` pixels: the letters are
    /// centred in the line.
    pub(crate) fn baseline(&self, size: f32, px: f32) -> f32 {
        let (ascent, descent) = self
            .inner
            .as_ref()
            .map_or((0.95, -0.25), |loaded| (loaded.ascent, loaded.descent));
        (size + (ascent + descent) * px) / 2.0
    }

    /// Calls `each` for every character of `text` drawn `px` high, left to
    /// right, and returns the width of the whole line.
    ///
    /// Characters that are not drawn at all, such as a line break or a tab,
    /// are left out and take no room: a piece of UI text is one line.
    /// Nothing is called if the font could not be read.
    pub(crate) fn each_glyph(&self, text: &str, px: f32, mut each: impl FnMut(Placed)) -> f32 {
        let Some(loaded) = &self.inner else {
            return 0.0;
        };
        let font = &loaded.font;
        let mut x = 0.0;
        let mut last = None;
        for character in text.chars().filter(|character| !character.is_control()) {
            let glyph = font.lookup_glyph_index(character);
            if let Some(last) = last {
                // Pairs such as "AV" sit closer together than their widths say.
                x += font.horizontal_kern_indexed(last, glyph, px).unwrap_or(0.0);
            }
            each(Placed { glyph, x });
            x += font.metrics_indexed(glyph, px).advance_width;
            last = Some(glyph);
        }
        x
    }

    /// The picture of one of the font's shapes, `px` pixels high: one byte
    /// a pixel saying how much of it the shape covers, row by row from the
    /// top, with where it goes. `None` if the font could not be read.
    pub(crate) fn rasterize(&self, glyph: u16, px: f32) -> Option<(fontdue::Metrics, Vec<u8>)> {
        let loaded = self.inner.as_ref()?;
        (glyph < loaded.font.glyph_count()).then(|| loaded.font.rasterize_indexed(glyph, px))
    }
}

impl TextMeasure for Font {
    fn width(&self, text: &str, size: f32) -> f32 {
        if self.inner.is_none() {
            // Sizes are small, and so is any text worth showing.
            #[allow(clippy::cast_precision_loss)]
            return text.chars().count() as f32 * size * GUESSED_WIDTH;
        }
        if !(size.is_finite() && size > 0.0) {
            return 0.0;
        }
        self.each_glyph(text, size, |_| {})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_font_is_read_and_has_what_a_hud_needs() {
        let font = Font::bundled();
        assert!(font.inner.is_some(), "the bundled font could not be read");
        for character in "AZaz09 /:.,!?%+-()[]<>'\"éñüßøÅ€".chars() {
            assert!(font.has(character), "no {character:?} in the bundled font");
        }
    }

    #[test]
    fn widths_follow_the_letters_and_the_size() {
        let font = Font::bundled();
        let narrow = font.width("illi", 28.0);
        let wide = font.width("mwmw", 28.0);
        assert!(
            narrow > 0.0 && narrow * 2.0 < wide,
            "{narrow} against {wide}"
        );
        // Twice the size is twice as wide.
        let double = font.width("30 / 90", 56.0);
        assert!((double - 2.0 * font.width("30 / 90", 28.0)).abs() < 0.01);
        // A line of text is roughly half as wide as it is high per letter.
        let per_letter = font.width("Resume the match", 28.0) / 16.0;
        assert!((8.0..22.0).contains(&per_letter), "{per_letter}");
    }

    #[test]
    fn nothing_and_nonsense_have_no_width() {
        let font = Font::bundled();
        assert_eq!(font.width("", 28.0), 0.0);
        assert_eq!(font.width("abc", 0.0), 0.0);
        assert_eq!(font.width("abc", f32::NAN), 0.0);
        assert_eq!(font.width("abc", -3.0), 0.0);
        // A line break takes no room: UI text is one line.
        assert_eq!(font.width("a\nb\t", 28.0), font.width("ab", 28.0));
    }

    #[test]
    fn the_same_letters_are_placed_where_the_width_says() {
        let font = Font::bundled();
        let mut starts = Vec::new();
        let width = font.each_glyph("AVA", 40.0, |placed| starts.push(placed.x));
        assert_eq!(starts.len(), 3);
        assert_eq!(starts[0], 0.0);
        assert!(starts[1] > 0.0 && starts[2] > starts[1] && width > starts[2]);
    }

    #[test]
    fn letters_are_centred_in_their_line() {
        let font = Font::bundled();
        // The line a 40-pixel text stands on is well below the middle and
        // above the bottom of a 40-pixel line.
        let baseline = font.baseline(40.0, 40.0);
        assert!((24.0..38.0).contains(&baseline), "{baseline}");
    }
}
