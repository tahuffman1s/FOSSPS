// Hands the SDK its compiled shaders.
//
// AMD's build makes this table with a Windows-only tool. Here build.rs
// compiles the same GLSL with glslang and writes the table into
// `fossps_fsr_blobs.inc`.

#include <FidelityFX/host/ffx_fsr3upscaler.h>
#include <FidelityFX/host/ffx_util.h>
#include <ffx_shader_blobs.h>
#include <fsr3upscaler/ffx_fsr3upscaler_private.h>

#include <cstdint>
#include <cstring>

// One compiled shader for one pass and one choice of options.
struct FosspsFsrShader
{
    FfxFsr3UpscalerPass  pass;
    bool                 sharpen;
    const FfxShaderBlob* blob;
};

// Defines `static const FosspsFsrShader g_shaders[]`.
#include "fossps_fsr_blobs.inc"

// The options build.rs compiles every shader with. They match the flags
// shim.cpp creates the upscaler with.
static const uint32_t FIXED_OPTIONS = FSR3UPSCALER_SHADER_PERMUTATION_HDR_COLOR_INPUT | FSR3UPSCALER_SHADER_PERMUTATION_LOW_RES_MOTION_VECTORS |
                                      FSR3UPSCALER_SHADER_PERMUTATION_JITTER_MOTION_VECTORS | FSR3UPSCALER_SHADER_PERMUTATION_DEPTH_INVERTED;
static const uint32_t FIXED_VALUES  = FSR3UPSCALER_SHADER_PERMUTATION_HDR_COLOR_INPUT | FSR3UPSCALER_SHADER_PERMUTATION_LOW_RES_MOTION_VECTORS |
                                      FSR3UPSCALER_SHADER_PERMUTATION_DEPTH_INVERTED;

FfxErrorCode ffxGetPermutationBlobByIndex(FfxEffect effectId, FfxPass passId, FfxBindStage, uint32_t permutationOptions, FfxShaderBlob* outBlob)
{
    if (effectId != FFX_EFFECT_FSR3UPSCALER || (permutationOptions & FIXED_OPTIONS) != FIXED_VALUES)
        return FFX_ERROR_INVALID_ARGUMENT;

    // The half-precision, 64-wide and Lanczos-lookup variants are not built.
    // The plain shaders run on every card, so asking for those gets the
    // plain ones.
    const bool sharpen       = FFX_CONTAINS_FLAG(permutationOptions, FSR3UPSCALER_SHADER_PERMUTATION_ENABLE_SHARPENING);
    for (size_t index = 0; index < sizeof(g_shaders) / sizeof(g_shaders[0]); ++index)
    {
        const FosspsFsrShader& shader = g_shaders[index];
        if (shader.pass == static_cast<FfxFsr3UpscalerPass>(passId) && shader.sharpen == sharpen)
        {
            memcpy(outBlob, shader.blob, sizeof(FfxShaderBlob));
            return FFX_OK;
        }
    }
    return FFX_ERROR_INVALID_ENUM;
}

FfxErrorCode ffxIsWave64(FfxEffect, uint32_t, bool& isWave64)
{
    isWave64 = false;
    return FFX_OK;
}
