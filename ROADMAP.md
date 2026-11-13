# Roadmap

An overview only. Progress is tracked in [GitHub issues](https://github.com/tahuffman1s/FOSSPS/issues?q=label%3Aroadmap) labelled `roadmap`, built in issue order.

## M0 — Foundations

- Window and main loop; keyboard, mouse and gamepad feeding one action map (#1)
- Vulkan 1.3 device, swapchain and a triangle on screen (#2)
- Luau VM: run a game's `main.luau`, hot-reload on save, friendly script errors (#3)
- CLI: `fossps new <name>` creates a template game, `fossps run` plays it (#4)

## M1 — Playable offline

- glTF level and model loading with PBR materials (#5)
- Collision and an FPS character controller (walk, jump, crouch, sprint) (#6)
- Weapons: hitscan and projectile, recoil, reload, exposed to scripts (#7)
- HUD and menus, fully navigable with a controller (#8)
- Audio: positional sound, footsteps, gunfire (#9)

## M2 — Multiplayer

- Headless dedicated server binary with a fixed-tick simulation (#10)
- Snapshot replication with client prediction and reconciliation (#11)
- Lag compensation for hit registration (#12)
- Game-mode script API (teams, rounds, spawns, scoring) and a sample team deathmatch (#13)
- Server browser or direct-connect flow that works in one step (#14)

## M3 — Modern graphics and performance

- Clustered lighting, shadows, HDR and tonemapping, temporal anti-aliasing (#15)
- FSR 3.1 upscaling (first verify the SDK builds on Linux) (#16)
- Benchmark scene and frame-time budget checked in CI (#17)

## M4 — Ship

- `fossps package`: Linux AppImage, Windows `.exe`, macOS `.app` (MoltenVK; verify feasibility first) (#18)
- Release workflow that builds and uploads all three on a version tag (#19)
- Docs site with a ten-minute "first match" tutorial (#20)

## M5 — Editor

A graphical editor for FPS maps and game modes only, built after the engine ships. No terrain, material graphs or visual scripting.

- `fossps edit`: open a game in edit mode with a fly camera and grid (#38)
- Panels on the Vulkan renderer: entity list and inspector (#39)
- Map file for entities, saved as text beside the level (#40)
- Place and move spawn points, zones and objectives (#41)
- Play from where you stand, and return to editing with one key (#42)
- Blockout boxes and ramps, and dropping in models (#43)
- Checks that say what to fix before you play (#44)
