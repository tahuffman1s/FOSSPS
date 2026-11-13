-- {name}: a FOSSPS game.
--
-- Play it by running this in the game's folder:
--     fossps run
-- Change anything here and save: the running game picks it up.
-- print() writes to the terminal you ran that from, not to the game window.

local jumps = 0

game.onStart(function()
    print("Welcome to {name}! Click the game window, then press jump:")
    print("Space on a keyboard, or the bottom face button on a gamepad.")
end)

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
    print("main.luau was saved and reloaded. The jump count starts again from 0.")
end)
