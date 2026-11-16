//! Playing the level from the editor: one key, or one gamepad button,
//! starts the game where the camera is, and the same one comes back.
//!
//! What is played is the game as `fossps run` plays it (its script, real
//! netcode over this computer's own network, the level's shapes solid), on
//! a copy of the map the editor holds, saved or not. Nothing done while
//! playing reaches the editor: its camera, what is chosen and the map are
//! as they were when playing stops.
//!
//! The player's eyes start where the camera was, looking the same way, and
//! the player drops to the floor from there. A camera with nothing under
//! it to land on (off the edge of the level, say) starts the player at a
//! spawn point, as the game itself would, and that is said.

use std::path::Path;

use fossps_input::ActionMap;

use crate::game::{Frame, Game};
use crate::map::Map;
use crate::online::{LocalGame, World};
use crate::script_game::ScriptGame;

/// Said on the screen for as long as the level is played.
const PLAYING: &str = "Playing    F5 or Select: back to editing";
/// Said under it when the player could not start where the camera was.
const NOT_FROM_HERE: &str = "There is nothing under the camera to stand on, so you started \
                             at a spawn point.";

/// The level being played from the editor.
#[derive(Debug)]
pub(super) struct Play {
    game: LocalGame,
    /// Whether the player started where the camera was.
    from_here: bool,
}

impl Play {
    /// Starts the game in `folder` (or, with no folder, a game with no
    /// script) in `world`, with `map` placed in it, and puts the player's
    /// eyes at `eye` looking the way `facing` says (yaw, then pitch).
    ///
    /// # Errors
    ///
    /// Says, for the person at the editor, why the game could not start.
    pub(super) fn start(
        folder: Option<&Path>,
        map: &Map,
        world: &World,
        actions: &ActionMap,
        eye: [f32; 3],
        facing: [f32; 2],
    ) -> Result<Self, String> {
        let game = match folder {
            Some(folder) => ScriptGame::open(folder, actions)
                .map_err(|error| format!("Could not play: {error}"))?,
            None => ScriptGame::from_source("", actions),
        };
        // The map as the editor has it, whatever is in the file by now.
        let game = game.with_map(map.clone());
        let (game, from_here) =
            LocalGame::start_from(game, world, actions, eye, facing).map_err(|error| {
                format!(
                    "Could not play: {error} The game is played over this computer's own \
                     network (127.0.0.1), which a firewall or security program may be blocking."
                )
            })?;
        Ok(Self { game, from_here })
    }

    /// Plays one tick. Says why if the game has ended by itself, which is
    /// not expected of a game on this computer.
    pub(super) fn tick(&mut self, input: &fossps_input::InputFrame, step: f32) -> Option<String> {
        self.game.tick(input, step);
        self.game
            .joined()
            .closed()
            .map(|error| format!("Playing stopped. {error}"))
    }

    /// Presents the game's frame, with a line over it that says how to get
    /// back to editing.
    pub(super) fn frame(&mut self, frame: &mut Frame) {
        self.game.frame(frame);
        let from_here = self.from_here;
        frame.panels(|ui| {
            // A name being typed when playing began must not go on taking
            // the keys that now walk.
            ui.ctx().memory_mut(|memory| {
                if let Some(typing) = memory.focused() {
                    memory.surrender_focus(typing);
                }
            });
            banner(ui, from_here);
        });
    }

    /// Where the player's eyes are.
    #[cfg(test)]
    pub(super) fn eye(&self) -> [f32; 3] {
        self.game.joined().eye_position()
    }

    /// Which way the player looks: yaw, then pitch.
    #[cfg(test)]
    pub(super) fn facing(&self) -> [f32; 2] {
        [self.game.joined().yaw(), self.game.joined().pitch()]
    }

    /// Whether the player started where the camera was.
    #[cfg(test)]
    pub(super) fn started_here(&self) -> bool {
        self.from_here
    }

    /// The map the game is played with.
    #[cfg(test)]
    pub(super) fn map(&self) -> &Map {
        self.game.hosted().game().map()
    }
}

/// Says along the top of the picture that the level is being played and
/// how to get back. Only painted: it takes no clicks from the game.
fn banner(ui: &egui::Ui, from_here: bool) {
    let window = ui.ctx().content_rect();
    let painter = ui.painter();
    let dark = egui::Color32::from_black_alpha(190);
    let mut top = window.top() + 10.0;
    let lines = [
        (PLAYING, 16.0, egui::Color32::WHITE),
        (NOT_FROM_HERE, 14.0, egui::Color32::from_rgb(255, 214, 110)),
    ];
    for (words, size, ink) in lines.into_iter().take(if from_here { 1 } else { 2 }) {
        let galley =
            painter.layout_no_wrap(words.to_owned(), egui::FontId::proportional(size), ink);
        let at = egui::pos2(window.center().x - galley.size().x / 2.0, top + 5.0);
        let back = egui::Rect::from_min_size(at, galley.size()).expand2(egui::vec2(12.0, 5.0));
        painter.rect_filled(back, 6.0, dark);
        painter.galley(at, galley, ink);
        top = back.bottom() + 4.0;
    }
}
