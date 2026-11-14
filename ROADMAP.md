# Roadmap

An overview only. Progress is tracked in [GitHub issues](https://github.com/tahuffman1s/FOSSPS/issues?q=label%3Aroadmap) labelled `roadmap`, built in issue order.

## M0 — Foundations

- Window and main loop; keyboard, mouse and gamepad feeding one action map (#1)
- Vulkan 1.3 device, swapchain and a triangle on screen (#2)
- Luau VM: run a game's `main.luau`, hot-reload on save, friendly script errors (#3)
- CLI: `fossps new <name>` creates a template game, `fossps run` plays it (#4)

## M1 — Playable offline

- Read glTF levels and models into engine data: meshes, PBR materials, textures (#5)
- Draw loaded meshes: vertex and index buffers, a depth buffer and a camera (#58)
- PBR shading: textures, metallic-roughness materials, normal maps and a sun light (#59)
- A game shows its level: `level.glb` in the game folder, and a camera scripts can move (#60)
- Collision and an FPS character controller (walk, jump, crouch, sprint) (#6)
- The player walks the level: movement actions drive the character controller, with the camera at their eyes (#64)
- Weapons: hitscan and projectile, recoil, reload (#7), then in the player's hands and exposed to scripts (#66)
- HUD and menus, fully navigable with a controller (#8), then drawn (#124), in a running game with a built-in pause menu (#125), built from scripts (#126), and scrolling with button hints (#127)
- Audio: positional sound, footsteps, gunfire (#9), then heard in a running game (#129), played from scripts (#130), with Ogg files and music (#131)
- Any frame rate: the game steps at a fixed tick, the picture is blended between ticks (#70) and aiming follows the mouse every drawn frame (#71)
- Scripts behave the same at any frame rate without delta-time maths (#72), proven by a test at 30, 60, 144 and 1000 frames a second (#73)

## M2 — Multiplayer

- Headless dedicated server binary with a fixed-tick simulation (#10)
- Snapshot replication with client prediction and reconciliation (#11)
- Netcode packets over UDP, with joining, leaving and timeouts (#161)
- Players move through the netcode in `fossps run` and `fossps serve` (#162)
- Other players move smoothly between snapshots (#163)
- Each player's waiting commands held at the target by nudging their tick rate (#164)
- Lag compensation for hit registration (#12)
- Game-mode script API (teams, rounds, spawns, scoring) and a sample team deathmatch (#13), then played in a running match with the map's spawn points (#171), with a scoreboard and round clock on screen (#172) and spawn points chosen out of enemies' sight (#173)
- Server browser or direct-connect flow that works in one step (#14)
- Shots and jumps count from the moment they were pressed, not the next tick (#74)
- Bad-connection simulator and a network graph (#109)

## M3 — Modern graphics and performance

- Clustered lighting, shadows, HDR and tonemapping, temporal anti-aliasing (#15)
- FSR 3.1 upscaling (first verify the SDK builds on Linux) (#16)
- Benchmark scene and frame-time budget checked in CI (#17)
- Frame pacing: an accurate frame cap, a display-sync choice, low caps in menus and background (#75)
- Lower input delay: late input reading, and AMD Anti-Lag 2 where the driver has it (#76)
- Hide what the player cannot see, with no hand-placed hints (#86)

## M4 — Ship

- `fossps package`: Linux `.tar.gz`, Windows `.exe`, macOS `.app` (MoltenVK; verify feasibility first) (#18), with an AppImage as an option (#121)
- Game pack: one `.fpk` file holding a game's files, friendly to small updates (#77)
- Packaging needs no Rust: ready-made engine builds are downloaded once, so any system can package for all three (#78)
- Only what the game uses goes in, with a size report that says why (#79), after checks that say what to fix (#80)
- Windows `.exe` with name, icon and version (#81), a macOS `.app` that opens (#82), and a dedicated server package (#83)
- Assets shrunk for size and speed by default: GPU textures, Opus sound (#85); `--exact` keeps every asset bit-for-bit (#84)
- Scripting in VS Code: problems as you type, plain-English snippets, run and reload (#100)
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

Making it easy without making it small:

- The game mode writes your to-do list (#46)
- Shapes sized to the player: cover, jump, climb (#47)
- Build while standing in the map, with a controller (#48)
- See travel time and sight lines on the map (#49)
- Mirror mode for two-team maps (#50)
- Script values become sliders (#51)
- No save button, and a history you can scrub (#52)
- One search box for everything, and panels that stay out of the way (#53)

Beyond boxes, still FPS only:

- Pull and push faces, snapped to player sizes (#87); doors, windows, arches and stairs in one click (#88)
- Grid materials that show scale, and one-click face painting (#89); a shelf of ready-made pieces (#90)
- Swap blockout for finished models without losing the gameplay (#96); lighting that looks good in one choice (#97)
- Name areas once for minimap, HUD and scripts (#92); the minimap draws itself (#93)
- Bots walk the map and report what is unreachable (#91), then paint heatmaps with a one-line reading (#94)
- Playtest with friends in one key, and watch the match back as ghosts (#95)
- See what each view costs to draw (#98)
- Sketch lanes from above and get a walkable blockout (#99)

## M6 — Script and game tools

Tools that make scripting and tuning a shooter easy. Not a general IDE: your own editor stays first-class, and the built-in panel does what only the engine can.

- A local link to the running game: logs, live values, reload (#101), also offered to AI assistants (#102)
- Script debugger: pause on a line and look at values (#103)
- Script panel in the editor: errors on the line, live values beside the code (#104)
- Drag a number, colour or position in a script and watch the game change (#105)
- Click a thing in the map to see its script, and back (#106)
- Rewind a match: replays you can scrub tick by tick, and kill cams (#107)
- Weapon workbench: draw recoil, see time-to-kill (#108)
- Game mode rules as a form with a plain-English summary (#110)
- HUD layout editor that never loses controller reach (#111)
- Bindings screen with gamepad pictures, and a check that every action has a gamepad binding (#112)
- Weapon animation timeline for fire and reload moments (#113)
- Footstep and impact sounds per surface (#114)
