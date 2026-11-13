-- {name}: a FOSSPS game.
--
-- Play it by running this in the game's folder:
--     fossps run
-- Change anything here and save: the running game picks it up.
--
-- The window only shows a triangle for now. What happens in the game shows up
-- as text: print() writes to the terminal you ran `fossps run` from.

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
    end
end)

game.onReload(function()
    print("main.luau was saved and loaded again, so the jump count starts from 0.")
end)
