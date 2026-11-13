-- FOSSPS script API.
--
-- Point the Luau language server at this file to get autocomplete and type
-- checking for `game` in your editor.
--
-- `print(...)` writes a line to the terminal the game was started from.

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
    --- the game is running. Variables in the file start over on a reload.
    --- Example: game.onReload(function() print("New version loaded") end)
    onReload: (handler: () -> ()) -> (),

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
    },
}
