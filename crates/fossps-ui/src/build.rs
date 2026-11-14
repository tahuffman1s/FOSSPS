//! Building a menu screen.

use crate::error::UiError;
use crate::input::Direction;
use crate::node::{Kind, NONE, Node, WidgetId};

/// The position a new node will have in a list `len` long.
pub(crate) fn index(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(NONE)
}

/// Adds widgets to a screen, top to bottom.
///
/// Handed to the function given to [`Ui::add_screen`](crate::Ui::add_screen).
/// Each widget method returns the [`WidgetId`] to keep if the widget
/// will be changed or listened to later.
pub struct ScreenBuilder<'a> {
    pub(crate) nodes: &'a mut Vec<Node>,
    pub(crate) gap: f32,
    pub(crate) root: u32,
    pub(crate) parent: u32,
    pub(crate) error: Option<UiError>,
    pub(crate) wrap: bool,
    pub(crate) stays_open: bool,
    pub(crate) start: u32,
}

impl ScreenBuilder<'_> {
    pub(crate) fn add(&mut self, kind: Kind, text: &str) -> u32 {
        let new = index(self.nodes.len());
        self.nodes.push(Node::new(kind, text));
        if self.parent != NONE {
            let parent = &mut self.nodes[self.parent as usize];
            let last = std::mem::replace(&mut parent.last_child, new);
            if last == NONE {
                parent.first_child = new;
            } else {
                self.nodes[last as usize].next_sibling = new;
            }
        }
        new
    }

    fn inside(&mut self, container: Kind, build: impl FnOnce(&mut Self)) {
        let outer = self.parent;
        self.parent = self.add(container, "");
        build(self);
        self.parent = outer;
    }

    fn fail(&mut self, error: UiError) {
        self.error.get_or_insert(error);
    }

    /// Stacks the widgets added inside `build` top to bottom. A screen is
    /// already a column, so this is only needed inside a row or grid.
    pub fn column(&mut self, build: impl FnOnce(&mut Self)) {
        self.inside(Kind::Column { gap: self.gap }, build);
    }

    /// Puts the widgets added inside `build` side by side, left to right.
    pub fn row(&mut self, build: impl FnOnce(&mut Self)) {
        self.inside(Kind::Row { gap: self.gap }, build);
    }

    /// Lays the widgets added inside `build` out in a grid `columns`
    /// wide, filling each row left to right.
    pub fn grid(&mut self, columns: u32, build: impl FnOnce(&mut Self)) {
        if columns == 0 {
            self.fail(UiError::NoColumns);
        }
        self.inside(
            Kind::Grid {
                columns: columns.max(1),
                gap: self.gap,
            },
            build,
        );
    }

    /// A line of text. The player cannot land on it.
    pub fn label(&mut self, text: &str) -> WidgetId {
        WidgetId(self.add(Kind::Label { title: false }, text))
    }

    /// A button. Choosing it sends [`UiEvent::Pressed`](crate::UiEvent::Pressed).
    pub fn button(&mut self, text: &str) -> WidgetId {
        WidgetId(self.add(Kind::Button, text))
    }

    /// A setting that is on or off. Choosing it flips it.
    pub fn toggle(&mut self, text: &str, on: bool) -> WidgetId {
        WidgetId(self.add(Kind::Toggle { on }, text))
    }

    /// A setting that is a number from `lowest` to `highest`, changed
    /// `step` at a time with left and right, or dragged with the mouse.
    pub fn slider(
        &mut self,
        text: &str,
        lowest: f32,
        highest: f32,
        step: f32,
        value: f32,
    ) -> WidgetId {
        let sound = lowest < highest && step > 0.0 && (highest - lowest).is_finite();
        if !sound {
            self.fail(UiError::BadSlider {
                label: text.to_owned(),
            });
        }
        let value = if sound {
            snap(lowest, highest, step, value)
        } else {
            lowest
        };
        WidgetId(self.add(
            Kind::Slider {
                min: lowest,
                max: highest,
                step,
                value,
            },
            text,
        ))
    }

    /// A setting that is one of a few named options, changed with left
    /// and right. `selected` counts from 0.
    pub fn choice(&mut self, text: &str, options: &[&str], selected: usize) -> WidgetId {
        if options.is_empty() {
            self.fail(UiError::NoOptions {
                label: text.to_owned(),
            });
        }
        let selected = selected.min(options.len().saturating_sub(1));
        let options = options.iter().map(|&option| option.to_owned()).collect();
        WidgetId(self.add(Kind::Choice { options, selected }, text))
    }

    /// A bar of tabs with a page of widgets under each.
    ///
    /// `build` is called once for every name, with the tab's number
    /// counting from 0, and adds that tab's widgets. The shoulder
    /// buttons switch tabs from anywhere on the screen.
    ///
    /// ```
    /// # use fossps_ui::Ui;
    /// # let mut ui = Ui::default();
    /// ui.add_screen("Settings", |screen| {
    ///     screen.tabs(&["Game", "Sound"], |screen, tab| match tab {
    ///         0 => {
    ///             screen.toggle("Show hints", true);
    ///         }
    ///         _ => {
    ///             screen.slider("Volume", 0.0, 100.0, 5.0, 80.0);
    ///         }
    ///     });
    /// })?;
    /// # Ok::<(), fossps_ui::UiError>(())
    /// ```
    pub fn tabs(&mut self, names: &[&str], mut build: impl FnMut(&mut Self, usize)) -> WidgetId {
        if names.is_empty() {
            self.fail(UiError::NoOptions {
                label: "tabs".to_owned(),
            });
        }
        let outer = self.parent;
        let owned = names.iter().map(|&name| name.to_owned()).collect();
        let tabs = self.add(
            Kind::Tabs {
                names: owned,
                open: 0,
            },
            "",
        );
        for tab in 0..names.len() {
            self.parent = tabs;
            self.parent = self.add(Kind::Column { gap: self.gap }, "");
            build(self, tab);
        }
        self.parent = outer;
        WidgetId(tabs)
    }

    /// Says where `direction` goes from `from`, for the rare layout
    /// where the automatic choice is not the one wanted.
    pub fn neighbor(&mut self, from: WidgetId, direction: Direction, to: WidgetId) {
        let here = |widget: WidgetId| {
            widget.0 >= self.root
                && self
                    .nodes
                    .get(widget.0 as usize)
                    .is_some_and(|node| node.kind.takes_focus())
        };
        if here(from) && here(to) {
            self.nodes[from.0 as usize].neighbors[direction.index()] = to.0;
        } else {
            self.fail(UiError::NeighborNotOnScreen);
        }
    }

    /// The widget the player is on when the screen first opens. Without
    /// this it is the first one they can land on.
    pub fn start_on(&mut self, widget: WidgetId) {
        self.start = widget.0;
    }

    /// Stops a direction press at the last widget instead of carrying on
    /// round to the first.
    pub fn no_wrap(&mut self) {
        self.wrap = false;
    }

    /// Keeps the screen open when the player presses back, for a main
    /// menu with nothing behind it. The press still sends
    /// [`UiEvent::Back`](crate::UiEvent::Back).
    pub fn stays_open(&mut self) {
        self.stays_open = true;
    }
}

/// The nearest value a slider can hold.
pub(crate) fn snap(lowest: f32, highest: f32, step: f32, value: f32) -> f32 {
    if !value.is_finite() {
        return lowest;
    }
    let steps = ((value - lowest) / step).round();
    (lowest + steps * step).clamp(lowest, highest)
}
