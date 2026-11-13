# FOSSPS — FOSS Person Shooter

A free, open-source engine for multiplayer first-person shooters.

[Website](https://tahuffman1s.github.io/FOSSPS/) · [Roadmap](ROADMAP.md) · [Issues](https://github.com/tahuffman1s/FOSSPS/issues)

[![CI](https://github.com/tahuffman1s/FOSSPS/actions/workflows/ci.yml/badge.svg)](https://github.com/tahuffman1s/FOSSPS/actions/workflows/ci.yml)

**Status:** just started. Nothing is playable yet.

## Goals

- **Easy to use** — go from install to a playable networked match in minutes, with a Luau scripting API that reads like plain English.
- **Fast** — high, stable frame rates and low input latency; cheap dedicated servers.
- **Focused** — built only for multiplayer FPS games.
- **Modern** — Vulkan 1.3 renderer, FSR 3.1 upscaling, first-class controller support.

Planned platforms: Linux (AppImage), Windows, and macOS as a stretch goal.

## Building

Requires stable [Rust](https://rustup.rs) and a C toolchain for linking (`gcc` or `clang`).

```sh
cargo build
cargo test
```

## Contributing

Work is tracked in [issues](https://github.com/tahuffman1s/FOSSPS/issues) grouped by milestone. Changes land through pull requests that pass CI on Linux, Windows and macOS.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
Contributions are accepted under the same dual license.
