# {name}

A game made with [FOSSPS](https://github.com/tahuffman1s/FOSSPS).

## Play

In this folder, run:

```sh
fossps run
```

Click the window to capture the mouse; Escape frees it.
The window shows a test floor with a few blocks to walk between, until games have levels of their own. What the game does is printed in the terminal.

| Action | Keyboard and mouse | Gamepad |
| ------ | ------------------ | ------- |
| Jump   | Space              | Bottom face button |
| Fire   | Left mouse button  | Right trigger |
| Move   | W A S D            | Left stick |

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
