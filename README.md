# FOSSPS — FOSS Person Shooter

A free, open-source engine for multiplayer first-person shooters.

[Website](https://tahuffman1s.github.io/FOSSPS/) · [Docs](https://tahuffman1s.github.io/FOSSPS/docs/) · [Tutorial](https://tahuffman1s.github.io/FOSSPS/docs/first-match.html) · [Roadmap](ROADMAP.md) · [Issues](https://github.com/tahuffman1s/FOSSPS/issues)

[![CI](https://github.com/tahuffman1s/FOSSPS/actions/workflows/ci.yml/badge.svg)](https://github.com/tahuffman1s/FOSSPS/actions/workflows/ci.yml)

**Status:** just started. Nothing is playable yet. So far: a Vulkan 1.3 window that draws meshes with a depth buffer from the player's eyes (a game's own level from the `level.glb` in its folder, in flat colours, with solid walls and floors that players walk on), a fixed-rate main loop, keyboard, mouse and gamepad input through one action map with controls the player can change in a text file, and Luau scripts that reload when you save them.

## Goals

- **Easy to use** — go from install to a playable networked match in minutes, with a Luau scripting API that reads like plain English.
- **Fast** — high, stable frame rates and low input latency; cheap dedicated servers.
- **Focused** — built only for multiplayer FPS games.
- **Modern** — Vulkan 1.3 renderer, FSR 3.1 upscaling, first-class controller support.

Planned platforms: Linux, Windows, and macOS as a stretch goal.

New here? [Your first match](https://tahuffman1s.github.io/FOSSPS/docs/first-match.html) is a ten-minute tutorial: install with one command, make a game, change it while it runs, turn it into a team deathmatch and join it from a second window. The pages are plain HTML in [`site/docs`](site/docs), and a test follows the tutorial's steps with the built `fossps`, so a change that makes the page wrong fails CI.

## Building

Requires stable [Rust](https://rustup.rs) and a C and C++ compiler (`gcc` and `g++`, `clang`, or the Visual Studio build tools); the Luau scripting language is built from source.
On Linux, gamepad support also needs the libudev headers and sound needs the ALSA headers:

```sh
sudo apt install g++ libudev-dev libasound2-dev          # Debian, Ubuntu
sudo dnf install gcc-c++ systemd-devel alsa-lib-devel    # Fedora
```

```sh
cargo build
cargo test
cargo run -- new my-game      # creates a game in the folder my-game
cargo run -- run my-game      # plays it (the first "run" is cargo's)
cargo run -- check my-game    # tests its script without opening a window, pressing every action
cargo run -- serve my-game    # runs it as a server, with no window (Ctrl-C stops it)
cargo run -- join 192.168.1.20  # joins the server at that address (none: this computer)
cargo run -- demo             # opens the input demo window
cargo run --release -- bench  # times a built-in test scene, with no window
```

`cargo run --` stands in for the `fossps` command when building from source; to get a real `fossps` command that way, run `cargo install --path crates/fossps`.

Every tagged version is also built for Linux, Windows and macOS and put on the [releases page](https://github.com/tahuffman1s/FOSSPS/releases) as one archive per system, holding `fossps`, `fossps-server` and a short README, with nothing to compile. No version has been tagged yet, so until the first one building from source is the only way in. How a release is made is in [docs/releasing.md](docs/releasing.md).

A machine that only hosts can use the separate `fossps-server` program, which does what `fossps serve` does and has no windowing, graphics, gamepad or sound code in it: `cargo build --release -p fossps-server`. It ticks the game 60 times a second, sleeps in between, logs a status line every minute and warns when it cannot keep up. Players on the same network join with just `fossps join`, which finds the server by itself (`fossps servers` lists what is there); from elsewhere they add the server's address, which it prints when it starts. It listens on UDP port 24680 unless `--port` says otherwise. `fossps run --open` plays a game and lets others on the network join it, with no separate server. A joined player walks with no wait for the server (`fossps-net`: commands, snapshots, prediction and correction), and `fossps run` plays through the same netcode on its own. The server and `fossps run` walk the game's level: its shapes are solid and players start at its spawn points. A player who joins needs no copy of the game: the server sends its level file and map before giving them a place, so they see and walk the same level, and the level is kept on their computer (`~/.cache/fossps/levels` on Linux, `%LOCALAPPDATA%\fossps\levels` on Windows, `~/Library/Caches/fossps/levels` on macOS, or the folder `FOSSPS_CACHE` names) so it is fetched once. A level of up to 256 MB is sent, at a megabyte or two a second; scripts are never sent, as they run on the server only. Other players are not drawn yet, and are not yet smoothed between snapshots.

Running it needs a graphics driver with Vulkan 1.3; building does not need the Vulkan SDK. If it says Vulkan is missing or too old, update your graphics driver (on Linux, also install your distribution's Vulkan loader package, for example `sudo dnf install vulkan-loader`). The input demo writes which actions are active in the top-left corner of the picture, over a crosshair, and in the window title. Click the window to capture the mouse, then try the keyboard, mouse or a gamepad. On a gamepad, firing and the melee button shake it, a shot lightly and a hit hard. Escape (Start on a gamepad) pauses: the cursor comes back and the game's actions hear nothing until you press it again, which is how a game switches between its gameplay and menu controls.

`fossps package`, run in a game's folder on Linux, writes `dist/<game>-linux-x86_64.tar.gz`: one folder holding the game's program, its files in one game pack (`<game>.fpk`), its icon if the folder has an `icon.png`, a `.desktop` file, and the licence of the font built into the program. A player unpacks it and runs the program; nothing is compiled, and the program is a copy of the `fossps` that made the package, which plays the pack beside it when it has one. `fossps run <game>.fpk` plays a pack too. So far the pack stores files as they are, everything in the folder goes in except hidden files and the ones `fossps new` made for the game's maker, and only Linux is packaged for: Windows, macOS, an AppImage, a dedicated server, smaller packs and packaging for another system than your own are still being built (#77 to #85, #121).

The engine draws a HUD and menus over the picture: boxes with round corners and text, at the window's full size after upscaling, so text stays sharp at any upscaling setting. A game written in Rust builds them with `fossps::ui` (every menu works with a gamepad, keyboard or mouse) and shows them with `frame.draw_ui(&ui)`. Text is drawn in one font that comes with the engine, [Atkinson Hyperlegible Next](crates/fossps-render/fonts/README.md), which covers the Latin alphabet; other alphabets (#221) are not drawn yet. Scripts cannot build a HUD yet (#126), games get no pause menu by themselves yet (#125), and so far only the input demo and the script error panel show any of it in a window.

`fossps bench` draws a built-in scene (1,600 blocks seen from a camera walking a circle) with no window and says how long a usual frame, a slow frame (99 of 100 are faster) and the slowest took, and how much of that was the engine's own work. Every run draws the same frames. `--size`, `--upscaling` and `--frames` change what is drawn, `--json bench.json` saves the numbers, and `--budget-ms 16.6` makes it fail when a slow frame took longer. The engine's own checks run it on every change with a budget, on a software renderer, so that budget only catches drawing getting much slower; it says nothing about how fast a real graphics card is.

A game's level is the `level.glb` in its folder: one glTF Binary file, which Blender exports (File > Export > glTF 2.0, format glTF Binary). `fossps run` and `fossps edit` load and draw it, a packaged game carries it, and `fossps new` puts a small arena there to start from. `fossps check` loads it too and says what to change in the export when it cannot be read; a game with a level that does not load is not started or packaged. A game with no `level.glb` is played on the engine's test floor, flat at height 0. The level is drawn in its materials' flat colours, without textures or lights yet (#59). Every shape in it is solid: players walk its floors, ramps and steps and are stopped by its walls, on the server and on the player's own side alike. Where players start is one rule: at the spawn points in `map.toml` (below) when it has any; otherwise at the level's own spawn markers, which are empty objects whose names begin with `spawn` (`spawn`, `spawn.001`, `Spawn Red`), turned the way the player should look, as the starter arena has at each end; otherwise at the level's middle. `fossps check` starts a player at each spawn point and warns about any with no floor under it. A player who joins someone else's game is sent that game's level and map by the server, and a saved level is picked up at the next start, not while the game runs (#229).

`fossps edit`, run in a game's folder, is the start of the editor: it opens the level to fly around, with a grid on the ground (a line every metre, a stronger one every ten, the X axis red and the Z axis blue). Hold the right mouse button to look around; W, A, S and D fly where you look, E and Q go up and down, Shift flies faster and the mouse wheel changes the speed. On a gamepad the sticks fly and look, the triggers go down and up, pressing the left stick flies faster and the d-pad changes the speed. A panel on the left lists everything in the level (its objects, markers such as spawn points, and lights) by the names they were given in Blender; click a name and a panel on the right says where it is, how big it is, which way it is turned, its colours and its custom properties, and a double click flies the camera to it. The panels are for the mouse. What is chosen in the level file is not yet marked in the picture (#231).

What the editor places goes in a text file beside the level, `map.toml`, and the level file itself is never written, so it keeps opening in Blender. "Place a spawn here" puts a spawn point on the ground where the camera looks; chosen in the list, its type (`spawn`, or any word of your own such as `zone` or `objective`), name, position and facing can be changed on the right, and it can be removed. Each placed thing is marked in the picture by a dot, a line the way it faces and its name, drawn over walls so it can be found. There is no save button: the file is written a moment after each change. It lists each entity with an ID that never changes, its type, position, facing and any properties, under a format version:

```toml
version = 1

[[entity]]
id = "k7f3q2xa"
type = "spawn"
name = "Red spawn"
position = [4.0, 0.0, -12.5]
facing = 180.0

[entity.properties]
team = "red"
```

IDs are made by chance and entities are written in ID order, so two people who each place something get changes that merge. The file can be written by hand as well: the open editor and a game that is running (`fossps run`, `fossps serve`) read it again when it is saved, and say which line to fix when it has a mistake, keeping the map they had. `fossps check` reads it too, and a packaged game carries it. So far the running game uses one thing from it: players come into the world at its `spawn` entities (away from other players, any team at any point), looking the way each one faces. As soon as the map has one spawn point, the level's own spawn markers are no longer used; with none in either, players start in the middle of the level. Scripts cannot read the map yet, things are placed with a button and numbers rather than by clicking and dragging in the picture, properties are added in the file rather than in the editor, and there is no undo (#41, #52). The editor is the `editor` Cargo feature of the `fossps` crate, on by default; the dedicated server is built without it.

`fossps controls` shows which key and button does what, and makes the text file that changes it: `bindings.toml` in the `fossps` folder of your settings (`~/.config` on Linux, `%APPDATA%` on Windows, `~/Library/Application Support` on macOS). Every action is listed there as a comment; take the `#` away from a line, edit it (`jump = ["F", "GamepadSouth"]`) and save, and every game played with FOSSPS uses it from its next start. A line with a mistake is skipped with a message saying which line and why, and the game still starts; a key that ends up doing two things gets a warning. There is no controls screen inside a game yet (#112).

## Scripting

A game is a folder with a `main.luau` in it, its level beside it as `level.glb`, and what is placed in the level as `map.toml`. `fossps new my-game` creates one that already plays, and `fossps run` plays the game in the folder you are in (or `fossps run my-game` from outside it). The engine runs `main.luau`, and loads it again every time you save it, without restarting:

```lua
game.onStart(function()
    print("Hello from Luau!")
end)

game.onTick(function(step)
    if game.input.justPressed("jump") then
        print("Jump!")
    end
end)
```

`print` writes to the terminal you started the game from. `game.input` reads actions by name (`jump`, `fire`, `move`, `look`, `reload` and [the rest of the standard set](crates/fossps-input/src/map.rs)), so the same line works for keyboard, mouse and gamepad. When a script has a mistake, the game keeps running, and the terminal and the game's window both show the file, the line and what to try:

```text
Script error in main.luau, line 1:

     1 | game.onTik(function() end)

  game.onTik does not exist. Did you mean game.onTick?
  The game keeps running the last version that worked. Fix this and save the file: it reloads by itself.
```

In the window the same words are in a panel in the top-left corner, over the game, until a save loads the script again. A packaged game does not show it: its players did not write the script and have no file to fix. The panel has been drawn and checked without a window; nobody has looked at it in a real one yet (#203).

A script can shake the gamepad: `game.input.rumble(0.2, 0.6, 0.1)` is how hard the heavy motor thumps and the light one buzzes, each from 0 to 1, then for how many seconds. It works on Linux and Windows for the player at the computer running the script (`fossps run`). macOS (#210) and players who joined from another computer (#211) do not rumble yet, the player cannot turn it down yet (#212), and nobody has felt it on a real gamepad yet (#29). Never make rumble the only sign of something: show or play it as well.

A game mode is one call. This is a whole team deathmatch: the engine shares players out over the teams, scores a point for killing an enemy, brings the dead back, and starts and ends the rounds.

```lua
game.mode {
    teams = { "Red", "Blue" },
    scoreToWin = 50,
    timeLimit = 600,
}

-- Rules of your own go in functions like this one.
game.onKill(function(killer, victim)
    if killer and killer.team == victim.team then
        killer.team:addScore(-1)
    end
end)
```

[`examples/team-deathmatch`](examples/team-deathmatch/main.luau) is a complete one with kill streaks, kept in a second file, and [`fossps.d.luau`](crates/fossps-script/types/fossps.d.luau) lists everything a script can call. Saving the script changes the rules of the running match and keeps the score. A player who joins a served game is put on a team and the round starts, so the join and round rules run in a real match; kills there, spawn points for each team and the scoreboard on screen are still being built (#171, #172), so the rest of the rules run under tests and `fossps check`.

So far scripts can set up a game mode and react to the game starting, to each tick, to input and to what happens in the match. A game starts from `main.luau` and can be split over files: `local streaks = require("./streaks")` runs `streaks.luau`, the file beside the one that asks, and gives back what that file returns, saving any of the files reloads the game, and an error names the file and line it is in. A reload starts every variable in the scripts over, except what is in a table from `game.keep`: `local saved = game.keep("saved", { best = 0 })` gives a table that still holds what was put in it (numbers, text, true and false, players, and tables of those) after a save. It lasts until the game is closed; nothing is written to a file. Scripts can only be required from inside the game's folder. A new game is also set up for [Claude Code](https://claude.com/claude-code): it has a `CLAUDE.md` that explains the script API and tells Claude to test each change with `fossps check`, which runs a second of the game and a pretend match with no window and fails if the script has a mistake.

A new game comes with autocomplete set up for VS Code: install the [Luau Language Server](https://marketplace.visualstudio.com/items?itemName=JohnnyMorganz.luau-lsp) extension and open the game's folder. For another editor, point the Luau language server at the `fossps.d.luau` in the game's folder.

## Contributing

Work is tracked in [issues](https://github.com/tahuffman1s/FOSSPS/issues) grouped by milestone. Changes land through pull requests that pass CI on Linux, Windows and macOS. Each pull request also gets an automated code review from Claude; you can ask it questions by mentioning `@claude` in a comment (maintainers only).

The drawing tests check every Vulkan call when the Vulkan validation layer is installed (`vulkan-validation-layers` on Fedora and Arch, `vulkan-validationlayers` on Debian and Ubuntu, the Vulkan SDK on Windows) and run without it otherwise. Run them with `FOSSPS_REQUIRE_VALIDATION=1` to make a layer that did not load a failure, as CI does on Linux. (With `cargo test --release`, also set `FOSSPS_VALIDATION=1`: release builds check nothing unless asked.)

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
Contributions are accepted under the same dual license.
