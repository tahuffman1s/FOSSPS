//! A game whose rules are a Luau script.

use std::path::Path;

use fossps_input::{ActionMap, InputFrame};
use fossps_script::{OpenError, ScriptHost};

use crate::error_overlay::ErrorOverlay;
use crate::game::{Frame, Game};
use crate::map::{Map, MapError, MapFile};

/// A [`Game`] that runs the `main.luau` in a game folder, with the scripts
/// it requires, and loads them again whenever one of them is saved.
///
/// A mistake in a script is logged, and a game opened from a folder also
/// shows it on screen, over the game, until a save loads the script again.
/// A game run from text has no file to fix, so it shows nothing: its
/// players did not write it.
///
/// A game opened from a folder also has what its `map.toml` places in the
/// level ([`map`](Self::map)), read again whenever that file is saved.
///
/// ```no_run
/// use fossps::{Config, ScriptGame};
///
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let config = Config::default();
///     let game = ScriptGame::open("my-game", &config.actions)?;
///     fossps::run(config, game)?;
///     Ok(())
/// }
/// ```
#[derive(Debug)]
pub struct ScriptGame {
    host: ScriptHost,
    /// Shows the latest script error over the game. Only a game whose
    /// scripts are files that can be fixed and saved has one.
    errors: Option<ErrorOverlay>,
    /// What is placed in the level.
    map: Map,
    /// The map's file, looked at for saves. Only a game opened from a
    /// folder has one.
    map_file: Option<MapFile>,
    /// Why the map file as it now is could not be read, if it could not.
    map_error: Option<MapError>,
}

/// Said after a mistake in a map file, by what the game does about it.
const NOTHING_PLACED: &str = "The game runs with nothing placed in its level. Fix this and \
                              save the file: the game picks it up by itself.";
const KEEPS_MAP: &str = "The game keeps the map it had. Fix this and save the file: the \
                         game picks it up by itself.";

impl ScriptGame {
    /// Opens the game in `folder`. The script reads input through `actions`,
    /// which should be the same actions the engine is started with.
    ///
    /// A mistake inside the script does not fail here: it is logged and
    /// shown on screen, and the game waits for the file to be fixed and
    /// saved.
    ///
    /// The same goes for a mistake in the game's `map.toml`: it is logged,
    /// nothing is placed, and the game waits for the file to be fixed and
    /// saved ([`map_error`](Self::map_error) says what is wrong meanwhile).
    ///
    /// # Errors
    ///
    /// Fails if `folder` has no `main.luau` or the file cannot be read.
    pub fn open(folder: impl AsRef<Path>, actions: &ActionMap) -> Result<Self, OpenError> {
        let folder = folder.as_ref();
        let host = ScriptHost::open(folder, actions.clone())?;
        let (map_file, read) = MapFile::open(folder);
        let (map, map_error) = match read {
            Ok(map) => (map, None),
            Err(error) => {
                log::error!("{error}\n{NOTHING_PLACED}");
                (Map::default(), Some(error))
            }
        };
        Ok(Self {
            host,
            errors: Some(ErrorOverlay::default()),
            map,
            map_file: Some(map_file),
            map_error,
        })
    }

    /// Runs `source` as the game's `main.luau` with no file behind it and no
    /// other scripts to require. Nothing is watched or loaded again, and a
    /// mistake inside the script is logged and kept in the host's
    /// `last_error`.
    #[must_use]
    pub fn from_source(source: impl Into<String>, actions: &ActionMap) -> Self {
        Self {
            host: ScriptHost::from_source(source, actions.clone()),
            errors: None,
            map: Map::default(),
            map_file: None,
            map_error: None,
        }
    }

    /// Runs a game whose scripts are given as text, as a packaged game does.
    /// `scripts` are pairs of a name (`main.luau`, `weapons/rifle.luau`) and
    /// the script's text; `main.luau` is run and can require the others.
    /// Nothing is watched or loaded again, and a mistake inside a script is
    /// logged and kept in the host's `last_error`.
    #[must_use]
    pub fn from_sources(
        scripts: impl IntoIterator<Item = (String, String)>,
        actions: &ActionMap,
    ) -> Self {
        Self {
            host: ScriptHost::from_sources(scripts, actions.clone()),
            errors: None,
            map: Map::default(),
            map_file: None,
            map_error: None,
        }
    }

