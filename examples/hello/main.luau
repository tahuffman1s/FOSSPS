-- hello: a FOSSPS game.
--
-- Play it by running this in the game's folder:
--     fossps run
-- Change anything here and save: the running game picks it up.
--
-- The window shows the game's level: level.glb, which you can export your own
-- over from Blender as glTF Binary. What happens in the game shows up as text:
-- print() writes to the terminal you ran `fossps run` from.

-- What is in this table is kept when the file is saved and loaded again,
-- until the game is closed. Everything else in the file starts over.
local saved = game.keep("saved", { jumps = 0 })

game.onStart(function()
    print("Welcome to hello! Click the game window so it can see your mouse.")
    print("Jump: Space, or the bottom face button on a gamepad.")
    print("Fire: left mouse button, or the right trigger.")
end)

-- This runs 60 times a second. `step` is the length of one tick in seconds.
game.onTick(function(step)
    if game.input.justPressed("jump") then
        saved.jumps += 1
        print("Jump number " .. saved.jumps)
    end

    if game.input.justPressed("fire") then
        local x, y = game.input.axis("move")
        print("Fire! Moving " .. x .. ", " .. y)
        -- A gamepad shakes with the shot. The numbers are how hard its heavy
        -- motor thumps, how hard its light motor buzzes, and for how long:
        -- game.input.rumble(strong, weak, seconds).
        game.input.rumble(0.2, 0.6, 0.1)
        -- The shot is heard too. "gunshot" is one of the engine's starter
        -- sounds. Put a gunshot.wav of your own in a folder called sounds,
        -- inside this folder, and that is played instead.
        game.audio.play("gunshot")
    end
end)

game.onReload(function()
    print("main.luau was saved and loaded again. Jumps so far: " .. saved.jumps)
end)
