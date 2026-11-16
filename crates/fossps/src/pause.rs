//! The menu every game gets: `pause` opens it over the game, and it holds
//! the player's settings and the way out.
//!
//! The game underneath is not stopped. A match on a server goes on whether
//! or not one player has a menu open, and a game played alone runs through
//! the same netcode, so the menu is drawn over a world that keeps moving.
//! What stops is the player's own input: while the menu is open the `menu`
//! action set is live, so no gameplay action hears a key, and the mouse
//! points instead of turning the view.

use std::path::PathBuf;

use fossps_input::{ActionId, ActionMap, ActionSetId, InputFrame, Rumble};
use fossps_ui::{ScreenId, Ui, UiEvent, UiInput, Value, WidgetId};

use crate::game::{Frame, Game};
use crate::settings::{self, Settings};

/// The most menu events handled in one frame; the UI makes no more.
const MOST_EVENTS: usize = 16;

/// A game with the engine's pause menu over it.
pub(crate) struct PauseMenu<G> {
    game: G,
    /// `None` when the game's controls have no menu to open, or the game
    /// asked for no menu.
    menu: Option<Menu>,
    /// The HUD size the player chose, for a game with no menu to change it.
    ui_scale: f32,
}

impl<G: Game> PauseMenu<G> {
    /// Puts the pause menu over `game`, which reads `actions`. The menu
    /// starts with `settings` on its sliders and saves what the player
    /// changes to `file`, if there is one.
    ///
    /// The menu needs the `pause` action and the `menu` set of
    /// [`ActionMap::default_fps`]. A game whose controls have neither gets
    /// no menu, and is told how to get one in the log.
    pub(crate) fn over(
        game: G,
        actions: &ActionMap,
        settings: Settings,
        file: Option<PathBuf>,
    ) -> Self {
        let menu = Menu::new(actions, settings, file);
        if menu.is_none() {
            log::info!(
                "This game has no pause menu, because its controls have no `pause` action \
                 or no menu controls. Build the controls with `ActionMap::default_fps()` \
                 to get the menu. If the game has its own pause menu, set \
                 `Config::pause_menu` to false."
            );
        }
        Self {
            game,
            menu,
            ui_scale: settings.ui_scale(),
        }
    }

    /// Runs `game` with no menu over it. Its HUD is still the size the
    /// player chose in `settings`.
    pub(crate) fn off(game: G, settings: Settings) -> Self {
        Self {
            game,
            menu: None,
            ui_scale: settings.ui_scale(),
        }
    }
}

impl<G: Game> Game for PauseMenu<G> {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        if let Some(menu) = &mut self.menu {
            menu.hear(input);
        }
        // The game is ticked whether or not the menu is open: with the
        // `menu` set live, its own actions all read as let go.
        self.game.tick(input, step);
    }

    fn frame(&mut self, frame: &mut Frame) {
        let Some(menu) = &mut self.menu else {
            frame.set_ui_scale(self.ui_scale);
            self.game.frame(frame);
            return;
        };
        frame.set_ui_scale(menu.shown_scale);
        self.game.frame(frame);
        // Drawn after the game's own HUD, so on top of it.
        menu.show(frame);
    }
}

/// The actions the menu reads, looked up once.
#[derive(Debug, Clone, Copy)]
struct Actions {
    pause: ActionId,
    set: ActionSetId,
    walk: ActionId,
    confirm: ActionId,
    back: ActionId,
    previous_tab: ActionId,
    next_tab: ActionId,
    close: ActionId,
}

