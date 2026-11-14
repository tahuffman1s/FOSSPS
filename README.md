# FOSSPS — FOSS Person Shooter

A free, open-source engine for multiplayer first-person shooters.

[Website](https://tahuffman1s.github.io/FOSSPS/) · [Roadmap](ROADMAP.md) · [Issues](https://github.com/tahuffman1s/FOSSPS/issues)

[![CI](https://github.com/tahuffman1s/FOSSPS/actions/workflows/ci.yml/badge.svg)](https://github.com/tahuffman1s/FOSSPS/actions/workflows/ci.yml)

**Status:** just started. Nothing is playable yet. So far: a window with a Vulkan 1.3 triangle in it, a fixed-rate main loop, keyboard, mouse and gamepad input through one action map, and Luau scripts that reload when you save them.

## Goals

- **Easy to use** — go from install to a playable networked match in minutes, with a Luau scripting API that reads like plain English.
- **Fast** — high, stable frame rates and low input latency; cheap dedicated servers.
- **Focused** — built only for multiplayer FPS games.
- **Modern** — Vulkan 1.3 renderer, FSR 3.1 upscaling, first-class controller support.

Planned platforms: Linux, Windows, and macOS as a stretch goal.

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
cargo run -- check my-game    # tests its script without opening a window
cargo run -- serve my-game    # runs it as a server, with no window (Ctrl-C stops it)
cargo run -- demo             # opens the input demo window
```

`cargo run --` stands in for the `fossps` command until there are packaged builds; to get a real `fossps` command now, run `cargo install --path crates/fossps`.

A machine that only hosts can use the separate `fossps-server` program, which does what `fossps serve` does and has no windowing, graphics, gamepad or sound code in it: `cargo build --release -p fossps-server`. It ticks the game 60 times a second, sleeps in between, logs a status line every minute and warns when it cannot keep up. Players cannot join a server yet. The netcode itself is written and tested (`fossps-net`: commands, snapshots, prediction and correction), but it is not connected to a socket or to the running game yet.

Running it needs a graphics driver with Vulkan 1.3; building does not need the Vulkan SDK. If it says Vulkan is missing or too old, update your graphics driver (on Linux, also install your distribution's Vulkan loader package, for example `sudo dnf install vulkan-loader`). The input demo draws a triangle and shows which actions are active in the window title. Click the window to capture the mouse, then try the keyboard, mouse or a gamepad; Escape frees the mouse.

## Scripting

A game is a folder with a `main.luau` in it. `fossps new my-game` creates one that already plays, and `fossps run` plays the game in the folder you are in (or `fossps run my-game` from outside it). The engine runs `main.luau`, and loads it again every time you save it, without restarting:

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

`print` writes to the terminal you started the game from. `game.input` reads actions by name (`jump`, `fire`, `move`, `look`, `reload` and [the rest of the standard set](crates/fossps-input/src/map.rs)), so the same line works for keyboard, mouse and gamepad. When a script has a mistake, the game keeps running and the terminal shows the file, the line and what to try:

```text
Script error in main.luau, line 1:

     1 | game.onTik(function() end)

  game.onTik does not exist. Did you mean game.onTick?
  The game keeps running the last version that worked. Fix this and save the file: it reloads by itself.
```

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

[`examples/team-deathmatch`](examples/team-deathmatch/main.luau) is a complete one with kill streaks, and [`fossps.d.luau`](crates/fossps-script/types/fossps.d.luau) lists everything a script can call. Saving the script changes the rules of the running match and keeps the score. The rules run today under tests and `fossps check`; players joining a running game over the network and the scoreboard on screen are still being built (#171, #172).

So far scripts can set up a game mode and react to the game starting, to each tick, to input and to what happens in the match, and a game is a single `main.luau` (`require` comes later). A new game is also set up for [Claude Code](https://claude.com/claude-code): it has a `CLAUDE.md` that explains the script API and tells Claude to test each change with `fossps check`, which runs a second of the game and a pretend match with no window and fails if the script has a mistake.

A new game comes with autocomplete set up for VS Code: install the [Luau Language Server](https://marketplace.visualstudio.com/items?itemName=JohnnyMorganz.luau-lsp) extension and open the game's folder. For another editor, point the Luau language server at the `fossps.d.luau` in the game's folder.

## Contributing

Work is tracked in [issues](https://github.com/tahuffman1s/FOSSPS/issues) grouped by milestone. Changes land through pull requests that pass CI on Linux, Windows and macOS. Each pull request also gets an automated code review from Claude; you can ask it questions by mentioning `@claude` in a comment (maintainers only).

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
Contributions are accepted under the same dual license.
