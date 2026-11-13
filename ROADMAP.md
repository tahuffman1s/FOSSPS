# Roadmap

Work top to bottom. `/next` takes the first unchecked item. Add follow-ups where they belong; keep items small enough to finish and verify in one session.

## M0 — Foundations

- [ ] Window and main loop; keyboard, mouse and gamepad feeding one action map
- [ ] Vulkan 1.3 device, swapchain and a triangle on screen
- [ ] Luau VM: run a game's `main.luau`, hot-reload on save, friendly script errors
- [ ] CLI: `fossps new <name>` creates a template game, `fossps run` plays it

## M1 — Playable offline

- [ ] glTF level and model loading with PBR materials
- [ ] Collision and an FPS character controller (walk, jump, crouch, sprint)
- [ ] Weapons: hitscan and projectile, recoil, reload, exposed to scripts
- [ ] HUD and menus, fully navigable with a controller
- [ ] Audio: positional sound, footsteps, gunfire

## M2 — Multiplayer

- [ ] Headless dedicated server binary with a fixed-tick simulation
- [ ] Snapshot replication with client prediction and reconciliation
- [ ] Lag compensation for hit registration
- [ ] Game-mode script API (teams, rounds, spawns, scoring) and a sample team deathmatch
- [ ] Server browser or direct-connect flow that works in one step

## M3 — Modern graphics and performance

- [ ] Clustered lighting, shadows, HDR and tonemapping, temporal anti-aliasing
- [ ] FSR 3.1 upscaling (first verify the SDK builds on Linux)
- [ ] Benchmark scene and frame-time budget checked in CI

## M4 — Ship

- [ ] `fossps package`: Linux AppImage, Windows `.exe`, macOS `.app` (MoltenVK; verify feasibility first)
- [ ] Release workflow that builds and uploads all three on a version tag
- [ ] Docs site with a ten-minute "first match" tutorial