impl Actions {
    fn find(actions: &ActionMap) -> Option<Self> {
        let find = |name| actions.action(name).ok();
        let set = actions.set("menu").ok()?;
        let found = Self {
            pause: find("pause")?,
            set,
            walk: find("menu_move")?,
            confirm: find("menu_confirm")?,
            back: find("menu_back")?,
            previous_tab: find("menu_previous_tab")?,
            next_tab: find("menu_next_tab")?,
            close: find("menu_close")?,
        };
        // A `menu_back` that belongs to the gameplay set would never be
        // heard while the menu is open.
        let in_menu = [
            found.walk,
            found.confirm,
            found.back,
            found.previous_tab,
            found.next_tab,
            found.close,
        ]
        .iter()
        .all(|action| actions.set_of(*action) == Some(set));
        (in_menu && actions.set_of(found.pause) != Some(set)).then_some(found)
    }
}

/// What the menu actions were doing in the ticks since the last frame.
///
/// Ticks and frames do not line up: a slow frame spans several ticks and a
/// fast display draws several frames in one. A press that came and went
/// between two frames is kept until a frame has seen it.
#[derive(Debug, Clone, Copy, Default)]
struct Keys {
    /// What is held as of the last tick.
    held: UiInput,
    /// What was pressed in a tick no frame has seen yet.
    tapped: UiInput,
}

impl Keys {
    fn hear(&mut self, input: &InputFrame, actions: &Actions) {
        let direction = input.axis2d(actions.walk);
        self.held.direction = direction;
        if direction != [0.0, 0.0] {
            self.tapped.direction = direction;
        }
        for (action, held, tapped) in [
            (
                actions.confirm,
                &mut self.held.confirm,
                &mut self.tapped.confirm,
            ),
            (actions.back, &mut self.held.back, &mut self.tapped.back),
            (
                actions.previous_tab,
                &mut self.held.previous_tab,
                &mut self.tapped.previous_tab,
            ),
            (
                actions.next_tab,
                &mut self.held.next_tab,
                &mut self.tapped.next_tab,
            ),
        ] {
            *held = input.pressed(action);
            *tapped |= input.just_pressed(action);
        }
    }

    /// What a frame should see, after which the taps are forgotten.
    fn take(&mut self) -> UiInput {
        let tapped = std::mem::take(&mut self.tapped);
        let held = self.held;
        UiInput {
            direction: if held.direction == [0.0, 0.0] {
                tapped.direction
            } else {
                held.direction
            },
            confirm: held.confirm || tapped.confirm,
            back: held.back || tapped.back,
            previous_tab: held.previous_tab || tapped.previous_tab,
            next_tab: held.next_tab || tapped.next_tab,
            pointer: None,
            pointer_down: false,
        }
    }
}

/// The menu's screens and the widgets on them.
#[derive(Debug, Clone, Copy)]
struct Widgets {
    /// The first screen: resume, settings, quit.
    top: ScreenId,
    resume: WidgetId,
    open_settings: WidgetId,
    quit: WidgetId,
    /// The settings screen.
    settings: ScreenId,
    volume: WidgetId,
    sensitivity: WidgetId,
    invert_look: WidgetId,
    ui_size: WidgetId,
    reset: WidgetId,
    done: WidgetId,
    /// The screen that asks whether to quit.
    leaving: ScreenId,
    stay: WidgetId,
    leave: WidgetId,
}

