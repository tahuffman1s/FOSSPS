//! The input demo: live input shown on screen and in the window title, to
//! try a keyboard, mouse or gamepad by hand.
//!
//! The actions that are active are written in the top-left corner over a
//! crosshair, which is also the first HUD the engine draws in a window.
//!
//! `pause` (Escape or Start) opens the engine's own pause menu, as it does
//! in every game: the cursor comes back and no gameplay action hears
//! anything until the menu is closed.
//!
//! `fire` shakes the gamepad like a shot and `melee` like a hit taken, to
//! feel rumble by hand.

use std::fmt::Write as _;

use fossps::input::ActionKind;
use fossps::render::Font;
use fossps::ui::{Anchor, Color, Ui, UiInput, WidgetId};
use fossps::{ActionId, ActionMap, Frame, Game, InputFrame, Rumble};

/// Shows which actions are active, on screen and in the window title, and
/// logs every press.
pub struct InputDemo {
    map: ActionMap,
    look: Option<ActionId>,
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
        Self { ui, status }
    }

    /// Shows `status` in the corner.
    fn show(&mut self, status: &str) {
        // The widget is this UI's own text, so setting it cannot fail.
        let _ = self.ui.set_text(self.status, status);
    }
}

impl InputDemo {
    /// A demo of `map`'s actions.
    pub fn new(map: ActionMap) -> Self {
        Self {
            look: map.action("look").ok(),
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

        // A shot is short and mostly buzz; a hit is longer and mostly thump.
        let pressed = |action: Option<ActionId>| action.is_some_and(|id| input.just_pressed(id));
        if pressed(self.fire) {
            self.rumble = Some(Rumble::new(0.2, 0.6, 0.1));
        }
        if pressed(self.melee) {
            self.rumble = Some(Rumble::new(1.0, 0.3, 0.4));
        }

        self.title.clear();
        // Writing to a String cannot fail.
        let _ = write!(self.title, "aim {:.0}, {:.0}", self.aim[0], self.aim[1]);
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
        if self.title != self.shown {
            self.shown.clone_from(&self.title);
            frame.set_title(format!("FOSSPS input demo | {}", self.title));
            self.hud.show(&self.title);
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
    fn fire_and_melee_ask_for_rumble_once_per_press() {
        let mut demo = InputDemo::new(ActionMap::default_fps());
        let mut input = Input::new(ActionMap::default_fps());
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
    fn the_hud_says_what_is_pressed() {
        use fossps::ui::Draw;

        let mut demo = InputDemo::new(ActionMap::default_fps());
        let mut input = Input::new(ActionMap::default_fps());
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
        let idle = texts(&mut demo, &mut input);
        assert!(!idle[0].contains("jump"), "{idle:?}");
    }
}