    /// Places `map` in the game's level, as a packaged game does with the
    /// map it was packed with. It takes the place of any map the game had.
    #[must_use]
    pub fn with_map(mut self, map: Map) -> Self {
        self.map = map;
        self.map_error = None;
        self
    }

    /// What is placed in the game's level: its spawn points, zones and
    /// objectives. For a game opened from a folder this is its `map.toml`
    /// as last saved without a mistake.
    #[must_use]
    pub fn map(&self) -> &Map {
        &self.map
    }

    /// What is wrong with the game's `map.toml` as it now is on disk, if it
    /// could not be read. The game keeps the map it had.
    #[must_use]
    pub fn map_error(&self) -> Option<&MapError> {
        self.map_error.as_ref()
    }

    /// Reads the map file again if it was saved since it was last read.
    fn poll_map(&mut self) {
        let Some(change) = self.map_file.as_mut().and_then(MapFile::changed) else {
            return;
        };
        match change {
            Ok(map) => {
                log::info!("Loaded {} again: {}.", crate::map::MAP_FILE, map.summary());
                self.map = map;
                self.map_error = None;
            }
            Err(error) => {
                log::error!("{error}\n{KEEPS_MAP}");
                self.map_error = Some(error);
            }
        }
    }

    /// The script host, for example to read its latest error.
    #[must_use]
    pub fn host(&self) -> &ScriptHost {
        &self.host
    }

    /// The script host, to tell it what players do: who joined, who left and
    /// who killed whom.
    pub fn host_mut(&mut self) -> &mut ScriptHost {
        &mut self.host
    }
}

impl Game for ScriptGame {
    fn tick(&mut self, input: &InputFrame, step: f32) {
        self.host.poll_reload();
        self.poll_map();
        self.host.tick(input, step);
    }