impl Widgets {
    /// Builds the three screens in `ui`, with `settings` on the sliders.
    fn build(ui: &mut Ui, settings: &Settings) -> Option<Self> {
        let mut top = None;
        let top_screen = ui
            .add_screen("Menu", |screen| {
                top = Some([
                    screen.button("Resume"),
                    screen.button("Settings"),
                    screen.button("Quit"),
                ]);
            })
            .ok()?;
        let [resume, open_settings, quit] = top?;

        let mut sliders = None;
        let settings_screen = ui
            .add_screen("Settings", |screen| {
                let slider = |screen: &mut fossps_ui::ScreenBuilder<'_>,
                              name: &str,
                              range: settings::Range,
                              value: f32| {
                    screen.slider(name, range.lowest, range.highest, range.step, value)
                };
                sliders = Some([
                    slider(screen, "Volume", Settings::VOLUME, settings.volume),
                    slider(
                        screen,
                        "Look sensitivity",
                        Settings::SENSITIVITY,
                        settings.sensitivity,
                    ),
                    screen.toggle("Invert look", settings.invert_look),
                    slider(
                        screen,
                        "HUD and menu size",
                        Settings::UI_SIZE,
                        settings.ui_size,
                    ),
                    screen.button("Reset to defaults"),
                    screen.button("Back"),
                ]);
            })
            .ok()?;
        let [volume, sensitivity, invert_look, ui_size, reset, done] = sliders?;

        let mut answers = None;
        let leaving = ui
            .add_screen("Quit the game?", |screen| {
                // The safe answer is the one the player starts on.
                let stay = screen.button("Keep playing");
                screen.start_on(stay);
                answers = Some([stay, screen.button("Quit")]);
            })
            .ok()?;
        let [stay, leave] = answers?;

        Some(Self {
            top: top_screen,
            resume,
            open_settings,
            quit,
            settings: settings_screen,
            volume,
            sensitivity,
            invert_look,
            ui_size,
            reset,
            done,
            leaving,
            stay,
            leave,
        })
    }
}

/// The pause menu itself.
struct Menu {
    actions: Actions,
    keys: Keys,
    ui: Ui,
    widgets: Widgets,
    /// What the player has chosen.
    settings: Settings,
    /// What the settings file holds, as far as is known.
    saved: Settings,
    /// Where settings are saved; `None` keeps them for this run only.
    file: Option<PathBuf>,
    /// Whether `settings` has changed and has yet to be put into effect.
    /// The window itself starts with the settings the game was given.
    unapplied: bool,
    /// The size the menu is drawn at. It follows the setting, but not
    /// while the mouse is dragging the slider that sets it.
    shown_scale: f32,
    /// `pause` was pressed in a tick since the last frame.
    open_asked: bool,
    /// `menu_close` was pressed in a tick since the last frame.
    close_asked: bool,
    /// The action set the game itself last asked for, to go back to.
    game_set: ActionSetId,
    /// Whether the menu was open in the last frame.
    was_open: bool,
}

impl Menu {
    fn new(actions: &ActionMap, settings: Settings, file: Option<PathBuf>) -> Option<Self> {
        let actions = Actions::find(actions)?;
        let mut ui = Ui::default();
        ui.set_text_measure(crate::error_overlay::measure());
        ui.set_ui_scale(settings.ui_scale());
        let widgets = Widgets::build(&mut ui, &settings)?;
        Some(Self {
            actions,
            keys: Keys::default(),
            ui,
            widgets,
            settings,
            saved: settings,
            file,
            unapplied: false,
            shown_scale: settings.ui_scale(),
            open_asked: false,
            close_asked: false,
            game_set: ActionMap::GAMEPLAY,
            was_open: false,
        })
    }

    /// Notes what the player did this tick.
    fn hear(&mut self, input: &InputFrame) {
        self.open_asked |= input.just_pressed(self.actions.pause);
        self.close_asked |= input.just_pressed(self.actions.close);
        self.keys.hear(input, &self.actions);
    }

