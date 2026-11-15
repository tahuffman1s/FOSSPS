//! The input demo: live input shown on screen and in the window title, to
//! try a keyboard, mouse or gamepad by hand.
//!
//! The actions that are active are written in the top-left corner over a
//! crosshair, which is also the first HUD the engine draws in a window.
//!
//! `pause` (Escape or Start) switches to a second action set, as a pause
//! menu would: the cursor comes back and no gameplay action hears anything
//! until `resume` (the same two buttons) switches back.
//!
//! `fire` shakes the gamepad like a shot and `melee` like a hit taken, to
//! feel rumble by hand.

use std::fmt::Write as _;

use fossps::input::ActionKind;
use fossps::render::Font;
use fossps::ui::{Anchor, Color, Ui, UiInput, WidgetId};
use fossps::{ActionId, ActionMap, ActionSetId, Frame, Game, InputFrame, MouseMode, Rumble};

/// Shows which actions are active, on screen and in the window title, and
/// logs every press.
pub struct InputDemo {
    map: ActionMap,
    look: Option<ActionId>,
    pause: Option<ActionId>,
    /// The set that is live while paused, and the action that leaves it.
    menu: Option<(ActionSetId, ActionId)>,
    paused: bool,
    /// The buttons that shake the gamepad: one like a shot, one like a hit.
    fire: Option<ActionId>,
    melee: Option<ActionId>,
    /// The rumble to start on the next frame.
    rumble: Option<Rumble>,
    /// Total look movement, to show that mouse and stick both drive it.
    aim: [f32; 2],
    title: String,
    shown: String,
    hud: Hud,
}

/// What the demo draws over the picture.
struct Hud {
    ui: Ui,
    /// The line that says which actions are active.
    status: WidgetId,
    /// Two lines in the middle of the screen while paused.
    paused: [WidgetId; 2],
}

impl Hud {
    fn new() -> Self {
        let mut ui = Ui::default();
        // Text is measured in the font it is drawn in.
        ui.set_text_measure(Box::new(Font::bundled()));
        let status = ui.hud_text(Anchor::TopLeft, [0.0, 0.0], 28.0, "");
        for size in [[4.0, 24.0], [24.0, 4.0]] {
            ui.hud_rect(Anchor::Center, [0.0, 0.0], size, Color::WHITE);
        }
        let paused = [
            ui.hud_text(Anchor::Center, [0.0, -130.0], 44.0, ""),
            ui.hud_text(Anchor::Center, [0.0, -70.0], 28.0, ""),
        ];
        Self { ui, status, paused }
    }

    /// Shows `status` in the corner, and the pause lines if `paused`.
    fn show(&mut self, status: &str, paused: bool) {
        let lines = if paused {
            ["Paused", "Escape or Start resumes"]
        } else {
            ["", ""]
        };
        // The widgets are this UI's own text, so setting them cannot fail.
        let _ = self.ui.set_text(self.status, status);
        for (widget, line) in self.paused.into_iter().zip(lines) {
            let _ = self.ui.set_text(widget, line);
        }
    }
}

impl InputDemo {
    /// A demo of `map`'s actions, with a `menu` set added to it. Run it with
    /// [`actions`](Self::actions), so the window knows the set too.
    pub fn new(mut map: ActionMap) -> Self {
        // A map that already has a "menu" set or a "resume" action keeps
        // them, and the demo then simply never pauses.
        let menu = map
            .add_set("menu", MouseMode::Pointer)
            .ok()
            .and_then(|set| {
                let resume = map.button("resume", ["Escape", "GamepadStart"]).ok()?;
                Some((set, resume))
            });
        Self {
            look: map.action("look").ok(),
            pause: map.action("pause").ok(),
            menu,
            paused: false,
            fire: map.action("fire").ok(),
            melee: map.action("melee").ok(),
            rumble: None,
            map,
            aim: [0.0; 2],
            title: String::new(),
            shown: String::new(),
            hud: Hud::new(),
        }
    }
}

impl InputDemo {
    /// The actions the demo reads: the ones it was given and its `menu` set.
    pub fn actions(&self) -> &ActionMap {
        &self.map
    }
}

impl Game for InputDemo {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        const MOUSE_DEGREES_PER_COUNT: f32 = 0.05;
        const STICK_DEGREES_PER_SECOND: f32 = 180.0;

        if let Some(look) = self.look {
            let mouse = input.delta(look);
            let stick = input.axis2d(look);
            for axis in 0..2 {
                self.aim[axis] += mouse[axis] * MOUSE_DEGREES_PER_COUNT
                    + stick[axis] * STICK_DEGREES_PER_SECOND * step;
            }
        }

        if let (Some(pause), Some((_, resume))) = (self.pause, self.menu) {
            if !self.paused && input.just_pressed(pause) {
                self.paused = true;
                log::info!(
                    "tick {}: paused. The \"menu\" action set is live.",
                    input.tick
                );
            } else if self.paused && input.just_pressed(resume) {
                self.paused = false;
                log::info!(
                    "tick {}: resumed. The \"gameplay\" action set is live.",
                    input.tick
                );
            }
        }

        // A shot is short and mostly buzz; a hit is longer and mostly thump.
        let pressed = |action: Option<ActionId>| action.is_some_and(|id| input.just_pressed(id));
        if pressed(self.fire) {
            self.rumble = Some(Rumble::new(0.2, 0.6, 0.1));
        }
        if pressed(self.melee) {
            self.rumble = Some(Rumble::new(1.0, 0.3, 0.4));
        }
        // Pausing stops a rumble that is playing.
        if self.paused && self.rumble.is_none() && pressed(self.pause) {
            self.rumble = Some(Rumble::STOP);
        }