    fn frame(&mut self, frame: &mut Frame) {
        if let Some(rumble) = self.host.take_rumble() {
            frame.rumble(rumble);
        }
        // Last, so it is on top of anything the game draws.
        if let Some(errors) = &mut self.errors {
            errors.show(self.host.last_error(), frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use fossps_ui::Draw;

    use super::*;

    /// The lines of text `game` draws over one frame.
    fn words(game: &mut ScriptGame) -> Vec<String> {
        let mut frame = Frame::new(0.0);
        game.frame(&mut frame);
        frame
            .take_ui()
            .draw()
            .iter()
            .filter_map(|draw| match draw {
                Draw::Text { text, .. } => Some(text.to_owned()),
                Draw::Rect { .. } => None,
            })
            .collect()
    }

    #[test]
    fn a_script_error_is_on_screen_until_a_save_loads_the_script_again() {
        let folder =
            std::env::temp_dir().join(format!("fossps-script-game-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let script = folder.join("main.luau");
        fs::write(&script, "local ticks = 0\ngame.onTik(function() end)\n").unwrap();

        let actions = ActionMap::default();
        let mut game = ScriptGame::open(&folder, &actions).unwrap();
        let shown = words(&mut game);
        assert_eq!(shown[0], "Script error in main.luau, line 2", "{shown:?}");
        assert_eq!(shown[1], "2 | game.onTik(function() end)");
        assert!(shown.iter().any(|line| line.contains("game.onTick")));
        // It stays while the file is as it was.
        game.tick(&InputFrame::default(), 1.0 / 60.0);
        assert_eq!(words(&mut game), shown);

        // The file is fixed and saved, and the game picks it up.
        fs::write(&script, "game.onTick(function() end)\n").unwrap();
        let give_up = Instant::now() + Duration::from_secs(20);
        while game.host().last_error().is_some() && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(20));
            game.tick(&InputFrame::default(), 1.0 / 60.0);
        }
        assert_eq!(words(&mut game), Vec::<String>::new());

        // A mistake that only shows while the game runs is shown too.
        fs::write(
            &script,
            "game.onTick(function() local a = nil; a.b = 1 end)\n",
        )
        .unwrap();
        let give_up = Instant::now() + Duration::from_secs(20);
        while game.host().last_error().is_none() && Instant::now() < give_up {
            std::thread::sleep(Duration::from_millis(20));
            game.tick(&InputFrame::default(), 1.0 / 60.0);
        }
        let shown = words(&mut game);
        assert_eq!(shown[0], "Script error in main.luau, line 1", "{shown:?}");
        let _ = fs::remove_dir_all(&folder);
    }

    /// The script error is drawn for real, with no window, and the picture
    /// is read back: a dark panel with a red line along its top and light
    /// letters in it, in the top-left corner, and the game untouched
    /// everywhere else.
    #[cfg(feature = "window")]
    #[test]
    fn the_error_is_drawn_as_a_dark_panel_with_light_text_in_the_corner() {
        use fossps_render::{Camera, Headless, RenderError, Upscaling};

        const SCREEN: [u32; 2] = [1280, 720];
        let mut headless = match Headless::new(SCREEN[0], SCREEN[1], Upscaling::Off) {
            Ok(headless) => headless,
            Err(
                error @ (RenderError::NoVulkan(_)
                | RenderError::OldVulkan { .. }
                | RenderError::NoSuitableGpu { .. }),
            ) => {
                let required =
                    std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
                assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {error}");
                eprintln!("Skipping the drawing test: {error}");
                return;
            }
            Err(error) => panic!("{error}"),
        };
        let folder =
            std::env::temp_dir().join(format!("fossps-script-game-drawn-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("main.luau"), "game.onTik(function() end)\n").unwrap();
        let mut game = ScriptGame::open(&folder, &ActionMap::default()).unwrap();
        let _ = fs::remove_dir_all(&folder);

        let camera = Camera::default();
        let bare = headless.screenshot(&camera).unwrap();
        let mut frame = Frame::new(0.0).for_window(Some(SCREEN), fossps_ui::DrawBuffer::default());
        game.frame(&mut frame);
        let ui = frame.take_ui();
        let Some(Draw::Rect { position, size, .. }) = ui.draw().iter().next() else {
            panic!("the panel is drawn first");
        };
        headless.set_ui(ui.draw());
        let shown = headless.screenshot(&camera).unwrap();
        assert_eq!(fossps_render::validation_errors(), 0);

        let pixel = |picture: &[u8], x: u32, y: u32| {
            let start = ((y * SCREEN[0] + x) * 4) as usize;
            [picture[start], picture[start + 1], picture[start + 2]]
        };
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let [left, top, right, bottom] = [
            position[0],
            position[1],
            position[0] + size[0],
            position[1] + size[1],
        ]
        .map(|edge| edge.round() as u32);
        assert!(
            left >= 60 && top >= 30,
            "inside the safe area: {position:?}"
        );
        assert!(right < SCREEN[0] && bottom < SCREEN[1] / 2, "{size:?}");

        // The red line along the top, in the colour it was asked for.
        let [red, green, blue] = pixel(&shown, (left + right) / 2, top + 1);
        assert!(red > 240 && green.abs_diff(84) < 12 && blue.abs_diff(84) < 12);

        // Inside: mostly the dark panel, with a share of light letters.
        let (mut dark, mut light, mut all) = (0_u32, 0_u32, 0_u32);
        for y in top + 6..bottom - 2 {
            for x in left + 2..right - 2 {
                let [red, green, blue] = pixel(&shown, x, y);
                all += 1;
                dark += u32::from(red < 60 && green < 60 && blue < 60);
                light += u32::from(red > 170 && green > 130);
            }
        }
        assert!(dark * 10 > all * 7, "{dark} dark pixels of {all}");
        assert!(light * 200 > all, "{light} light pixels of {all}");
        assert!(light * 4 < all, "{light} light pixels of {all}");

        // Outside the panel the picture is as it was.
        for (x, y) in [
            (left - 3, top + 10),
            (right + 3, top + 10),
            (left + 10, bottom + 3),
            (SCREEN[0] - 1, SCREEN[1] - 1),
            (SCREEN[0] / 2, SCREEN[1] - 40),
        ] {
            assert_eq!(pixel(&shown, x, y), pixel(&bare, x, y), "at {x}, {y}");
        }
    }

    #[test]
    fn a_game_with_no_file_to_fix_shows_no_error() {
        let actions = ActionMap::default();
        let mut game = ScriptGame::from_source("game.onTik(function() end)", &actions);
        assert!(game.host().last_error().is_some());
        assert_eq!(words(&mut game), Vec::<String>::new());
    }
}
