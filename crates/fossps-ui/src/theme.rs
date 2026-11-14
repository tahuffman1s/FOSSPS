//! How the HUD and menus look: colours, sizes and spacing.

/// A colour: red, green, blue and how solid it is, each from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    /// Red, from 0 to 1.
    pub r: f32,
    /// Green, from 0 to 1.
    pub g: f32,
    /// Blue, from 0 to 1.
    pub b: f32,
    /// How solid the colour is: 0 is invisible, 1 hides what is behind it.
    pub a: f32,
}

impl Color {
    /// White.
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);
    /// Black.
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);

    /// A solid colour.
    #[must_use]
    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// A see-through colour; `a` of 0 is invisible and 1 is solid.
    #[must_use]
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }
}

/// The smallest text the UI will draw, in the same units as every other
/// size: pixels on a screen 1080 high.
///
/// Smaller text cannot be read from a sofa or on a handheld, so sizes
/// below this are raised to it.
pub const SMALLEST_TEXT: f32 = 24.0;

/// The UI scale a player may choose: from three quarters to one and a half.
pub(crate) const UI_SCALE_RANGE: (f32, f32) = (0.75, 1.5);

/// Colours, sizes and spacing for menus.
///
/// Sizes are in pixels on a screen 1080 high; on any other screen
/// everything grows or shrinks to match, so a menu covers the same share
/// of a 720p handheld and a 4K television.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Text on labels, buttons and settings.
    pub text: Color,
    /// Text on the widget the player is on.
    pub text_selected: Color,
    /// The darkening drawn over the game while a menu is open.
    pub backdrop: Color,
    /// The panel behind a screen's widgets.
    pub panel: Color,
    /// A button or setting the player is not on.
    pub widget: Color,
    /// The button or setting the player is on.
    pub widget_selected: Color,
    /// The outline around the widget a gamepad or keyboard is on, the
    /// filled part of a slider, and the open tab.
    pub accent: Color,
    /// The empty part of a slider or a HUD bar.
    pub track: Color,
    /// Height of ordinary text.
    pub text_size: f32,
    /// Height of a screen's title.
    pub title_size: f32,
    /// Height of a button or setting.
    pub row_height: f32,
    /// Narrowest a button is drawn.
    pub button_width: f32,
    /// Width of a toggle, slider or choice.
    pub setting_width: f32,
    /// Space between a widget's edge and its text, and around a screen's
    /// widgets inside the panel.
    pub padding: f32,
    /// Space between widgets when a column, row or grid does not say.
    pub gap: f32,
    /// How round corners are.
    pub corner: f32,
    /// How far the outline reaches past the widget it surrounds.
    pub outline: f32,
    /// The share of each screen edge kept clear, because televisions cut
    /// off their edges: 0.05 keeps the middle 90%.
    pub safe_margin: f32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            text: Color::rgb(0.92, 0.93, 0.95),
            text_selected: Color::WHITE,
            backdrop: Color::rgba(0.0, 0.0, 0.0, 0.6),
            panel: Color::rgba(0.07, 0.08, 0.1, 0.92),
            widget: Color::rgb(0.16, 0.18, 0.22),
            widget_selected: Color::rgb(0.24, 0.28, 0.36),
            accent: Color::rgb(1.0, 0.72, 0.2),
            track: Color::rgb(0.3, 0.32, 0.38),
            text_size: 28.0,
            title_size: 44.0,
            row_height: 56.0,
            button_width: 360.0,
            setting_width: 640.0,
            padding: 20.0,
            gap: 12.0,
            corner: 8.0,
            outline: 4.0,
            safe_margin: 0.05,
        }
    }
}

/// Estimates how wide a piece of text is.
///
/// The UI needs widths to centre text and size buttons. The renderer
/// supplies real measurements from its font; until one is set,
/// [`EstimatedText`] is used.
pub trait TextMeasure {
    /// The width of `text` drawn `size` high, in the same units as `size`.
    fn width(&self, text: &str, size: f32) -> f32;
}

/// Guesses text width as a little over half the height for every character.
///
/// Good enough to lay a menu out before a font is loaded, and exact
/// enough for tests.
#[derive(Debug, Clone, Copy, Default)]
pub struct EstimatedText;

impl TextMeasure for EstimatedText {
    fn width(&self, text: &str, size: f32) -> f32 {
        let characters = text.chars().count() as f32;
        characters * size * 0.55
    }
}
