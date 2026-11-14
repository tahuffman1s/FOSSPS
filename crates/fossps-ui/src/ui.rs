//! The HUD and menus of one game window.

use crate::build::{ScreenBuilder, index, snap};
use crate::draw::{Command, DrawList, Painter};
use crate::error::UiError;
use crate::focus;
use crate::input::{Direction, InputMode, Repeater, UiInput, held_direction};
use crate::layout::{measure_screen, place_hud, place_screen, slider_track, tab_width};
use crate::node::{Anchor, Kind, NONE, Node, Rect, Screen, ScreenId, WidgetId};
use crate::paint::{self, Look};
use crate::theme::{Color, EstimatedText, TextMeasure, Theme, UI_SCALE_RANGE};

/// Sizes are given for a screen this many pixels high.
const REFERENCE_HEIGHT: f32 = 1080.0;
/// The most events one frame can produce, with room to spare.
const MOST_EVENTS: usize = 16;

/// Something the player did in a menu this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UiEvent {
    /// A button was chosen.
    Pressed(WidgetId),
    /// A toggle, slider, choice or tab bar now has this value.
    Changed(WidgetId, Value),
    /// The player pressed back on this screen. The screen has already
    /// closed, unless it was built with
    /// [`stays_open`](ScreenBuilder::stays_open).
    Back(ScreenId),
}

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// A toggle: on or off.
    Toggle(bool),
    /// A slider's number.
    Slider(f32),
    /// Which option of a choice is picked, counting from 0.
    Choice(usize),
    /// Which tab is open, counting from 0.
    Tab(usize),
}

/// The HUD and menus of one game window.
///
/// Build the HUD pieces and menu screens once, then call
/// [`update`](Self::update) every frame and hand [`draw`](Self::draw) to
/// the renderer. See the [crate docs](crate) for a full example.
pub struct Ui {
    theme: Theme,
    ui_scale: f32,
    measure: Box<dyn TextMeasure>,
    nodes: Vec<Node>,
    screens: Vec<Screen>,
    hud: Vec<u32>,
    /// The open screens, the one on show last.
    stack: Vec<u32>,
    mode: InputMode,
    /// The widget under the pointer.
    hovered: u32,
    /// The widget the mouse button went down on.
    pressed: u32,
    /// The mouse button went down on a slider's track and is still held.
    dragging: bool,
    repeater: Repeater,
    previous: UiInput,
    events: Vec<UiEvent>,
    commands: Vec<Command>,
    draw_text: String,
    /// Real pixels per reference pixel.
    scale: f32,
    /// The screen in reference pixels.
    size: [f32; 2],
}

impl Default for Ui {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

impl Ui {
    /// An empty UI that will look like `theme`.
    #[must_use]
    pub fn new(theme: Theme) -> Self {
        Self {
            theme,
            ui_scale: 1.0,
            measure: Box::new(EstimatedText),
            nodes: Vec::new(),
            screens: Vec::new(),
            hud: Vec::new(),
            stack: Vec::new(),
            mode: InputMode::default(),
            hovered: NONE,
            pressed: NONE,
            dragging: false,
            repeater: Repeater::default(),
            previous: UiInput::default(),
            events: Vec::with_capacity(MOST_EVENTS),
            commands: Vec::new(),
            draw_text: String::new(),
            scale: 1.0,
            size: [1920.0, REFERENCE_HEIGHT],
        }
    }

    /// The player's own size setting: 1 is normal, and it is kept
    /// between 0.75 and 1.5.
    pub fn set_ui_scale(&mut self, scale: f32) {
        let (smallest, largest) = UI_SCALE_RANGE;
        self.ui_scale = if scale.is_finite() {
            scale.clamp(smallest, largest)
        } else {
            1.0
        };
    }

    /// Uses real text widths, once the renderer has a font to measure with.
    pub fn set_text_measure(&mut self, measure: Box<dyn TextMeasure>) {
        self.measure = measure;
    }

    // ---- Building -------------------------------------------------------

