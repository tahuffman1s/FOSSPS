-- Kill streaks: how many kills each player has made since they last died.
--
-- main.luau gets this file's table with: local streaks = require("./streaks")

local streaks = {}

-- The count for each player. game.keep gives a table that is still there,
-- with the counts in it, after either file is saved and the game reloads. A
-- plain `local kills = {}` would start empty on every save.
local kills = game.keep("streaks")

-- Counts one more kill for a player and gives back their streak so far.
function streaks.add(player)
    kills[player] = (kills[player] or 0) + 1
    return kills[player]
end

-- Ends a player's streak: they died, or left the game.
function streaks.forget(player)
    kills[player] = nil
end

-- Ends every streak, for a new round.
function streaks.clear()
    -- This empties the kept table. `kills = {}` would swap it for a new
    -- table that is not kept.
    table.clear(kills)
end

return streaks
