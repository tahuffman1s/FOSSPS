-- FOSSPS script API.
--
-- Point the Luau language server at this file to get autocomplete and type
-- checking for `game` in your editor.
--
-- `print(...)` writes a line to the terminal the game was started from.
--
-- `require("./weapons")` runs weapons.luau, the file beside the script that
-- asks, once, and gives back what that file returns. Use it to split a game
-- over files. Start the path with `./` (beside this script) or `../` (the
-- folder above), write `/` between folders and leave `.luau` off:
--     local rifle = require("./weapons/rifle")
-- End the required file with `return` and the table to hand over. A file
-- that returns nothing gives nil, which is fine for one that only adds
-- `game.on...` functions. Only scripts inside the game's folder can be
-- required, and scripts cannot require each other in a circle.

--- A team in the match. Get one from `player.team`, `game.team("Red")` or
--- `game.teams()`.
declare class Team
    --- The team's name, as written in `game.mode`.
    name: string

    --- The team's score in this round. It goes back to 0 when a round starts.
    score: number

    --- The players on this team right now.
    --- Example: for _, player in team:players() do print(player.name) end
    function players(self): { Player }

    --- Adds points to the team's score. A negative number takes points away.
    --- Reaching `scoreToWin` wins the round. Only counts while a round is
    --- being played: during the warm-up and after the round it does nothing.
    --- Example: player.team:addScore(5)
    function addScore(self, points: number): ()
end

--- A player in the match. The same player is always the same value, so a
--- player can be a key in a table of your own: `streak[player] = 3`.
declare class Player
    --- The name the player goes by.
    name: string

    --- The player's team, or nil when the mode has no teams.
    --- Example: if killer.team == victim.team then print("Team kill!") end
    team: Team?

    --- The player's own score in this round. With no teams, the first
    --- player to reach `scoreToWin` wins.
    score: number

    --- How many players this player has killed in this round.
    kills: number

    --- How many times this player has died in this round.
    deaths: number

    --- True while the player is in the world; false while they wait to
    --- respawn.
    alive: boolean

    --- Adds points to the player's own score. A negative number takes points
    --- away. Only counts while a round is being played.
    --- Example: player:addScore(1)
    function addScore(self, points: number): ()
end