        self.title.clear();
        // Writing to a String cannot fail.
        let _ = write!(self.title, "aim {:.0}, {:.0}", self.aim[0], self.aim[1]);
        if self.paused {
            self.title.push_str(" | PAUSED: Escape or Start resumes");
        }
        for action in self.map.actions() {
            let name = self.map.name(action);
            if input.just_pressed(action) && self.map.kind(action) == Some(ActionKind::Button) {
                log::info!("tick {}: {name} pressed", input.tick);
            }
            if input.pressed(action) && Some(action) != self.look {
                let [x, y] = input.axis2d(action);
                let _ = match self.map.kind(action) {
                    Some(ActionKind::Axis2D) => write!(self.title, " | {name} {x:.2}, {y:.2}"),
                    Some(ActionKind::Axis1D) => write!(self.title, " | {name} {x:.2}"),
                    _ => write!(self.title, " | {name}"),
                };
            }
        }
    }

    fn frame(&mut self, frame: &mut Frame) {
        if let Some(rumble) = self.rumble.take() {
            frame.rumble(rumble);
        }
        if let Some((menu, _)) = self.menu {
            frame.use_action_set(if self.paused {
                menu
            } else {
                ActionMap::GAMEPLAY
            });
        }
        if self.title != self.shown {
            self.shown.clone_from(&self.title);
            frame.set_title(format!("FOSSPS input demo | {}", self.title));
            self.hud.show(&self.title, self.paused);
        }
        // Nothing in the demo's HUD is pointed at or pressed, and nothing
        // in it moves by itself, so it needs no input and no time.
        self.hud
            .ui
            .update(&UiInput::default(), 0.0, frame.screen_size());
        frame.draw_ui(&self.hud.ui);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests")]

    use fossps::input::{Input, Key};

    use super::*;

    #[test]
    fn pause_and_resume_share_a_key_without_bouncing() {
        let mut demo = InputDemo::new(ActionMap::default_fps());
        let (menu, _) = demo.menu.unwrap();
        let mut input = Input::new(demo.actions().clone());
        // What the window does with `Frame::use_action_set` after each frame.
        let step = |demo: &mut InputDemo, input: &mut Input| {
            demo.tick(input.tick(), 1.0 / 60.0);
            input.set_active_set(if demo.paused {
                menu
            } else {
                ActionMap::GAMEPLAY
            });
        };

        input.press(Key::Escape);
        step(&mut demo, &mut input);
        assert!(demo.paused);
        // Escape is still down: it must not resume.
        step(&mut demo, &mut input);
        step(&mut demo, &mut input);
        assert!(demo.paused);
        assert_eq!(input.active_set(), menu);

        input.release(Key::Escape);
        step(&mut demo, &mut input);
        input.press(Key::Escape);
        step(&mut demo, &mut input);
        assert!(!demo.paused);
        step(&mut demo, &mut input);
        assert!(!demo.paused, "the same press must not pause again");
        assert_eq!(input.active_set(), ActionMap::GAMEPLAY);
    }

    #[test]
    fn fire_and_melee_ask_for_rumble_once_per_press() {
        let mut demo = InputDemo::new(ActionMap::default_fps());
        let mut input = Input::new(demo.actions().clone());
        // What the window does each frame: tick, then let the demo present.
        let asked = |demo: &mut InputDemo, input: &mut Input| {
            demo.tick(input.tick(), 1.0 / 60.0);
            let asked = demo.rumble;
            demo.frame(&mut Frame::default());
            assert!(demo.rumble.is_none(), "a frame takes the rumble");
            asked
        };

        assert_eq!(asked(&mut demo, &mut input), None);
        input.press(fossps::input::GamepadButton::RightTrigger);
        assert_eq!(
            asked(&mut demo, &mut input),
            Some(Rumble::new(0.2, 0.6, 0.1))
        );
        // Still held: the shot is not felt again.
        assert_eq!(asked(&mut demo, &mut input), None);
    }

    #[test]
    fn the_hud_says_what_is_pressed_and_when_the_demo_is_paused() {
        use fossps::ui::Draw;

        let mut demo = InputDemo::new(ActionMap::default_fps());
        let mut input = Input::new(demo.actions().clone());
        let texts = |demo: &mut InputDemo, input: &mut Input| -> Vec<String> {
            demo.tick(input.tick(), 1.0 / 60.0);
            demo.frame(&mut Frame::default());
            demo.hud
                .ui
                .draw()
                .iter()
                .filter_map(|draw| match draw {
                    Draw::Text { text, .. } => Some(text.to_owned()),
                    Draw::Rect { .. } => None,
                })
                .collect()
        };

        input.press(Key::Space);
        let playing = texts(&mut demo, &mut input);
        assert_eq!(playing.len(), 1, "{playing:?}");
        assert!(playing[0].contains("jump"), "{playing:?}");
        // The crosshair is two boxes.
        assert_eq!(demo.hud.ui.draw().len(), 3);

        input.release(Key::Space);
        input.press(Key::Escape);
        let paused = texts(&mut demo, &mut input);
        assert!(paused.contains(&"Paused".to_owned()), "{paused:?}");
        assert!(paused.contains(&"Escape or Start resumes".to_owned()));
    }

    #[test]
    fn a_map_that_has_a_menu_set_already_is_left_alone() {
        let mut map = ActionMap::default_fps();
        map.add_set("menu", MouseMode::Pointer).unwrap();
        let demo = InputDemo::new(map.clone());
        assert!(demo.menu.is_none());
        assert_eq!(demo.actions(), &map);
    }
}
