# Decisions

One dated line per non-obvious choice. Newest at the bottom.

- 2026-11-13 — Input lives in `fossps-input`, a crate with no platform dependencies, so the dedicated server, replays and tests use the same `InputFrame` without linking a window system.
- 2026-11-13 — Windowing is winit 0.30 and gamepads are gilrs 0.11, both behind the `window` feature of `fossps` (on by default) so a headless build links neither.
- 2026-11-13 — Keys are bound by physical position, not by the letter printed on them, so WASD works on AZERTY and Dvorak.
- 2026-11-13 — Gamepad buttons are named by position (`GamepadSouth`), not by Xbox or PlayStation labels.
- 2026-11-13 — An action has one of three kinds (button, 1D axis, 2D axis). No Unreal-style modifier and trigger stacks: too much to learn for the "Easy" pillar.
- 2026-11-13 — `look` is one action with two readings: mouse movement in raw counts (`delta`, never scaled by time) and stick tilt as a rate (`axis2d`). The game applies a separate sensitivity to each.
- 2026-11-13 — When several bindings drive one axis the strongest wins; they are not added, so keyboard plus stick is never faster than either.
- 2026-11-13 — Sticks use a circular scaled dead zone (inner 0.15, outer 0.95); triggers press at 0.5 and release at 0.4. gilrs's own filters are off so shaping happens once, in testable code.
- 2026-11-13 — Presses are latched per button between ticks: a tap shorter than a tick still counts once, and extra ticks in one frame do not repeat it.
- 2026-11-13 — A missing gamepad or keyboard binding is a startup warning, not an error, so a half-finished prototype still runs.
- 2026-11-13 — All connected gamepads feed one player; the active pad is the one last pushed firmly (past half way), so drift on an idle pad cannot take over. Gamepad input is ignored while the window is unfocused.
- 2026-11-13 — Simulation defaults to 60 ticks per second, with at most 5 catch-up ticks per frame and stalls over 250 ms dropped.
- 2026-11-13 — Until the renderer exists the main loop sleeps until the next tick, and `softbuffer` fills the window with a solid colour because Wayland hides windows that have drawn nothing. Both are replaced in #2.
- 2026-11-13 — Escape always frees the mouse (and still reaches the action map); the click that captures the mouse is not passed to the game.
