//! The latest script error, shown over the game until the script loads
//! again.
//!
//! The same error is in the terminal, but the person who made it is
//! looking at the game. The panel sits in the top-left corner and the game
//! carries on behind it.

use std::fmt;

use fossps_script::ScriptError;
use fossps_ui::{Anchor, Color, TextMeasure, Ui, UiInput};

use crate::game::Frame;

/// The widest the panel gets, in pixels on a screen 1080 high.
const WIDEST: f32 = 1100.0;
/// Space between the panel's edge and its text.
const PADDING: f32 = 20.0;
/// The coloured line along the top of the panel.
const STRIPE: f32 = 6.0;
/// Height of the first line, which says where the error is.
const TITLE_SIZE: f32 = 30.0;
/// Height of every other line: the smallest text the UI draws.
const TEXT_SIZE: f32 = fossps_ui::SMALLEST_TEXT;
/// A line's height is this many times its text's.
const LINE_SPACING: f32 = 1.3;
/// Lines are wrapped a little short of the panel, because the widths
/// measured here are for text a hair larger or smaller than what is drawn.
const SLACK: f32 = 0.97;
/// The most lines shown. A longer error ends by pointing at the terminal.
const MOST_LINES: usize = 14;
/// The most characters of one part of an error (its message, its hint)
/// that are looked at. Scripts write their own messages, so one can be
/// any length.
const MOST_CHARACTERS: usize = 600;
/// The last line of an error too long to show whole.
const CUT_SHORT: &str = "The rest is in the terminal.";

const BACKGROUND: Color = Color::rgba(0.09, 0.09, 0.1, 0.94);
const RED: Color = Color::rgb(1.0, 0.33, 0.33);
const TITLE: Color = Color::rgb(1.0, 0.56, 0.52);
const SOURCE: Color = Color::rgb(0.98, 0.86, 0.5);
const MESSAGE: Color = Color::rgb(0.95, 0.95, 0.96);
const QUIET: Color = Color::rgb(0.72, 0.74, 0.78);

/// One line of the panel.
#[derive(Debug, Clone, PartialEq)]
struct Line {
    text: String,
    size: f32,
    color: Color,
}

/// What text is measured with: the font it is drawn in when there is a
/// window, and a guess on a server, where nothing is drawn.
fn measure() -> Box<dyn TextMeasure> {
    #[cfg(feature = "window")]
    {
        Box::new(fossps_render::Font::bundled())
    }
    #[cfg(not(feature = "window"))]
    {
        Box::new(fossps_ui::EstimatedText)
    }
}

/// Shows a script error over the game.
pub(crate) struct ErrorOverlay {
    /// The error on show and the window size it was laid out for.
    shown: Option<(ScriptError, [u32; 2])>,
    ui: Ui,
    measure: Box<dyn TextMeasure>,
}

impl fmt::Debug for ErrorOverlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ErrorOverlay")
            .field("shown", &self.shown)
            .finish_non_exhaustive()
    }
}

impl Default for ErrorOverlay {
    fn default() -> Self {
        Self {
            shown: None,
            ui: Ui::default(),
            measure: measure(),
        }
    }
}

impl ErrorOverlay {
    /// Draws `error` over this frame, or nothing when there is none.
    ///
    /// The panel is laid out again only when the error or the window's size
    /// changes, so showing the same error frame after frame asks for no
    /// memory.
    pub(crate) fn show(&mut self, error: Option<&ScriptError>, frame: &mut Frame) {
        let Some(error) = error else {
            self.shown = None;
            return;
        };
        let screen = frame.screen_size();
        let current = self
            .shown
            .as_ref()
            .is_some_and(|(shown, size)| shown == error && *size == screen);
        if !current {
            self.ui = panel(error, screen, self.measure.as_ref());
            self.shown = Some((error.clone(), screen));
        }
        self.ui.update(&UiInput::default(), 0.0, screen);
        frame.draw_ui(&self.ui);
    }
}

