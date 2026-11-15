-- Team Deathmatch: a FOSSPS game mode.
--
-- Check it for mistakes by running this in the game's folder:
--     fossps check
-- Change anything here and save: a running game picks it up and keeps the score
-- and the kill streaks.

-- The score a team needs to win a round.
local scoreToWin = 50

-- This one call is a whole team deathmatch. The engine puts each new player
-- on the smallest team, scores a point for killing an enemy, brings the dead
-- back, and starts and ends the rounds. Times are in seconds.
game.mode {
    name = "Team Deathmatch",
    teams = { "Red", "Blue" },
    scoreToWin = scoreToWin,
    timeLimit = 600,
    respawnDelay = 3,
}

-- Everything below adds to those rules. Delete any of it and the mode still
-- plays. print() writes to the server's terminal. This mode has teams, so a
-- player's team is never nil here.

-- Kill streaks are counted in streaks.luau, the file beside this one. require
-- runs that file and gives back the table it returns. Saving either file
-- reloads the game.
local streaks = require("./streaks")

game.onPlayerJoin(function(player)
    print(player.name .. " joined " .. player.team.name .. ".")
end)

game.onPlayerLeave(function(player)
    streaks.forget(player)
    print(player.name .. " left.")
end)

game.onRoundStart(function()
    streaks.clear()
    print("The round is on! First team to " .. scoreToWin .. " wins.")
end)

-- `killer` is nil when nobody else did it, such as a fall.
game.onKill(function(killer, victim)
    streaks.forget(victim)
    if killer == nil then
        print(victim.name .. " died.")
    elseif killer.team == victim.team then
        -- Killing a team-mate scores nothing by itself. Here it costs a point.
        killer.team:addScore(-1)
        print(killer.name .. " killed a team-mate. " .. killer.team.name .. " loses a point.")
    else
        local kills = streaks.add(killer)
        print(killer.name .. " killed " .. victim.name .. ".")
        -- Every fifth kill without dying is worth a bonus point.
        if kills % 5 == 0 then
            killer.team:addScore(1)
            print(killer.name .. " is on a streak of " .. kills .. "! Bonus point.")
        end
    end
end)

game.onRoundEnd(function(winner)
    if winner then
        print(winner.name .. " wins the round!")
    else
        print("The round is a draw.")
    end
    -- This visits each team in turn.
    for _, team in game.teams() do
        print(team.name .. ": " .. team.score)
    end
end)
