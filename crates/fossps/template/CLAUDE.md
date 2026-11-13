# {name}

A multiplayer FPS game made with [FOSSPS](https://github.com/tahuffman1s/FOSSPS). The whole game is one Luau script, `main.luau`.

## Commands

```sh
fossps check    # run main.luau without a window; exits non-zero and prints the error if it has a mistake
fossps run      # play the game in a window (a person has to look at it and close it)
```

Run `fossps check` after every change to `main.luau` and fix what it reports before saying the change is done. It runs the start handler and one second of ticks with nothing pressed, so code inside an `if game.input...` branch is loaded but not run: read that code carefully, and ask the person to try it with `fossps run`.

Do not start `fossps run` yourself unless asked: it opens a window and does not exit until the window is closed. While it is running, saving `main.luau` reloads the game, so there is no need to restart it.

## Writing scripts

- `fossps.d.luau` is the complete list of what a script can call, with an example for each. Read it before using anything on `game`; if it is not in that file, it does not exist. Do not edit the file: the engine rewrites it.
- Scripts register functions: `game.onStart`, `game.onTick(function(step) ... end)`, `game.onReload`. `step` is the length of one tick in seconds; multiply speeds by it.
- Read input by action name (`"jump"`, `"fire"`, `"move"`, `"look"`, ...), never by key, so keyboard, mouse and gamepad all work. The action names are listed in `fossps.d.luau`.
- `print()` goes to the terminal, not the window.
- Variables at the top of the file start over every time the file is saved and reloaded.
- The script is sandboxed: no `require`, file access or network. Keep everything in `main.luau`.
- A script error message names the line and says what to try. Follow it before guessing.

## What the engine cannot do yet

FOSSPS is early. The window draws a test triangle; there are no levels, players, weapons, HUD, sound or networking yet, so a script can only react to the game starting, to ticks and to input, and print. If a request needs something that is not in `fossps.d.luau`, say so plainly; do not invent API. Running `fossps run` after updating FOSSPS refreshes `fossps.d.luau` with whatever is new.

## Style

- Luau, four-space indent, `camelCase` names, one idea per function.
- Write comments and printed text for someone new to programming.