/// A UI that shows `error` in the top-left corner of a `screen` of that
/// many pixels.
fn panel(error: &ScriptError, screen: [u32; 2], measure: &dyn TextMeasure) -> Ui {
    let mut ui = Ui::default();
    // The UI lays out on a screen 1080 high and keeps its middle 90%.
    #[allow(
        clippy::cast_precision_loss,
        reason = "screens are far smaller than 2^24 pixels"
    )]
    let across = screen[0] as f32 * 1080.0 / screen[1].max(1) as f32;
    let wide = (across * 0.9).min(WIDEST);
    let lines = lines(error, (wide - 2.0 * PADDING) * SLACK, measure);

    let high: f32 = STRIPE
        + 2.0 * PADDING
        + lines
            .iter()
            .map(|line| line.size * LINE_SPACING)
            .sum::<f32>();
    ui.hud_rect(Anchor::TopLeft, [0.0, 0.0], [wide, high], BACKGROUND);
    ui.hud_rect(Anchor::TopLeft, [0.0, 0.0], [wide, STRIPE], RED);
    let mut down = STRIPE + PADDING;
    for line in &lines {
        let gap = line.size * (LINE_SPACING - 1.0) / 2.0;
        let text = ui.hud_text(
            Anchor::TopLeft,
            [PADDING, down + gap],
            line.size,
            &line.text,
        );
        // The widget is this UI's own HUD text, so colouring it cannot fail.
        let _ = ui.set_color(text, line.color);
        down += line.size * LINE_SPACING;
    }
    // Every piece is pinned by its left edge, so the UI's own guess at how
    // wide text is moves nothing.
    ui
}

/// The lines of the panel for `error`, none wider than `width`: where it
/// is, the line of the script, what went wrong, what to try and what the
/// engine did about it.
fn lines(error: &ScriptError, width: f32, measure: &dyn TextMeasure) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut add = |text: &str, size: f32, color: Color| {
        let mut wrapped = Vec::new();
        wrap(&clean(text), width, size, measure, &mut wrapped);
        lines.extend(wrapped.into_iter().map(|text| Line { text, size, color }));
    };

    let place = match error.line {
        Some(line) => format!("Script error in {}, line {line}", error.file),
        None => format!("Script error in {}", error.file),
    };
    add(&place, TITLE_SIZE, TITLE);
    if let (Some(line), Some(source)) = (error.line, &error.source_line) {
        add(&format!("{line} | {}", source.trim()), TEXT_SIZE, SOURCE);
    }
    add(&error.message, TEXT_SIZE, MESSAGE);
    if let Some(hint) = &error.hint {
        add(&format!("Hint: {hint}"), TEXT_SIZE, QUIET);
    }
    if let Some(note) = &error.note {
        add(note, TEXT_SIZE, QUIET);
    }

    if lines.len() > MOST_LINES {
        lines.truncate(MOST_LINES - 1);
        lines.push(Line {
            text: CUT_SHORT.to_owned(),
            size: TEXT_SIZE,
            color: QUIET,
        });
    }
    lines
}