    /// Adds a menu screen with `title` at the top and the widgets that
    /// `build` adds under it. The screen stays hidden until
    /// [`open`](Self::open) shows it.
    ///
    /// # Errors
    ///
    /// The first mistake made inside `build`, such as a slider whose
    /// lowest value is above its highest. The screen is not added.
    pub fn add_screen(
        &mut self,
        title: &str,
        build: impl FnOnce(&mut ScreenBuilder<'_>),
    ) -> Result<ScreenId, UiError> {
        let first = self.nodes.len();
        let gap = self.theme.gap;
        let mut builder = ScreenBuilder {
            nodes: &mut self.nodes,
            gap,
            root: index(first),
            parent: NONE,
            error: None,
            wrap: true,
            stays_open: false,
            start: NONE,
        };
        builder.parent = builder.add(Kind::Column { gap }, "");
        if !title.is_empty() {
            builder.add(Kind::Label { title: true }, title);
        }
        build(&mut builder);
        let ScreenBuilder {
            root,
            error,
            wrap,
            stays_open,
            start,
            ..
        } = builder;
        if let Some(error) = error {
            // The half-built widgets stay in the list, unreachable, so the
            // ids handed out inside `build` can never name a later widget.
            return Err(error);
        }
        let end = index(self.nodes.len());
        let start = if (root..end).contains(&start) {
            start
        } else {
            NONE
        };
        self.screens.push(Screen {
            root,
            end,
            focus: NONE,
            start,
            wrap,
            stays_open,
        });
        self.stack.reserve(self.screens.len());
        self.reserve();
        Ok(ScreenId(index(self.screens.len() - 1)))
    }

    /// Adds text to the HUD, such as an ammo count.
    ///
    /// The text is pinned to `anchor` and then moved by `offset`:
    /// `[right, down]` in pixels on a screen 1080 high. `size` is the
    /// text's height in the same units, and is raised to
    /// [`SMALLEST_TEXT`](crate::SMALLEST_TEXT) if it is below it.
    pub fn hud_text(
        &mut self,
        anchor: Anchor,
        offset: [f32; 2],
        size: f32,
        text: &str,
    ) -> WidgetId {
        let color = self.theme.text;
        self.add_hud(
            Kind::HudText {
                anchor,
                offset,
                size,
                color,
            },
            [0.0; 2],
            text,
        )
    }

    /// Adds a bar to the HUD, such as health. `fraction` is how full it
    /// is, from 0 to 1; `size` is its `[width, height]` when full.
    pub fn hud_bar(
        &mut self,
        anchor: Anchor,
        offset: [f32; 2],
        size: [f32; 2],
        color: Color,
        fraction: f32,
    ) -> WidgetId {
        let fraction = clean_fraction(fraction);
        self.add_hud(
            Kind::HudBar {
                anchor,
                offset,
                fraction,
                color,
            },
            size,
            "",
        )
    }

    /// Adds a plain box to the HUD, such as one arm of a crosshair.
    pub fn hud_rect(
        &mut self,
        anchor: Anchor,
        offset: [f32; 2],
        size: [f32; 2],
        color: Color,
    ) -> WidgetId {
        self.add_hud(
            Kind::HudRect {
                anchor,
                offset,
                color,
            },
            size,
            "",
        )
    }

    fn add_hud(&mut self, kind: Kind, size: [f32; 2], text: &str) -> WidgetId {
        let new = index(self.nodes.len());
        let mut node = Node::new(kind, text);
        node.size = size;
        self.nodes.push(node);
        self.hud.push(new);
        self.reserve();
        WidgetId(new)
    }

    /// Makes sure a frame's drawing never has to ask for more memory.
    fn reserve(&mut self) {
        let (mut commands, mut text) = (2, 0);
        for node in &self.nodes {
            let (boxes, letters) = match &node.kind {
                Kind::Tabs { names, .. } => (3 * names.len(), names.iter().map(String::len).sum()),
                Kind::Choice { options, .. } => {
                    (8, options.iter().map(String::len).max().unwrap_or(0))
                }
                _ => (8, 0),
            };
            commands += boxes;
            text += letters + node.text.capacity() + 64;
        }
        self.commands.reserve(commands);
        self.draw_text.reserve(text);
    }

    // ---- The menu stack -------------------------------------------------

    /// Shows a screen on top of whatever is open. If it is already open
    /// underneath, the screens above it close instead.
    ///
    /// # Errors
    ///
    /// [`UiError::UnknownScreen`] if `screen` came from another `Ui`.
    pub fn open(&mut self, screen: ScreenId) -> Result<(), UiError> {
        if screen.0 as usize >= self.screens.len() {
            return Err(UiError::UnknownScreen);
        }
        match self.stack.iter().position(|&open| open == screen.0) {
            Some(place) => self.stack.truncate(place + 1),
            None => self.stack.push(screen.0),
        }
        self.pointer_left();
        Ok(())
    }

    /// Closes the screen on show, revealing the one under it. Returns
    /// false if no menu was open.
    pub fn close(&mut self) -> bool {
        self.pointer_left();
        self.stack.pop().is_some()
    }

    /// Closes every screen, leaving only the HUD.
    pub fn close_all(&mut self) {
        self.pointer_left();
        self.stack.clear();
    }

    /// The screen on show, if a menu is open.
    #[must_use]
    pub fn open_screen(&self) -> Option<ScreenId> {
        self.stack.last().map(|&screen| ScreenId(screen))
    }

    /// Whether a menu is open. While one is, the game should stop
    /// sending the player's input to the match.
    #[must_use]
    pub fn is_menu_open(&self) -> bool {
        !self.stack.is_empty()
    }

    /// The widget the player is on, if a menu is open and has one.
    #[must_use]
    pub fn focused(&self) -> Option<WidgetId> {
        let focus = self.screens[*self.stack.last()? as usize].focus;
        (focus != NONE).then_some(WidgetId(focus))
    }

    /// Whether the player last used the mouse, or a gamepad or keyboard.
    #[must_use]
    pub fn mode(&self) -> InputMode {
        self.mode
    }

    // ---- Changing widgets -----------------------------------------------

    fn node(&mut self, widget: WidgetId) -> Result<&mut Node, UiError> {
        self.nodes
            .get_mut(widget.0 as usize)
            .ok_or(UiError::UnknownWidget)
    }

    /// Changes the words on a label, button, setting or HUD text.
    ///
    /// Text no longer than the widget has held before is copied in
    /// place, so updating an ammo count every frame costs nothing.
    ///
    /// # Errors
    ///
    /// If the widget is not from this `Ui`, or has no words.
    pub fn set_text(&mut self, widget: WidgetId, text: &str) -> Result<(), UiError> {
        let node = self.node(widget)?;
        match node.kind {
            Kind::Label { .. }
            | Kind::Button
            | Kind::Toggle { .. }
            | Kind::Slider { .. }
            | Kind::Choice { .. }
            | Kind::HudText { .. } => {}
            _ => {
                return Err(UiError::WrongKind {
                    expected: "widget with text",
                    found: node.kind.name(),
                });
            }
        }
        let grows = text.len() > node.text.capacity();
        node.text.clear();
        node.text.push_str(text);
        if grows {
            self.reserve();
        }
        Ok(())
    }

    /// Changes the colour of a HUD text, bar or rectangle.
    ///
    /// # Errors
    ///
    /// If the widget is not from this `Ui`, or is not a HUD piece.
    pub fn set_color(&mut self, widget: WidgetId, new: Color) -> Result<(), UiError> {
        let node = self.node(widget)?;
        match &mut node.kind {
            Kind::HudText { color, .. }
            | Kind::HudBar { color, .. }
            | Kind::HudRect { color, .. } => {
                *color = new;
                Ok(())
            }
            other => Err(UiError::WrongKind {
                expected: "HUD piece",
                found: other.name(),
            }),
        }
    }

    /// Changes how full a HUD bar is, from 0 to 1.
    ///
    /// # Errors
    ///
    /// If the widget is not from this `Ui`, or is not a HUD bar.
    pub fn set_bar(&mut self, widget: WidgetId, new: f32) -> Result<(), UiError> {
        match &mut self.node(widget)?.kind {
            Kind::HudBar { fraction, .. } => {
                *fraction = clean_fraction(new);
                Ok(())
            }
            other => Err(UiError::WrongKind {
                expected: "HUD bar",
                found: other.name(),
            }),
        }
    }

    /// Turns a toggle on or off. No event is sent.
    ///
    /// # Errors
    ///
    /// If the widget is not from this `Ui`, or is not a toggle.
    pub fn set_toggle(&mut self, widget: WidgetId, new: bool) -> Result<(), UiError> {
        match &mut self.node(widget)?.kind {
            Kind::Toggle { on } => {
                *on = new;
                Ok(())
            }
            other => Err(UiError::WrongKind {
                expected: "toggle",
                found: other.name(),
            }),
        }
    }

    /// Sets a slider, moving `new` to the nearest value the slider can
    /// hold. No event is sent.
    ///
    /// # Errors
    ///
    /// If the widget is not from this `Ui`, or is not a slider.
    pub fn set_slider(&mut self, widget: WidgetId, new: f32) -> Result<(), UiError> {
        match &mut self.node(widget)?.kind {
            Kind::Slider {
                min,
                max,
                step,
                value,
            } => {
                *value = snap(*min, *max, *step, new);
                Ok(())
            }
            other => Err(UiError::WrongKind {
                expected: "slider",
                found: other.name(),
            }),
        }
    }

    /// Picks an option of a choice, counting from 0. A number past the
    /// end picks the last option. No event is sent.
    ///
    /// # Errors
    ///
    /// If the widget is not from this `Ui`, or is not a choice.
    pub fn set_choice(&mut self, widget: WidgetId, new: usize) -> Result<(), UiError> {
        match &mut self.node(widget)?.kind {
            Kind::Choice { options, selected } => {
                *selected = new.min(options.len().saturating_sub(1));
                Ok(())
            }
            other => Err(UiError::WrongKind {
                expected: "choice",
                found: other.name(),
            }),
        }
    }

    /// Opens a tab of a tab bar, counting from 0. A number past the end
    /// opens the last tab. No event is sent.
    ///
    /// # Errors
    ///
    /// If the widget is not from this `Ui`, or is not a tab bar.
    pub fn set_tab(&mut self, widget: WidgetId, new: usize) -> Result<(), UiError> {
        match &mut self.node(widget)?.kind {
            Kind::Tabs { names, open } => {
                *open = new.min(names.len().saturating_sub(1));
                Ok(())
            }
            other => Err(UiError::WrongKind {
                expected: "tab bar",
                found: other.name(),
            }),
        }
    }

    /// What a toggle, slider, choice or tab bar holds right now, or
    /// `None` for any other widget.
    #[must_use]
    pub fn value(&self, widget: WidgetId) -> Option<Value> {
        match &self.nodes.get(widget.0 as usize)?.kind {
            Kind::Toggle { on } => Some(Value::Toggle(*on)),
            Kind::Slider { value, .. } => Some(Value::Slider(*value)),
            Kind::Choice { selected, .. } => Some(Value::Choice(*selected)),
            Kind::Tabs { open, .. } => Some(Value::Tab(*open)),
            _ => None,
        }
    }

    // ---- Every frame ----------------------------------------------------

    /// Moves the UI on by one frame and says what the player did.
    ///
    /// Call this every frame, menu open or not, with what the player is
    /// holding, the seconds since the last frame, and the window's
    /// `[width, height]` in pixels. Then [`draw`](Self::draw) has this
    /// frame's picture.
    pub fn update(&mut self, input: &UiInput, dt: f32, screen: [u32; 2]) -> &[UiEvent] {
        self.events.clear();
        #[allow(
            clippy::cast_precision_loss,
            reason = "screens are far smaller than 2^24 pixels"
        )]
        let [width, height] = [screen[0] as f32, screen[1].max(1) as f32];
        self.scale = height / REFERENCE_HEIGHT * self.ui_scale;
        self.size = [width / self.scale, height / self.scale];

