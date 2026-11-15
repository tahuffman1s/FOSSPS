-- {name}: a FOSSPS game.
--
-- Play it by running this in the game's folder:
--     fossps run
-- Change anything here and save: the running game picks it up.
--
-- The window shows a test floor with a few blocks to walk between, until
-- games have levels of their own. What happens in the game shows up as text:
-- print() writes to the terminal you ran `fossps run` from.

local jumps = 0

game.onStart(function()
    print("Welcome to {name}! Click the game window so it can see your mouse.")
    print("Jump: Space, or the bottom face button on a gamepad.")
    print("Fire: left mouse button, or the right trigger.")
end)

-- This runs 60 times a second. `step` is the length of one tick in seconds.
game.onTick(function(step)
    if game.input.justPressed("jump") then
        jumps += 1
        print("Jump number " .. jumps)
    end

    if game.input.justPressed("fire") then
        local x, y = game.input.axis("move")
        print("Fire! Moving " .. x .. ", " .. y)
        -- A gamepad shakes with the shot. The numbers are how hard its heavy
        -- motor thumps, how hard its light motor buzzes, and for how long:
        -- game.input.rumble(strong, weak, seconds).
        game.input.rumble(0.2, 0.6, 0.1)
    end
end)

game.onReload(function()
    print("main.luau was saved and loaded again, so the jump count starts from 0.")
end)