    /// Works the menu for one frame and draws it over the game.
    fn show(&mut self, frame: &mut Frame) {
        if std::mem::take(&mut self.open_asked) && !self.ui.is_menu_open() {
            // The screen is this UI's own, so opening it cannot fail.
            let _ = self.ui.open(self.widgets.top);
            // A rumble that was playing would shake a hand that is now
            // reading a menu.
            frame.rumble(Rumble::STOP);
        }
        if std::mem::take(&mut self.close_asked) {
            self.ui.close_all();
        }

        let mut input = self.keys.take();
        input.pointer = frame.pointer();
        input.pointer_down = frame.pointer_down();

        let screen = frame.screen_size();
        let mut events = [None; MOST_EVENTS];
        let made = self.ui.update(&input, frame.delta_seconds(), screen);
        for (kept, event) in events.iter_mut().zip(made) {
            *kept = Some(*event);
        }
        let mut changed = false;
        for event in events.into_iter().flatten() {
            changed |= self.answer(event, frame);
        }
        // The size follows its slider at once from a gamepad or keyboard,
        // but a slider that grew under the mouse dragging it would run away.
        let scale = self.settings.ui_scale();
        if !input.pointer_down && self.shown_scale != scale {
            self.shown_scale = scale;
            self.ui.set_ui_scale(scale);
            changed = true;
        }
        if changed {
            // What was just done is shown in this frame, not the next. The
            // same input again presses nothing new.
            self.ui.update(&input, 0.0, screen);
        }

        if std::mem::take(&mut self.unapplied) {
            frame.apply_settings(self.settings);
        }
        if self.ui.open_screen() != Some(self.widgets.settings) {
            self.save();
        }

        let open = self.ui.is_menu_open();
        if let Some(asked) = frame.action_set() {
            self.game_set = asked;
        }
        if open {
            frame.use_action_set(self.actions.set);
        } else if self.was_open {
            frame.use_action_set(self.game_set);
        }
        self.was_open = open;
        frame.draw_ui(&self.ui);
    }

    /// Does what one press or change in the menu asks for. Returns whether
    /// the menu now looks different from how it was drawn.
    fn answer(&mut self, event: UiEvent, frame: &mut Frame) -> bool {
        let widgets = self.widgets;
        match event {
            UiEvent::Pressed(button) => {
                if button == widgets.resume {
                    self.ui.close_all();
                } else if button == widgets.open_settings {
                    let _ = self.ui.open(widgets.settings);
                } else if button == widgets.quit {
                    let _ = self.ui.open(widgets.leaving);
                } else if button == widgets.done || button == widgets.stay {
                    self.ui.close();
                } else if button == widgets.leave {
                    frame.close();
                } else if button == widgets.reset {
                    self.settings = Settings::default();
                    self.unapplied = true;
                    // The widgets are this UI's own and of these kinds, so
                    // setting them cannot fail.
                    let _ = self.ui.set_slider(widgets.volume, self.settings.volume);
                    let _ = (self.ui).set_slider(widgets.sensitivity, self.settings.sensitivity);
                    let _ = (self.ui).set_toggle(widgets.invert_look, self.settings.invert_look);
                    let _ = self.ui.set_slider(widgets.ui_size, self.settings.ui_size);
                } else {
                    return false;
                }
                true
            }
            UiEvent::Changed(widget, value) => {
                match value {
                    Value::Slider(number) if widget == widgets.volume => {
                        self.settings.volume = number;
                    }
                    Value::Slider(number) if widget == widgets.sensitivity => {
                        self.settings.sensitivity = number;
                    }
                    Value::Slider(number) if widget == widgets.ui_size => {
                        self.settings.ui_size = number;
                    }
                    Value::Toggle(on) if widget == widgets.invert_look => {
                        self.settings.invert_look = on;
                    }
                    _ => return false,
                }
                self.unapplied = true;
                // The widget already shows its new value.
                false
            }
            UiEvent::Back(_) => false,
        }
    }

    /// Writes the settings to the player's file if they differ from it.
    ///
    /// Called once the settings screen is left, not on every step of a
    /// slider, and when the game ends.
    fn save(&mut self) {
        if self.settings == self.saved {
            return;
        }
        // Tried once for each change, whether or not it works: a file that
        // cannot be written is not tried again every frame.
        self.saved = self.settings;
        let Some(file) = &self.file else {
            return;
        };
        if let Err(error) = settings::save(file, &self.settings) {
            log::warn!(
                "Your settings could not be saved to {}: {error}. They last until the game \
                 is closed. Check that you may write there, or set {} to the path of \
                 another file.",
                file.display(),
                settings::FILE_VARIABLE
            );
        }
    }
}

impl Drop for Menu {
    /// A window closed with the settings screen still open keeps what was
    /// changed there.
    fn drop(&mut self) {
        self.save();
    }
}

#[cfg(test)]
mod tests;
