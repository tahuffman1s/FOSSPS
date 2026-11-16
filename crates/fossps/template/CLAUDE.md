# {name}

A multiplayer FPS game made with [FOSSPS](https://github.com/tahuffman1s/FOSSPS). The game is a Luau script, `main.luau`, and a level, `level.glb`.

## Commands

```sh
fossps check    # run main.luau and load level.glb without a window; exits non-zero and prints the error if either has a mistake
fossps run      # play the game in a window (a person has to look at it and close it)
fossps package  # write a download for players to dist/ (Linux only so far); refuses a script with a mistake
```

Run them from this folder (or name it: `fossps check path/to/this-folder`).

Run `fossps check` after every change to `main.luau` and fix what it reports before saying the change is done. It runs the start handler and one second of ticks with nothing pressed, then plays a pretend match (two players join, the round starts, one kills the other, one dies alone, the round ends, both leave) so every `game.on...` function in the script is called at least once. While the round is on it presses, holds and lets go of every action, one at a time, pushes each stick all the way round and moves the mouse, so the code inside `if game.input.justPressed("jump")` runs too; after an error in that code it prints which action it was pressing. Expect the script's own prints for those presses in the output. It does not hold two actions together, and it does not reach a branch that needs more than a press or the pretend match, such as a shot that has to hit someone or a team kill. A mistake there is not caught: read that code carefully, and ask the person to try it with `fossps run`.

Do not start `fossps run` yourself unless asked: it opens a window and does not exit until the window is closed. While it is running, saving `main.luau` reloads the game, so there is no need to restart it.

## Writing scripts

- `fossps.d.luau` is the complete list of what a script can call, with an example for each. Read it before using anything on `game`; if it is not in that file, it does not exist. Do not edit the file: `fossps run` rewrites it when the engine is updated.
- Scripts register functions: `game.onStart`, `game.onTick(function(step) ... end)`, `game.onReload`. `step` is the length of one tick in seconds; multiply speeds by it.
- A game mode is one call near the top of the file: `game.mode { teams = { "Red", "Blue" }, scoreToWin = 50 }`. The engine then shares out players, scores kills, respawns the dead and runs the rounds by itself. Do not write timers or score counters for those; change the settings instead.
- Add rules with `game.onKill(function(killer, victim) ... end)`, `game.onPlayerJoin`, `game.onPlayerLeave`, `game.onSpawn`, `game.onRoundStart` and `game.onRoundEnd(function(winner) ... end)`. `killer` is nil when a player died alone. `player.team` is nil when the mode has no teams.
- Functions on a player or a team take a colon: `player:addScore(1)`, `team:players()`. Reading takes a dot: `player.name`, `team.score`. Scores only count while a round is being played.
- To remember something about a player, use a table with the player as the key (`streak[player] = 3`) and clear it in `game.onPlayerLeave`.
- The script runs on the server. Saving it changes the rules of the running match and keeps the score. Players can join a running game (`fossps serve`, then `fossps join`) and are put on teams, but nobody can be killed in one yet, so for now `fossps check` is how rules about kills and winning are tried.
- Read input by action name (`"jump"`, `"fire"`, `"move"`, `"look"`, ...), never by key, so keyboard, mouse and gamepad all work. The action names are listed in `fossps.d.luau`.
- `print()` goes to the terminal, not the window.
- Variables at the top of the file start over every time the file is saved and reloaded. To keep values across a reload, put them in a table from `game.keep`: `local saved = game.keep("saved", { best = 0 })`, then `saved.best += 1`. Change what is inside the table; `saved = {}` would replace it with a table that is not kept (empty it with `table.clear(saved)`), and `local n = saved.n` copies the number, so change `saved.n` itself. Use the same name on every save. It holds numbers, text, true and false, players (also as keys) and tables of those, not functions or teams (keep `team.name`). It lasts until the game is closed, not after.
- The script is sandboxed: no file access or network.
- The level is `level.glb` in this folder: one glTF Binary file, exported from Blender or another 3D tool (File > Export > glTF 2.0, format glTF Binary), in metres with +Y up. It is a binary file, so do not edit it as text; the person exports it. `fossps check` loads it and says what to change in the export if it cannot be read. It is drawn in its materials' colours and every shape in it is solid: players walk its floors and ramps and are stopped by its walls. Empty objects in it whose names begin with `spawn` are where players start while `map.toml` has no spawn point. `fossps check` starts a player at each spawn point and warns about one with no floor under it. Without the file, the game is played on the engine's flat test floor.
- What is placed in the level is `map.toml` in this folder (there is none until something is placed): TOML with `version = 1` and one `[[entity]]` each with `id` (any text no other entity has; never change one), `type`, optional `name`, `position = [x, y, z]` in metres, optional `facing` in degrees turned left from -Z, and optional `[entity.properties]` of text, numbers and true or false. Players come into the world at entities with `type = "spawn"`, looking the way each faces; once there is one, the level's own spawn markers are not used. `fossps edit` writes the file and it can be edited as text; a running game reloads it on save, and `fossps check` gives the line of a mistake. Scripts cannot read it yet.
- The game starts from `main.luau`. Split a growing game into files: `local weapons = require("./weapons")` runs `weapons.luau`, the file beside the one that asks, once, and gives back what that file returns (end it with `return weapons`; a file that returns nothing gives nil, which suits one that only adds `game.on...` functions). Paths start with `./` (beside the file that asks) or `../` (the folder above), use `/` for folders (`"./weapons/rifle"`), leave `.luau` off, and cannot leave the game's folder. Files cannot require each other in a circle. Saving any file reloads the whole game.
- A script error message names the file and the line and says what to try. Follow it before guessing.

## What the engine cannot do yet

FOSSPS is early and grows with each version. `fossps.d.luau` is the only list of what exists today. If a request needs something that is not in it, say so plainly; do not invent API. Running `fossps run` after updating FOSSPS refreshes `fossps.d.luau` with whatever is new.

## Style

- Luau, four-space indent, `camelCase` names, one idea per function.
- Write comments and printed text for someone new to programming.
