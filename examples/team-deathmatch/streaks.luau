-- Kill streaks: how many kills each player has made since they last died.
--
-- main.luau gets this file's table with: local streaks = require("./streaks")

local streaks = {}

-- The count for each player. Nothing outside this file can reach it.
local kills = {}

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
    kills = {}
end

return streaks
