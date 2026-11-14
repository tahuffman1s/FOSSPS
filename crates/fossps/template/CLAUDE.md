# {name}

A multiplayer FPS game made with [FOSSPS](https://github.com/tahuffman1s/FOSSPS). The whole game is one Luau script, `main.luau`.

## Commands

```sh
fossps check    # run main.luau without a window; exits non-zero and prints the error if it has a mistake
fossps run      # play the game in a window (a person has to look at it and close it)
```

Run both from this folder (or name it: `fossps check path/to/this-folder`).

Run `fossps check` after every change to `main.luau` and fix what it reports before saying the change is done. It runs the start handler and one second of ticks with nothing pressed, then plays a pretend match (two players join, the round starts, one kills the other, one dies alone, the round ends, both leave) so every `game.on...` function in the script is called at least once. Code inside an `if game.input...` branch is loaded but not run, and neither is a branch the pretend match does not reach, such as a team kill. A mistake there, such as a misspelled action name, is not caught: read that code carefully, and ask the person to try it with `fossps run`.

Do not start `fossps run` yourself unless asked: it opens a window and does not exit until the window is closed. While it is running, saving `main.luau` reloads the game, so there is no need to restart it.

## Writing scripts

- `fossps.d.luau` is the complete list of what a script can call, with an example for each. Read it before using anything on `game`; if it is not in that file, it does not exist. Do not edit the file: `fossps run` rewrites it when the engine is updated.
- Scripts register functions: `game.onStart`, `game.onTick(function(step) ... end)`, `game.onReload`. `step` is the length of one tick in seconds; multiply speeds by it.
- A game mode is one call near the top of the file: `game.mode { teams = { "Red", "Blue" }, scoreToWin = 50 }`. The engine then shares out players, scores kills, respawns the dead and runs the rounds by itself. Do not write timers or score counters for those; change the settings instead.
- Add rules with `game.onKill(function(killer, victim) ... end)`, `game.onPlayerJoin`, `game.onPlayerLeave`, `game.onSpawn`, `game.onRoundStart` and `game.onRoundEnd(function(winner) ... end)`. `killer` is nil when a player died alone. `player.team` is nil when the mode has no teams.
- Functions on a player or a team take a colon: `player:addScore(1)`, `team:players()`. Reading takes a dot: `player.name`, `team.score`. Scores only count while a round is being played.
- To remember something about a player, use a table with the player as the key (`streak[player] = 3`) and clear it in `game.onPlayerLeave`.
- The script runs on the server. Saving it changes the rules of the running match and keeps the score. Players cannot join a running game yet, so for now `fossps check` is how the rules are tried.
- Read input by action name (`"jump"`, `"fire"`, `"move"`, `"look"`, ...), never by key, so keyboard, mouse and gamepad all work. The action names are listed in `fossps.d.luau`.
- `print()` goes to the terminal, not the window.
- Variables at the top of the file start over every time the file is saved and reloaded.
- The script is sandboxed: no file access or network. `require` does not work yet, so keep everything in `main.luau`.
- A script error message names the line and says what to try. Follow it before guessing.

## What the engine cannot do yet

FOSSPS is early and grows with each version. `fossps.d.luau` is the only list of what exists today. If a request needs something that is not in it, say so plainly; do not invent API. Running `fossps run` after updating FOSSPS refreshes `fossps.d.luau` with whatever is new.

## Style

- Luau, four-space indent, `camelCase` names, one idea per function.
- Write comments and printed text for someone new to programming.
