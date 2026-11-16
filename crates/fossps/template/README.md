# {name}

A game made with [FOSSPS](https://github.com/tahuffman1s/FOSSPS).

## Play

In this folder, run:

```sh
fossps run
```

Click the window to capture the mouse; Escape frees it.
The window shows the game's level: a small arena with a red end and a blue end. What the game does is printed in the terminal.

## The level

The level is `level.glb` in this folder. To make your own, build it in Blender (or another 3D tool), then export it over that file: File > Export > glTF 2.0, with the format set to glTF Binary (.glb). One unit is one metre, and a player is 1.8 m tall. `fossps edit` flies around the level, and `fossps check` says what to change if it cannot be read. Start the game again to see a level you have saved.

The level is drawn in its materials' colours, and it is not solid yet: players stand on a flat floor at height 0 and walk through walls.

| Action | Keyboard and mouse | Gamepad |
| ------ | ------------------ | ------- |
| Jump   | Space              | Bottom face button |
| Fire   | Left mouse button  | Right trigger |
| Move   | W A S D            | Left stick |

## Sounds

Firing plays the engine's starter gunshot. To use a sound of your own, make a folder called `sounds` in this folder and put a WAV file in it: `sounds/gunshot.wav` replaces the starter gunshot, and any other name is a new sound, so `sounds/door.wav` is played with `game.audio.play("door")`. A sound saved while the game runs is picked up. Only WAV files load for now. Only use sounds you made or are allowed to share, and keep their licence beside them.

## Change it

The game is `main.luau`. Edit it and save: the running game loads the new version by itself.
If there is a mistake, the terminal shows the line and what to try, and the game keeps running the last version that worked.

When `main.luau` gets long, move part of it to another file in this folder, such as `weapons.luau`, end that file with `return weapons`, and get it in `main.luau` with `local weapons = require("./weapons")`. Saving either file reloads the game.

To test the script without opening a window, run `fossps check`.

## Share it

To make a download other people can play, run this in this folder on a Linux computer:

```sh
fossps package
```

It writes `dist/{name}-linux-x86_64.tar.gz` (a space in the name becomes a `-`). A player unpacks it and runs
the program inside; they do not need FOSSPS. Put a square PNG picture called `icon.png` in this folder to
give the game an icon. Packages for Windows and macOS are not built yet.

## Working with Claude

This folder is set up for [Claude Code](https://claude.com/claude-code): `CLAUDE.md` tells it how the game
works and how to test its changes with `fossps check`. Open the folder in Claude Code and say what you want
the game to do. `CLAUDE.md` is yours: add your own rules and notes about the game to it.

## Autocomplete

`fossps.d.luau` describes everything a script can use. In VS Code, install the
[Luau Language Server](https://marketplace.visualstudio.com/items?itemName=JohnnyMorganz.luau-lsp)
extension and open this folder: `.vscode/settings.json` already points it at that file.
`fossps.d.luau` belongs to the engine: `fossps run` rewrites it when FOSSPS is updated, so don't edit it.
