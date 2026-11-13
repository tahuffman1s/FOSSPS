# FOSSPS (FOSS Person Shooter)

Free, open-source engine for multiplayer FPS games (Call of Duty / Counter-Strike / Battlefield class).
Rust core, Luau scripting, Vulkan 1.3. Licensed MIT OR Apache-2.0.

## Pillars (earlier wins when they conflict)

1. **Easy** — install to a playable networked match in minutes. Script API reads like plain English. Error messages say what to do next.
2. **Fast** — high, stable frame rate and low input latency on mid-range hardware; dedicated servers are cheap to run.
3. **Focused** — multiplayer FPS only. Decline general-purpose engine features.
4. **Modern** — current rendering techniques, upscaling, first-class controllers.

## Decisions

- Rust stable, edition 2024. Cargo workspace; crates live in `crates/`.
- One render backend: Vulkan 1.3 (Windows + Linux). FSR needs raw Vulkan handles, so no abstraction layer that hides them.
- Upscaling: FSR 3.1 (MIT) only. No closed-source upscalers (XeSS was dropped: closed, Windows-only).
- Scripting: Luau through `mlua` (`luau` feature). Sandboxed, hot-reloadable, typed API definitions shipped for editor autocomplete.
- Netcode: server-authoritative, client prediction, lag compensation. Headless dedicated server is a first-class binary.
- Controller parity: every action is bindable on a gamepad; nothing ships keyboard/mouse-only.
- Targets: Linux (AppImage), Windows (`.exe`), macOS (`.app` through MoltenVK — stretch goal, feasibility unverified). Keep code portable; CI builds all three.

## Autonomy

The owner wants to give very little input. `/next` builds the next roadmap issue end to end.

- Decide, don't ask. Pick the option that best serves the pillars and log non-obvious choices as one line in `docs/decisions.md`.
- Don't use `/feature-dev`'s question phase; use its `code-architect` and `code-reviewer` agents directly.
- Verify your own work (check, clippy, tests, run it) before calling anything done.
- Stop only when the user alone can unblock: system packages, credentials, licensing, irreversible actions. Label the issue `blocked` and move to the next one.

## GitHub (`gh`, repo `tahuffman1s/FOSSPS`)

- Issues are the source of truth. Roadmap work carries the `roadmap` label and a milestone (M0–M4); `ROADMAP.md` is only an overview. New work found along the way becomes an issue, not a TODO.
- Every change goes through a PR from a branch named `<issue>-<slug>`, with `Closes #<issue>`. Never commit to `main` directly, never force-push.
- The owner has authorized Claude to open, review and merge its own PRs without asking. Review the diff with `code-reviewer`, post the outcome with `gh pr review --comment` (GitHub does not allow approving a PR from the same account), then squash-merge once CI is green. CI (Linux, Windows, macOS) is the final check.
- Ask for JSON and only the fields you need (`gh ... --json number,title`); never dump full issue or PR lists.
- Never publish releases, change repo settings or visibility, or delete branches other than your own merged ones without being asked.
- The website is `site/` (plain HTML, no build step), deployed to GitHub Pages on merge. Keep its claims in line with what actually works.

## Commands

```sh
cargo check -q --workspace --message-format=short
cargo clippy -q --workspace --all-targets --message-format=short -- -D warnings
cargo test -q --workspace
cargo fmt --all
```

A hook runs rustfmt on every edited `.rs` file; don't format by hand.

## Research first

Before designing anything a user touches (script API, CLI, editor, error text, docs, onboarding), search the web for how the best tools solve it and for newer ideas, then design. Use the `researcher` agent so the reading stays out of the main context. Note the sources that shaped the design in the commit or PR description.

## Working efficiently

- Navigate with the rust-analyzer LSP (definitions, references) instead of reading whole files.
- Look up crate APIs with context7 instead of guessing or reading vendored source.
- Send broad searches and web reading to subagents (`Explore`, `researcher`); keep only conclusions.
- Run `api-reviewer` on any new or changed script-facing API before finishing.
- After a feature lands: `code-simplifier`. Commits: `/commit`.
- Never read `target/` or `Cargo.lock`. Keep cargo output quiet (`-q`, short messages).
- Keep this file short. Put subsystem detail in `docs/` and link it here only when needed.

## Quality bar

- Clippy clean with `-D warnings`; workspace lints are in the root `Cargo.toml`.
- No `unwrap`/`expect`/panics on engine runtime paths; return errors with actionable messages.
- `unsafe` only at FFI and GPU boundaries, each block with a `// SAFETY:` comment.
- Simulation and netcode are deterministic and unit-tested. No per-frame heap allocation in hot loops.
- A performance claim needs a benchmark or a frame capture.