declare game: {
    --- Calls `handler` once, on the first tick, when the game starts.
    --- Example: game.onStart(function() print("Go!") end)
    onStart: (handler: () -> ()) -> (),

    --- Calls `handler` on every simulation tick. `step` is the length of one
    --- tick in seconds. It is always the same (1/60 by default), so
    --- `position += speed * step` moves at the same speed on every computer.
    --- You can add several handlers; they run in the order they were added.
    --- Example: game.onTick(function(step) timeLeft -= step end)
    onTick: (handler: (step: number) -> ()) -> (),

    --- Calls `handler` each time this file is saved and loaded again while
    --- the game is running. Variables in the file start over on a reload;
    --- what is in a `game.keep` table does not.
    --- Example: game.onReload(function() print("New version loaded") end)
    onReload: (handler: () -> ()) -> (),

    --- Gives a table that keeps what you put in it when the script is saved
    --- and loaded again. Everything else in the file starts over on a
    --- reload. `name` is how the table is found again, so use the same name
    --- on every save. `start` is what the table holds the first time; leave
    --- it out for an empty table. Call it at the top of the file.
    ---
    --- Change what is inside the table. `saved = {}` would swap in a new
    --- table that is not kept (to empty it, write `table.clear(saved)`), and
    --- `local best = saved.best` copies the number, so write `saved.best`
    --- where you change it.
    ---
    --- A kept table can hold numbers, text, true and false, players, and
    --- tables of those. A player can be a key: `streaks[player] = 3`.
    --- Players who left are taken out. Functions and teams are not kept
    --- (keep `team.name`).
    ---
    --- What `start` has and the kept table lacks is added on the next save,
    --- also something you set to nil; what it already holds is not changed.
    --- To start a table over, give it a new name or start the game again.
    --- Nothing is written to a file: kept tables last until the game is
    --- closed.
    --- Example: local saved = game.keep("saved", { best = 0 })
    keep: (<T>(name: string, start: T) -> T) & ((name: string) -> { [any]: any }),

    --- Sets up the match: its teams and its numbers. Call it once, near the
    --- top of the file. Every setting can be left out. The script runs on the
    --- server, so players cannot change the rules or the score. Saving
    --- the file with new numbers changes the running match and keeps the
    --- score; changing the teams starts the match again.
    ---
    --- With this alone the game is a complete deathmatch: players are put on
    --- the smallest team, a kill scores, the dead respawn, and rounds start
    --- and end by themselves. The `game.on...` functions below are for
    --- adding rules of your own.
    --- Example: game.mode { teams = { "Red", "Blue" }, scoreToWin = 50 }
    mode: (settings: {
        --- What the mode is called. "Team Deathmatch" if left out and there
        --- are teams, "Deathmatch" if there are none.
        name: string?,
        --- The teams' names. Leave it out and it is every player for
        --- themselves.
        teams: { string }?,
        --- The score that wins the round: a team's score, or a player's when
        --- there are no teams. 50 if left out. 0 means no score ends the round.
        scoreToWin: number?,
        --- What killing an enemy is worth, to the killer and to their team.
        --- 1 if left out. Use 0 if your mode scores something else.
        killPoints: number?,
        --- How long a round lasts, in seconds. When it runs out the highest
        --- score wins. 600 (ten minutes) if left out. 0 means no time limit.
        timeLimit: number?,
        --- How long a player waits to come back after dying, in seconds.
        --- 3 if left out.
        respawnDelay: number?,
        --- How long players can run around before the round starts counting,
        --- in seconds. 5 if left out.
        warmup: number?,
        --- How long the final score stays up before the next round, in
        --- seconds. 10 if left out.
        intermission: number?,
    }) -> (),

    --- Calls `handler` when a player joins the match. Players who were
    --- already here when the file was saved and reloaded do not join again:
    --- use `game.players()` for them.
    --- Example: game.onPlayerJoin(function(player) print(player.name .. " joined") end)
    onPlayerJoin: (handler: (player: Player) -> ()) -> (),

    --- Calls `handler` when a player leaves the match. `player` can still be
    --- read inside the handler, and not after it.
    --- Example: game.onPlayerLeave(function(player) streak[player] = nil end)
    onPlayerLeave: (handler: (player: Player) -> ()) -> (),

    --- Calls `handler` when a player dies. `killer` is nil when nobody else
    --- did it, such as a fall. The kill has already been counted and scored
    --- (see `killPoints`), so add only what is special about your mode.
    --- Example: game.onKill(function(killer, victim) print(victim.name .. " died") end)
    onKill: (handler: (killer: Player?, victim: Player) -> ()) -> (),

    --- Calls `handler` each time a player comes into the world: when they
    --- join, when they respawn, and when a round starts.
    --- Example: game.onSpawn(function(player) print(player.name .. " is in") end)
    onSpawn: (handler: (player: Player) -> ()) -> (),

    --- Calls `handler` when the warm-up ends and the round starts counting.
    --- Every score is back to 0.
    --- Example: game.onRoundStart(function() print("Go!") end)
    onRoundStart: (handler: () -> ()) -> (),

    --- Calls `handler` when the round ends. `winner` is the winning team (or
    --- the winning player when there are no teams), or nil for a draw. Both
    --- have a `name` and a `score`.
    --- Example: game.onRoundEnd(function(winner) if winner then print(winner.name .. " wins!") end end)
    onRoundEnd: (handler: (winner: Team | Player | nil) -> ()) -> (),

    --- Everyone in the match right now, in the order they joined.
    --- Example: for _, player in game.players() do print(player.name, player.kills) end
    players: () -> { Player },

    --- All the teams, in the order `game.mode` lists them. Empty when the
    --- mode has no teams. For one team, use `game.team("Red")`.
    --- Example: for _, team in game.teams() do print(team.name, team.score) end
    teams: () -> { Team },

    --- The one team with this name. Capital letters do not matter. For all
    --- of them, use `game.teams()`.
    --- Example: game.team("Red"):addScore(5)
    team: (name: string) -> Team,

    --- The round being played. A match is one round after another: a short
    --- warm-up, the round itself, then the final score for a while.
    round: {
        --- Where the round is: "waiting" (nobody is here), "warmup",
        --- "playing" or "over". It is a function, so write the brackets:
        --- `game.round.phase()`.
        --- Example: if game.round.phase() == "playing" then print("The round is on") end
        phase: () -> "waiting" | "warmup" | "playing" | "over",

        --- Seconds until the current phase ends by itself. It is `math.huge`
        --- while waiting for players and in a round with no time limit. It
        --- is a function, so write the brackets.
        --- Example: if game.round.timeLeft() < 60 then print("Last minute!") end
        timeLeft: () -> number,

        --- Ends the round now. Give it the winning team (or player, when
        --- there are no teams), or nothing for a draw. Does nothing unless a
        --- round is being played.
        --- Example: game.round.finish(game.team("Red"))
        finish: (winner: Team | Player | nil) -> (),
    },

    --- What the player is doing right now, by action name. An action works
    --- the same for keyboard, mouse and gamepad.
    ---
    --- The standard actions are: "move" and "look" (two directions), and the
    --- buttons "jump", "crouch", "sprint", "fire", "aim", "reload", "interact",
    --- "melee", "grenade", "next_weapon", "previous_weapon", "scoreboard" and
    --- "pause". Action names are lower case with underscores.
    input: {
        --- True while the action is held down, or its stick is off centre.
        --- Example: if game.input.pressed("sprint") then speed = 8 end
        pressed: (action: string) -> boolean,

        --- True only on the tick the action was first pressed.
        --- Example: if game.input.justPressed("jump") then print("Jump!") end
        justPressed: (action: string) -> boolean,

        --- True only on the tick the action was let go.
        --- Example: if game.input.justReleased("aim") then print("Stopped aiming") end
        justReleased: (action: string) -> boolean,

        --- How hard a button action is pressed, from 0 to 1. A key is 0 or 1;
        --- a gamepad trigger can be in between. For "move" and "look" use
        --- `axis` instead.
        --- Example: local squeeze = game.input.value("fire")
        value: (action: string) -> number,

        --- The x and y of a two-direction action, "move" or "look", each from
        --- -1 to 1. Right and up are positive.
        --- Example: local x, y = game.input.axis("move")
        axis: (action: string) -> (number, number),

        --- How far the mouse moved since the last tick, for an action the
        --- mouse drives, such as "look": x and y, right and up positive.
        --- It is 0, 0 when the player is using a stick. It is already a
        --- distance, so do not multiply it by `step`.
        --- Example: local x, y = game.input.mouse("look")
        mouse: (action: string) -> (number, number),

        --- Shakes the player's gamepad. `strong` is the heavy motor (a
        --- thump) and `weak` the light one (a buzz), each from 0 (off) to 1
        --- (full). It stops by itself after `seconds`: 0.2 if left out, and
        --- at most 5. Numbers outside those ranges are brought into them. A
        --- new rumble replaces the one playing, so `rumble(0, 0)` stops it.
        --- Only the player at the computer running the script feels it
        --- (`fossps run`), on a gamepad, on Linux and Windows.
        --- Example: if game.input.justPressed("fire") then game.input.rumble(0.2, 0.6, 0.1) end
        rumble: (strong: number, weak: number, seconds: number?) -> (),
    },
}
