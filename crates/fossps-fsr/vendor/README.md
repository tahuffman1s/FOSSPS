# Vendored source

Third-party code that `fossps-fsr` compiles. Each keeps its own licence, which
is in its folder.

## FidelityFX-SDK

AMD FidelityFX SDK **v1.1.4** (FSR 3.1.4), commit
`c6efa6bf7f2027b3ec94f28578bb5965eabb9e55` of
<https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK>. MIT, see
`FidelityFX-SDK/LICENSE.txt`.

Only what the FSR 3.1 upscaler and its Vulkan backend need is here, at the
paths it has in the SDK's `sdk/` folder:

- `include/FidelityFX/host/`: the headers the upscaler and backend include
- `include/FidelityFX/gpu/`: the shared shader headers, plus `fsr1`, `spd`, `fsr3upscaler`
- `src/shared/`, `src/components/fsr3upscaler/`
- `src/backends/vk/ffx_vk.cpp`, `src/backends/vk/shaders/fsr3upscaler/`
- `src/backends/shared/ffx_shader_blobs.h`

Two files are changed; `fossps.patch` is the whole difference, and every
changed place is marked `FOSSPS:` in the source.

1. `include/FidelityFX/host/ffx_types.h`: the context size can be set from the
   build. The context stores names as `wchar_t`, which is 4 bytes outside
   Windows, so AMD's size is too small there.
2. `src/backends/vk/ffx_vk.cpp`, `findMemoryTypeIndex`: falls back to memory
   the CPU can see when the device has no other kind. Without it every
   allocation fails on software renderers such as lavapipe.

Everything else that AMD's Windows-only build does differently is handled
outside the vendored files, in `../shim/` and `../build.rs`.

To move to a newer SDK: copy the same files from the new tag, apply
`fossps.patch` from `FidelityFX-SDK/` (`patch -p1 < ../fossps.patch`), and run
the crate's tests with the Vulkan validation layer installed.

## Vulkan-Headers

Khronos Vulkan-Headers **vulkan-sdk-1.4.321.0**, commit
`2cd90f9d20df57eac214c148f3aed885372ddcfe` of
<https://github.com/KhronosGroup/Vulkan-Headers>. Apache-2.0 OR MIT, see
`Vulkan-Headers/LICENSE.md`. Unchanged.

Only the C headers the SDK includes (`vulkan/vulkan.h`, `vulkan_core.h`,
`vk_platform.h` and `vk_video/`), so building needs no Vulkan SDK.
