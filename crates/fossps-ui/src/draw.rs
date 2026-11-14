//! What to draw this frame: filled boxes and pieces of text.

use std::fmt::{self, Write};

use crate::node::Rect;
use crate::theme::Color;

/// One thing to draw. Positions and sizes are in real pixels, measured
/// from the top-left corner of the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Draw<'a> {
    /// A filled box.
    Rect {
        /// The top-left corner.
        position: [f32; 2],
        /// Width and height.
        size: [f32; 2],
        /// The fill colour.
        color: Color,
        /// How round the corners are; 0 is square.
        corner: f32,
    },
    /// One line of text.
    Text {
        /// The top-left corner of the line.
        position: [f32; 2],
        /// The height of the text.
        size: f32,
        /// The colour of the text.
        color: Color,
        /// The words.
        text: &'a str,
    },
}

/// A draw command as stored: text is a range into one shared string, so
/// the list can be rebuilt every frame without asking for memory.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Command {
    Rect {
        position: [f32; 2],
        size: [f32; 2],
        color: Color,
        corner: f32,
    },
    Text {
        position: [f32; 2],
        size: f32,
        color: Color,
        start: usize,
        end: usize,
    },
}

/// Everything to draw this frame, back to front.
///
/// ```
/// use fossps_ui::{Anchor, Draw, Ui, UiInput};
///
/// let mut ui = Ui::default();
/// ui.hud_text(Anchor::BottomRight, [0.0, 0.0], 32.0, "30 / 90");
/// ui.update(&UiInput::default(), 1.0 / 60.0, [1920, 1080]);
///
/// for draw in ui.draw() {
///     match draw {
///         Draw::Rect { position, size, .. } => println!("box at {position:?}, {size:?}"),
///         Draw::Text { position, text, .. } => println!("\"{text}\" at {position:?}"),
///     }
/// }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct DrawList<'a> {
    pub(crate) commands: &'a [Command],
    pub(crate) text: &'a str,
}

impl<'a> DrawList<'a> {
    /// How many things there are to draw.
    #[must_use]
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Whether there is nothing to draw.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Each thing to draw, back to front.
    #[must_use]
    pub fn iter(&self) -> DrawIter<'a> {
        DrawIter {
            commands: self.commands.iter(),
            text: self.text,
        }
    }
}

impl<'a> IntoIterator for DrawList<'a> {
    type Item = Draw<'a>;
    type IntoIter = DrawIter<'a>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

/// Walks a [`DrawList`] back to front.
#[derive(Debug, Clone)]
pub struct DrawIter<'a> {
    commands: std::slice::Iter<'a, Command>,
    text: &'a str,
}

impl<'a> Iterator for DrawIter<'a> {
    type Item = Draw<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(match *self.commands.next()? {
            Command::Rect {
                position,
                size,
                color,
                corner,
            } => Draw::Rect {
                position,
                size,
                color,
                corner,
            },
            Command::Text {
                position,
                size,
                color,
                start,
                end,
            } => Draw::Text {
                position,
                size,
                color,
                text: self.text.get(start..end).unwrap_or(""),
            },
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.commands.size_hint()
    }
}

impl ExactSizeIterator for DrawIter<'_> {}

/// Adds to the draw list, turning reference pixels into real ones.
pub(crate) struct Painter<'a> {
    pub commands: &'a mut Vec<Command>,
    pub text: &'a mut String,
    pub scale: f32,
}

impl Painter<'_> {
    pub(crate) fn rect(&mut self, rect: Rect, color: Color, corner: f32) {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let scale = self.scale;
        self.commands.push(Command::Rect {
            position: [rect.x * scale, rect.y * scale],
            size: [rect.w * scale, rect.h * scale],
            color,
            corner: corner * scale,
        });
    }

    pub(crate) fn text(&mut self, position: [f32; 2], size: f32, color: Color, text: &str) {
        self.text_with(position, size, color, format_args!("{text}"));
    }

    /// Like `text`, for words made on the spot such as a slider's number.
    pub(crate) fn text_with(
        &mut self,
        position: [f32; 2],
        size: f32,
        color: Color,
        words: fmt::Arguments<'_>,
    ) {
        let start = self.text.len();
        // Writing to a String cannot fail.
        let _ = self.text.write_fmt(words);
        let end = self.text.len();
        if end == start {
            return;
        }
        let scale = self.scale;
        self.commands.push(Command::Text {
            position: [position[0] * scale, position[1] * scale],
            size: size * scale,
            color,
            start,
            end,
        });
    }
}