/// `text` as one run of words that can be drawn: line breaks, tabs and
/// other characters with no shape become spaces, and text past
/// [`MOST_CHARACTERS`] is left out with `...` in its place.
fn clean(text: &str) -> String {
    let mut cleaned: String = text
        .chars()
        .take(MOST_CHARACTERS)
        .map(|character| {
            if character.is_control() || character.is_whitespace() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if text.chars().nth(MOST_CHARACTERS).is_some() {
        cleaned.push_str("...");
    }
    cleaned
}

/// Splits `text` into lines no wider than `width` when drawn `size` high,
/// between words where it can. A word wider than a whole line is split
/// wherever the line ends.
fn wrap(text: &str, width: f32, size: f32, measure: &dyn TextMeasure, lines: &mut Vec<String>) {
    let fits = |line: &str| measure.width(line, size) <= width;
    let mut line = String::new();
    for word in text.split_whitespace() {
        let before = line.len();
        if before > 0 {
            line.push(' ');
        }
        line.push_str(word);
        if fits(&line) {
            continue;
        }
        // The word goes on a line of its own.
        line.truncate(before);
        if before > 0 {
            lines.push(std::mem::take(&mut line));
        }
        for character in word.chars() {
            line.push(character);
            // A line holds at least one character, however narrow it is.
            if !fits(&line) && line.chars().nth(1).is_some() {
                line.pop();
                lines.push(std::mem::take(&mut line));
                line.push(character);
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
}

#[cfg(test)]
mod tests {
    use fossps_ui::{Draw, DrawBuffer, EstimatedText};

    use super::*;

    fn error() -> ScriptError {
        ScriptError {
            file: "main.luau".to_owned(),
            line: Some(4),
            message: "game.onTik does not exist. Did you mean game.onTick?".to_owned(),
            source_line: Some("game.onTik(function(step)".to_owned()),
            hint: Some("Check the spelling.".to_owned()),
            note: Some("The game keeps running.".to_owned()),
        }
    }

    fn wrapped(text: &str, width: f32) -> Vec<String> {
        let mut lines = Vec::new();
        // Every character is 0.55 * 20 = 11 wide.
        wrap(text, width, 20.0, &EstimatedText, &mut lines);
        lines
    }

    /// A box's top-left corner and its size.
    type Placed = ([f32; 2], [f32; 2]);

    /// The words and boxes of what a frame was asked to draw.
    fn drawn(buffer: &DrawBuffer) -> (Vec<String>, Vec<Placed>) {
        let mut words = Vec::new();
        let mut boxes = Vec::new();
        for draw in buffer.draw() {
            match draw {
                Draw::Text { text, .. } => words.push(text.to_owned()),
                Draw::Rect { position, size, .. } => boxes.push((position, size)),
            }
        }
        (words, boxes)
    }

    #[test]
    fn text_is_wrapped_between_words() {
        // Ten characters fit in 110.
        assert_eq!(
            wrapped("one two three four", 110.0),
            ["one two", "three four"]
        );
        assert_eq!(wrapped("  spaced   out  ", 110.0), ["spaced out"]);
        assert!(wrapped("", 110.0).is_empty());
    }

    #[test]
    fn a_word_longer_than_a_line_is_split() {
        assert_eq!(
            wrapped("a abcdefghijklmnopqrstuvwxyz b", 110.0),
            ["a", "abcdefghij", "klmnopqrst", "uvwxyz b"]
        );
        // Even where nothing fits, each line takes a character and it ends.
        assert_eq!(wrapped("abc", 1.0), ["a", "b", "c"]);
        assert_eq!(wrapped("abc", f32::NAN), ["a", "b", "c"]);
    }

    #[test]
    fn text_that_cannot_be_drawn_is_made_plain_and_long_text_is_cut() {
        assert_eq!(clean("one\ttwo\nthree\u{7}"), "one two three ");
        let long = "é".repeat(MOST_CHARACTERS + 50);
        let cleaned = clean(&long);
        assert_eq!(cleaned.chars().count(), MOST_CHARACTERS + 3);
        assert!(cleaned.ends_with("é..."));
        // Text of exactly the limit is left whole.
        let whole = "x".repeat(MOST_CHARACTERS);
        assert_eq!(clean(&whole), whole);
    }

    #[test]
    fn the_lines_say_where_what_and_what_to_do_in_that_order() {
        let said: Vec<String> = lines(&error(), 2000.0, &EstimatedText)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert_eq!(
            said,
            [
                "Script error in main.luau, line 4",
                "4 | game.onTik(function(step)",
                "game.onTik does not exist. Did you mean game.onTick?",
                "Hint: Check the spelling.",
                "The game keeps running.",
            ]
        );

        let bare = ScriptError {
            line: None,
            source_line: None,
            hint: None,
            note: None,
            ..error()
        };
        let said: Vec<String> = lines(&bare, 2000.0, &EstimatedText)
            .into_iter()
            .map(|line| line.text)
            .collect();
        assert_eq!(
            said,
            [
                "Script error in main.luau",
                "game.onTik does not exist. Did you mean game.onTick?",
            ]
        );
    }

    #[test]
    fn an_error_too_long_to_show_ends_by_pointing_at_the_terminal() {
        let long = ScriptError {
            message: "word ".repeat(5000),
            ..error()
        };
        let lines = lines(&long, 300.0, &EstimatedText);
        assert_eq!(lines.len(), MOST_LINES);
        assert_eq!(lines[MOST_LINES - 1].text, CUT_SHORT);
        assert_eq!(lines[0].text, "Script error in");
    }

    #[test]
    fn the_panel_is_drawn_inside_the_window_whatever_its_shape() {
        let huge = ScriptError {
            message: "x".repeat(5000),
            hint: Some("y ".repeat(5000)),
            ..error()
        };
        for screen in [[1920, 1080], [1280, 720], [640, 1080], [3440, 1440], [1, 1]] {
            for error in [error(), huge.clone()] {
                let mut overlay = ErrorOverlay::default();
                let mut frame = Frame::new(0.0).for_window(Some(screen), DrawBuffer::default());
                overlay.show(Some(&error), &mut frame);
                let (words, boxes) = drawn(&frame.take_ui());
                assert!(words.len() >= 2, "{screen:?}");
                // The panel, then the stripe along its top.
                let (corner, size) = boxes[0];
                #[allow(clippy::cast_precision_loss)]
                let [width, height] = screen.map(|pixels| pixels as f32);
                assert!(corner[0] >= 0.0 && corner[1] >= 0.0, "{screen:?}");
                assert!(corner[0] + size[0] <= width + 1.0, "{screen:?}");
                assert!(corner[1] + size[1] <= height + 1.0, "{screen:?}");
                assert_eq!(boxes[1].0, corner, "{screen:?}");
            }
        }
    }

    #[test]
    fn the_error_is_drawn_until_there_is_none() {
        let mut overlay = ErrorOverlay::default();
        let mut frame = Frame::new(0.0);
        overlay.show(Some(&error()), &mut frame);
        let (words, boxes) = drawn(&frame.take_ui());
        assert_eq!(words[0], "Script error in main.luau, line 4");
        assert_eq!(words.len(), 5);
        assert_eq!(boxes.len(), 2);

        // The same error on the next frame is drawn again, and a new one
        // takes its place.
        let other = ScriptError {
            line: Some(9),
            ..error()
        };
        for (error, title) in [
            (error(), "Script error in main.luau, line 4"),
            (other, "Script error in main.luau, line 9"),
        ] {
            let mut frame = Frame::new(0.0);
            overlay.show(Some(&error), &mut frame);
            assert_eq!(drawn(&frame.take_ui()).0[0], title);
        }

        let mut frame = Frame::new(0.0);
        overlay.show(None, &mut frame);
        assert!(frame.take_ui().draw().is_empty());
    }

    #[test]
    fn a_resized_window_gets_the_panel_laid_out_again() {
        let mut overlay = ErrorOverlay::default();
        let long = ScriptError {
            message: "several words that need more than one line when narrow ".repeat(3),
            ..error()
        };
        let mut lines_at = |screen| {
            let mut frame = Frame::new(0.0).for_window(Some(screen), DrawBuffer::default());
            overlay.show(Some(&long), &mut frame);
            drawn(&frame.take_ui()).0.len()
        };
        let wide = lines_at([1920, 1080]);
        let narrow = lines_at([700, 1080]);
        assert!(narrow > wide, "{narrow} lines narrow, {wide} wide");
        assert_eq!(lines_at([1920, 1080]), wide);
    }
}
