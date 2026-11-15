# FSR 3.1 upscaling (`fossps-fsr`)

`fossps-fsr` builds AMD's FSR 3.1 upscaler from source and gives the renderer
a small Rust type, `Upscaler`, to run it. It works on Linux, Windows and
macOS with plain `cargo build`: no CMake, Vulkan SDK or shader compiler to
install.

The renderer uses it by default: see "In the renderer" below.

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
| A struct is declared 32-byte aligned but placed at any 4-byte offset, which crashes an optimised GCC build | patch: the alignment is dropped |
| The GLSL declares the luminance history image as 8-bit while the SDK creates it as 16-bit floats, which Vulkan leaves undefined | `build.rs` corrects the declaration as it reads the shader |
| When creating a context fails half-way it frees nothing | the shim notes when the backend was created and, on failure, asks the SDK to take down whatever exists |

The vendored files and the three patches are listed in
`crates/fossps-fsr/vendor/README.md`.

## Choices made once, at build time

The SDK compiles every shader for every combination of options. FOSSPS has one
renderer, so `build.rs` compiles only what it uses:

- the picture is high dynamic range (before tone mapping);
- depth is 1 at the camera and 0 infinitely far away;
- motion vectors are the size of the drawn picture, without the jitter;
- exposure is worked out by FSR itself.

The last frames are resampled with the Lanczos curve worked out in the shader.
AMD's other variant reads the curve from a small texture and is picked on cards
with 32- to 64-wide subgroups (AMD's own). Built this way it made a still
picture drift up and left and blur on an RX 7800 XT with RADV, so it is not
built (#191 looks into why).

Half-precision and 64-wide shader variants are not built; the plain ones run on
every card. The two pyramid passes are compiled without subgroup operations
(`FFX_SPD_NO_WAVE_OPERATIONS`), which Vulkan does not promise in compute shaders.

## What the device needs

Vulkan 1.2 or newer, and the two features in `fossps_fsr::REQUIRED_FEATURES`:
`shaderStorageImageExtendedFormats` and `shaderStorageImageWriteWithoutFormat`.

## In the renderer

`fossps_render::Upscaling` picks how much smaller than the window the scene is
drawn: `Off`, `Native` (full size, edges smoothed), `Quality` (1.5 times
smaller each way, the default), `Balanced` (1.7), `Performance` (2) and
`UltraPerformance` (3). It is set in `RenderConfig` (`fossps::Config` for a
game) and changed while running with `Renderer::set_upscaling`.

Each frame:

1. The camera is moved by part of a pixel (`fossps_fsr::jitter`), a different
   part each frame. The picture moves `x` pixels right and `y` pixels down.
2. The mesh pass draws colour, depth and motion at the smaller size. Motion is
   where a pixel was last frame minus where it is now, without the camera's
   part-of-a-pixel move, as a fraction of the picture (x right, y down), in
   an `R16G16_SFLOAT` image. The background is written as not moving.
3. FSR writes the full-size picture, still in high dynamic range, with a
   sharpness of 0.3.
4. Tone mapping reads that picture instead of the drawn one.

After a jump of the view (`Renderer::cut_to`, a new scene, a resized window)
FSR is told to forget the frames before. A game run through `fossps::run`
gets this by itself when the view moves more than 5 m in one frame.

If the graphics card lacks the two features below, or FSR cannot start, a
warning is logged and the game is drawn at full size without it.

Not done yet: textures are not drawn, so there is no texture sharpening for
upscaling (mip bias, #141); nothing transparent or animated is drawn, so
FSR's reactive mask is not written; the frame rate does not yet pick the
size (#139).

## Limits

- One Vulkan device can have upscalers at a time (the SDK keeps the device in a global).
- On Windows only MSVC (or clang-cl) is supported, not MinGW.
- macOS (MoltenVK) compiles and links in CI but has not been run.

## Testing

`cargo test -p fossps-render` draws a tilted square at a quarter of the pixels,
upscales it over 48 frames, still and with the camera sliding, and checks the
result is closer to the same scene drawn four times too large and averaged
down than plain stretching is. It fails if the camera's part-of-a-pixel move
or the motion goes the wrong way. Another test reads the motion image back
and checks the numbers.

`cargo test -p fossps-fsr` upscales a 64x48 picture to 128x96 on whatever
Vulkan 1.3 device is there and checks the colours, with the validation layer
on if it is installed. CI runs it on lavapipe. It has also been run on an AMD
Radeon RX 7800 XT (RADV).
