# FSR 3.1 upscaling (`fossps-fsr`)

`fossps-fsr` builds AMD's FSR 3.1 upscaler from source and gives the renderer
a small Rust type, `Upscaler`, to run it. It works on Linux, Windows and
macOS with plain `cargo build`: no CMake, Vulkan SDK or shader compiler to
install.

The renderer does not use it yet. That is #16, which needs motion vectors
first (#184). The depth buffer it reads is there (#58).

## Why it is not AMD's build

AMD's FidelityFX SDK 1.1.4 only builds on Windows. Checked on Fedora with GCC 16:

| What stops it | What this crate does |
| --- | --- |
| Shaders are compiled by `FidelityFX_SC.exe`, a Windows program | `build.rs` compiles the same GLSL with the `glslang` crate and writes the table of shaders the SDK looks them up in |
| That tool also lists what each shader reads and writes, using SPIRV-Reflect after an optimiser pass | `build.rs` reads the same lists from the SPIR-V, keeping only variables a shader uses |
| The C++ uses MSVC-only calls (`wcscpy_s`, `_countof`, `sprintf_s`, …) and leaves out standard headers | `shim/compat.h`, put in front of every file by the build, supplies them |
| The context struct is too small where `wchar_t` is 4 bytes | one-line patch, and the size is set from the build |
| A few Vulkan functions are called by name, which needs the Vulkan library at link time | `compat.h` renames them to functions in `shim/shim.cpp` that forward to the Vulkan the game loaded |
| Some functions are looked up under an extension's name (`…KHR`), assuming the game enabled the extension | the shim tries the plain Vulkan 1.1 name when that finds nothing |
| It allocates from the last memory type that fits, which on AMD cards needs a feature the game has not enabled | the shim hides those memory types from the SDK |
| It refuses memory the CPU can see, the only kind a software renderer has | patch to `findMemoryTypeIndex` |
| When creating a context fails half-way it frees nothing | the shim notes when the backend was created and, on failure, asks the SDK to take down whatever exists |

The vendored files and the two patches are listed in
`crates/fossps-fsr/vendor/README.md`.

## Choices made once, at build time

The SDK compiles every shader for every combination of options. FOSSPS has one
renderer, so `build.rs` compiles only what it uses:

- the picture is high dynamic range (before tone mapping);
- depth is 1 at the camera and 0 infinitely far away;
- motion vectors are the size of the drawn picture, without the jitter;
- exposure is worked out by FSR itself.

Half-precision and 64-wide shader variants are not built; the plain ones run on
every card. The two pyramid passes are compiled without subgroup operations
(`FFX_SPD_NO_WAVE_OPERATIONS`), which Vulkan does not promise in compute shaders.

## What the device needs

Vulkan 1.2 or newer, and the two features in `fossps_fsr::REQUIRED_FEATURES`:
`shaderStorageImageExtendedFormats` and `shaderStorageImageWriteWithoutFormat`.

## Limits

- One Vulkan device can have upscalers at a time (the SDK keeps the device in a global).
- On Windows only MSVC (or clang-cl) is supported, not MinGW.
- macOS (MoltenVK) compiles and links in CI but has not been run.

## Testing

`cargo test -p fossps-fsr` upscales a 64x48 picture to 128x96 on whatever
Vulkan 1.3 device is there and checks the colours, with the validation layer
on if it is installed. CI runs it on lavapipe. It has also been run on an AMD
Radeon RX 7800 XT (RADV).