        self.layout();
        self.handle(input, dt);
        self.layout();
        self.paint();
        &self.events
    }

    /// What to draw this frame, back to front, in real pixels.
    #[must_use]
    pub fn draw(&self) -> DrawList<'_> {
        DrawList {
            commands: &self.commands,
            text: &self.draw_text,
        }
    }

    fn layout(&mut self) {
        let margin = self.theme.safe_margin.clamp(0.0, 0.25);
        let [width, height] = self.size;
        let safe = Rect {
            x: width * margin,
            y: height * margin,
            w: width * (1.0 - 2.0 * margin),
            h: height * (1.0 - 2.0 * margin),
        };
        for &piece in &self.hud {
            place_hud(&mut self.nodes[piece as usize], safe, self.measure.as_ref());
        }

        let Some(&top) = self.stack.last() else {
            return;
        };
        let screen = &mut self.screens[top as usize];
        let (first, end) = (screen.root as usize, screen.end as usize);
        measure_screen(
            &mut self.nodes,
            first,
            end,
            &self.theme,
            self.measure.as_ref(),
        );
        let root = &mut self.nodes[first];
        let [w, h] = root.size;
        root.rect = Rect {
            x: (width - w) / 2.0,
            y: (height - h) / 2.0,
            w,
            h,
        };
        root.visible = true;
        place_screen(&mut self.nodes, first, end, &self.theme);

        // The player is always on something they can see.
        let can_land = |widget: u32| widget != NONE && self.nodes[widget as usize].focusable();
        if !can_land(screen.focus) {
            screen.focus = if can_land(screen.start) {
                screen.start
            } else {
                (screen.root..screen.end)
                    .find(|&widget| can_land(widget))
                    .unwrap_or(NONE)
            };
        }
    }

    fn handle(&mut self, input: &UiInput, dt: f32) {
        let before = std::mem::replace(&mut self.previous, *input);
        let held = held_direction(input.direction);
        let Some(&top) = self.stack.last() else {
            self.repeater.wait_for_release(held);
            self.pointer_left();
            return;
        };
        let step = self.repeater.step(held, dt);
        let Screen { root, end, .. } = self.screens[top as usize];

        // The mouse.
        let clicked = input.pointer_down && !before.pointer_down;
        let let_go = !input.pointer_down && before.pointer_down;
        let moved = match (input.pointer, before.pointer) {
            (Some(now), Some(then)) => {
                (now[0] - then[0]).abs() > 0.5 || (now[1] - then[1]).abs() > 0.5
            }
            _ => false,
        };
        let pointer = input.pointer.map(|[x, y]| [x / self.scale, y / self.scale]);
        if (moved || clicked) && pointer.is_some() {
            self.mode = InputMode::Pointer;
        }
        if self.mode == InputMode::Pointer
            && let Some(pointer) = pointer
        {
            self.hovered = (root..end)
                .rev()
                .find(|&widget| {
                    let node = &self.nodes[widget as usize];
                    node.focusable() && node.rect.contains(pointer)
                })
                .unwrap_or(NONE);
            if self.hovered != NONE {
                self.screens[top as usize].focus = self.hovered;
            }
            if clicked {
                self.pressed = self.hovered;
                self.dragging = self.on_slider_track(self.hovered, pointer);
                if self.hovered == NONE {
                    self.click_tab(root, end, pointer);
                }
            }
            if input.pointer_down && self.dragging {
                self.drag_slider(self.pressed, pointer[0]);
            }
        } else {
            // The pointer has left the window, or is not in use.
            self.hovered = NONE;
        }
        if let_go {
            let pressed = std::mem::replace(&mut self.pressed, NONE);
            if pressed != NONE && pressed == self.hovered && self.mode == InputMode::Pointer {
                self.activate(pressed);
            }
        }

        // The gamepad and keyboard.
        let confirm = input.confirm && !before.confirm;
        if self.mode == InputMode::Pointer && (step.is_some() || confirm) {
            // The first press only brings the outline back, so the player
            // sees where they are before anything moves or is chosen.
            self.mode = InputMode::Focus;
            self.pointer_left();
        } else {
            if let Some(direction) = step {
                self.go(top, direction);
            }
            if confirm {
                self.activate(self.screens[top as usize].focus);
            }
        }
        let tab_step = i32::from(input.next_tab && !before.next_tab)
            - i32::from(input.previous_tab && !before.previous_tab);
        if tab_step != 0
            && let Some(tabs) = (root..end).find(|&widget| {
                let node = &self.nodes[widget as usize];
                node.visible && matches!(node.kind, Kind::Tabs { .. })
            })
        {
            self.nudge(tabs, tab_step);
            // Whatever the mouse was holding may now be on a hidden page.
            self.pointer_left();
        }
        if input.back && !before.back {
            self.push_event(UiEvent::Back(ScreenId(top)));
            if !self.screens[top as usize].stays_open {
                self.stack.pop();
                self.pointer_left();
            }
        }
    }

    /// Forgets what the pointer was over or holding.
    fn pointer_left(&mut self) {
        self.hovered = NONE;
        self.pressed = NONE;
        self.dragging = false;
    }

    /// Whether `pointer` is on the part of a slider that can be dragged:
    /// its track, as tall as the whole row and a little wider.
    fn on_slider_track(&self, widget: u32, pointer: [f32; 2]) -> bool {
        let Some(node) = self.nodes.get(widget as usize) else {
            return false;
        };
        let track = slider_track(node.rect, &self.theme);
        let reach = Rect {
            x: track.x - 12.0,
            y: node.rect.y,
            w: track.w + 24.0,
            h: node.rect.h,
        };
        matches!(node.kind, Kind::Slider { .. }) && reach.contains(pointer)
    }

    fn push_event(&mut self, event: UiEvent) {
        // Never grows: a frame cannot produce more than a handful.
        if self.events.len() < self.events.capacity() {
            self.events.push(event);
        }
    }

    /// Handles a direction press on the open screen.
    fn go(&mut self, top: u32, direction: Direction) {
        let screen = &self.screens[top as usize];
        let focus = screen.focus;
        if focus == NONE {
            return;
        }
        let node = &self.nodes[focus as usize];
        let sideways = match direction {
            Direction::Left => -1,
            Direction::Right => 1,
            Direction::Up | Direction::Down => 0,
        };
        if sideways != 0 && matches!(node.kind, Kind::Slider { .. } | Kind::Choice { .. }) {
            self.nudge(focus, sideways);
            return;
        }
        let chosen = node.neighbors[direction.index()];
        let next = if chosen != NONE && self.nodes[chosen as usize].focusable() {
            Some(chosen)
        } else {
            let (first, end) = (screen.root as usize, screen.end as usize);
            focus::next(
                &self.nodes,
                first,
                end,
                focus as usize,
                direction,
                screen.wrap,
            )
            .map(index)
        };
        if let Some(next) = next {
            self.screens[top as usize].focus = next;
        }
    }

    /// Chooses a widget: presses a button, flips a toggle, or moves a
    /// choice on by one.
    fn activate(&mut self, widget: u32) {
        let Some(node) = self.nodes.get_mut(widget as usize) else {
            return;
        };
        match &mut node.kind {
            Kind::Button => self.push_event(UiEvent::Pressed(WidgetId(widget))),
            Kind::Toggle { on } => {
                *on = !*on;
                let event = UiEvent::Changed(WidgetId(widget), Value::Toggle(*on));
                self.push_event(event);
            }
            Kind::Choice { .. } => self.nudge(widget, 1),
            _ => {}
        }
    }

    /// Moves a slider by a step, or a choice or tab bar to the next or
    /// previous entry, going round at the ends.
    fn nudge(&mut self, widget: u32, by: i32) {
        let around = |now: usize, count: usize| {
            if count == 0 {
                0
            } else if by > 0 {
                (now + 1) % count
            } else {
                (now + count - 1) % count
            }
        };
        let id = WidgetId(widget);
        let event = match &mut self.nodes[widget as usize].kind {
            Kind::Slider {
                min,
                max,
                step,
                value,
            } => {
                // Counted in whole steps from the lowest value, so a tiny
                // step on a huge number still moves.
                let steps = ((*value - *min) / *step).round() + if by > 0 { 1.0 } else { -1.0 };
                let new = (*min + steps * *step).clamp(*min, *max);
                let changed = new.to_bits() != value.to_bits();
                *value = new;
                changed.then_some(UiEvent::Changed(id, Value::Slider(new)))
            }
            Kind::Choice { options, selected } => {
                let new = around(*selected, options.len());
                let changed = new != *selected;
                *selected = new;
                changed.then_some(UiEvent::Changed(id, Value::Choice(new)))
            }
            Kind::Tabs { names, open } => {
                let new = around(*open, names.len());
                let changed = new != *open;
                *open = new;
                changed.then_some(UiEvent::Changed(id, Value::Tab(new)))
            }
            _ => None,
        };
        if let Some(event) = event {
            self.push_event(event);
        }
    }

    /// Sets the slider being dragged from where the pointer is.
    fn drag_slider(&mut self, widget: u32, pointer_x: f32) {
        let node = &mut self.nodes[widget as usize];
        let track = slider_track(node.rect, &self.theme);
        let Kind::Slider {
            min,
            max,
            step,
            value,
        } = &mut node.kind
        else {
            return;
        };
        let along = ((pointer_x - track.x) / track.w).clamp(0.0, 1.0);
        let new = snap(*min, *max, *step, *min + along * (*max - *min));
        if new.to_bits() != value.to_bits() {
            *value = new;
            self.push_event(UiEvent::Changed(WidgetId(widget), Value::Slider(new)));
        }
    }

    /// Opens the tab under the pointer, if there is one.
    fn click_tab(&mut self, root: u32, end: u32, pointer: [f32; 2]) {
        for widget in root..end {
            let node = &mut self.nodes[widget as usize];
            if !node.visible {
                continue;
            }
            let rect = node.rect;
            let Kind::Tabs { names, open } = &mut node.kind else {
                continue;
            };
            let mut x = rect.x;
            for (tab, name) in names.iter().enumerate() {
                let w = tab_width(name, &self.theme, self.measure.as_ref());
                let hit = Rect {
                    x,
                    y: rect.y,
                    w,
                    h: self.theme.row_height,
                }
                .contains(pointer);
                if hit && tab != *open {
                    *open = tab;
                    self.push_event(UiEvent::Changed(WidgetId(widget), Value::Tab(tab)));
                    return;
                }
                x += w + self.theme.gap;
            }
        }
    }

    fn paint(&mut self) {
        self.commands.clear();
        self.draw_text.clear();
        let mut painter = Painter {
            commands: &mut self.commands,
            text: &mut self.draw_text,
            scale: self.scale,
        };
        for &piece in &self.hud {
            paint::hud(&self.nodes[piece as usize], &self.theme, &mut painter);
        }

        let Some(screen) = self
            .stack
            .last()
            .map(|&screen| &self.screens[screen as usize])
        else {
            return;
        };
        let [width, height] = self.size;
        painter.rect(
            Rect {
                x: 0.0,
                y: 0.0,
                w: width,
                h: height,
            },
            self.theme.backdrop,
            0.0,
        );
        let panel = self.nodes[screen.root as usize]
            .rect
            .grown(self.theme.padding * 2.0);
        painter.rect(panel, self.theme.panel, self.theme.corner * 2.0);
        for widget in screen.root..screen.end {
            let node = &self.nodes[widget as usize];
            if !node.visible {
                continue;
            }
            let on_it = widget == screen.focus;
            let look = match self.mode {
                InputMode::Focus => Look {
                    selected: on_it,
                    outlined: on_it,
                },
                InputMode::Pointer => Look {
                    selected: widget == self.hovered,
                    outlined: false,
                },
            };
            paint::widget(node, look, &self.theme, self.measure.as_ref(), &mut painter);
        }
    }
}

/// A bar's fullness, kept between empty and full.
fn clean_fraction(fraction: f32) -> f32 {
    if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    }
}
