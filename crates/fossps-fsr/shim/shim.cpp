// A small C interface over the FidelityFX FSR 3.1 upscaler, so the Rust side
// declares a handful of plain structs instead of mirroring the SDK's headers.
//
// The struct layouts and function signatures here must match `src/ffi.rs`.

#include <FidelityFX/host/backends/vk/ffx_vk.h>
#include <FidelityFX/host/ffx_fsr3upscaler.h>
#include <ffx_shader_blobs.h>

#include <cfloat>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <new>

#include "shim.h"

// ---------------------------------------------------------------------------
// Vulkan functions the SDK calls by name (see compat.h). They forward to the
// Vulkan library the game loaded.

static FosspsFsrLoader g_loader = {};

extern "C" {

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetDeviceProcAddr(VkDevice device, const char* name)
{
    return g_loader.getDeviceProcAddr(device, name);
}

VKAPI_ATTR VkResult VKAPI_CALL vkEnumerateDeviceExtensionProperties(VkPhysicalDevice physicalDevice,
                                                                    const char*           layerName,
                                                                    uint32_t*             count,
                                                                    VkExtensionProperties* properties)
{
    return g_loader.enumerateDeviceExtensionProperties(physicalDevice, layerName, count, properties);
}

// The SDK takes the last kind of memory that fits. On AMD cards that is one
// of the "coherent" kinds, which may only be used with a device feature the
// game has not switched on. Those kinds are shown to the SDK as unusable, so
// it takes ordinary memory, with nothing asked of the renderer.
VKAPI_ATTR void VKAPI_CALL vkGetPhysicalDeviceMemoryProperties(VkPhysicalDevice physicalDevice, VkPhysicalDeviceMemoryProperties* properties)
{
    g_loader.getPhysicalDeviceMemoryProperties(physicalDevice, properties);
    const VkMemoryPropertyFlags special = VK_MEMORY_PROPERTY_DEVICE_COHERENT_BIT_AMD | VK_MEMORY_PROPERTY_DEVICE_UNCACHED_BIT_AMD;
    for (uint32_t index = 0; index < properties->memoryTypeCount; ++index)
    {
        if (properties->memoryTypes[index].propertyFlags & special)
            properties->memoryTypes[index].propertyFlags = 0;
    }
}

VKAPI_ATTR void VKAPI_CALL vkGetPhysicalDeviceProperties(VkPhysicalDevice physicalDevice, VkPhysicalDeviceProperties* properties)
{
    g_loader.getPhysicalDeviceProperties(physicalDevice, properties);
}

VKAPI_ATTR void VKAPI_CALL vkGetPhysicalDeviceProperties2(VkPhysicalDevice physicalDevice, VkPhysicalDeviceProperties2* properties)
{
    g_loader.getPhysicalDeviceProperties2(physicalDevice, properties);
}

VKAPI_ATTR void VKAPI_CALL vkGetPhysicalDeviceFeatures(VkPhysicalDevice physicalDevice, VkPhysicalDeviceFeatures* features)
{
    g_loader.getPhysicalDeviceFeatures(physicalDevice, features);
}

VKAPI_ATTR void VKAPI_CALL vkGetPhysicalDeviceFeatures2(VkPhysicalDevice physicalDevice, VkPhysicalDeviceFeatures2* features)
{
    g_loader.getPhysicalDeviceFeatures2(physicalDevice, features);
}

// The SDK creates one throwaway buffer by name to test a memory flag.
VKAPI_ATTR VkResult VKAPI_CALL vkCreateBuffer(VkDevice device, const VkBufferCreateInfo* info, const VkAllocationCallbacks* allocator, VkBuffer* buffer)
{
    PFN_vkCreateBuffer create = reinterpret_cast<PFN_vkCreateBuffer>(g_loader.getDeviceProcAddr(device, "vkCreateBuffer"));
    return create ? create(device, info, allocator, buffer) : VK_ERROR_INITIALIZATION_FAILED;
}

} // extern "C"

// Looks up a device function for the SDK.
//
// The SDK asks for some functions under the name of the extension they came
// from, and assumes the game switched that extension on whenever the card has
// it. Those functions have been part of Vulkan itself since 1.1, so when the
// extension's name finds nothing the plain name is tried.
static VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL lookUp(VkDevice device, const char* name)
{
    PFN_vkVoidFunction found = g_loader.getDeviceProcAddr(device, name);
    const size_t       length = strlen(name);
    if (!found && length > 3 && length < 96 && strcmp(name + length - 3, "KHR") == 0)
    {
        char plain[96];
        memcpy(plain, name, length - 3);
        plain[length - 3] = '\0';
        found = g_loader.getDeviceProcAddr(device, plain);
    }
    return found;
}

// Frame generation is not built; the backend only stores this function's address.
FfxErrorCode ffxSetFrameGenerationConfigToSwapchainVK(FfxFrameGenerationConfig const*)
{
    return FFX_ERROR_INVALID_ARGUMENT;
}

// ---------------------------------------------------------------------------
// One upscaler.

struct FosspsFsr
{
    FfxInterface           backend;
    void*                  scratch;
    // Which parts exist, so a half-made upscaler is taken apart correctly.
    bool                   backendCreated;
    bool                   hasDilatedDepth;
    bool                   hasDilatedMotionVectors;
    bool                   hasReconstructedDepth;
    FfxResourceInternal    dilatedDepth;
    FfxResourceInternal    dilatedMotionVectors;
    FfxResourceInternal    reconstructedDepth;
    FfxFsr3UpscalerContext context;
};

// The SDK keeps the device in one global, so a second upscaler on another
// device would silently draw on the first one's.
static VkDevice g_device       = VK_NULL_HANDLE;
static uint32_t g_upscalers    = 0;

// The upscaler being created, and the backend's own function for the step
// below. The Rust side lets only one thread create or destroy at a time.
static FosspsFsr*                  g_creating          = nullptr;
static FfxCreateBackendContextFunc g_createBackendReal = nullptr;

// Stands in for the backend's "create" step to note that it happened. When
// creating the upscaler fails after this point the SDK returns without
// freeing anything, and only then is it right to ask it to clean up.
static FfxErrorCode createBackend(FfxInterface* backend, FfxEffect effect, FfxEffectBindlessConfig* bindless, FfxUInt32* effectContextId)
{
    const FfxErrorCode code = g_createBackendReal(backend, effect, bindless, effectContextId);
    if (code == FFX_OK && g_creating)
        g_creating->backendCreated = true;
    return code;
}

// Frees an upscaler, however much of it was made.
//
// The upscaler is the backend's only user (`maxContexts` is 1 below), so its
// resources belong to effect context 0.
static void release(FosspsFsr* fsr)
{
    if (fsr->backendCreated)
    {
        if (fsr->hasDilatedDepth)
            fsr->backend.fpDestroyResource(&fsr->backend, fsr->dilatedDepth, 0);
        if (fsr->hasDilatedMotionVectors)
            fsr->backend.fpDestroyResource(&fsr->backend, fsr->dilatedMotionVectors, 0);
        if (fsr->hasReconstructedDepth)
            fsr->backend.fpDestroyResource(&fsr->backend, fsr->reconstructedDepth, 0);
        // Releases whatever pipelines and images exist, then the backend.
        ffxFsr3UpscalerContextDestroy(&fsr->context);
    }
    free(fsr->scratch);
    free(fsr);
}

static FfxResource wrapImage(const FosspsFsrImage& image, FfxResourceStates state)
{
    if (image.image == 0)
    {
        FfxResource none = {};
        return none;
    }
    VkImageCreateInfo info = {};
    info.sType         = VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO;
    info.imageType     = VK_IMAGE_TYPE_2D;
    info.format        = static_cast<VkFormat>(image.format);
    info.extent.width  = image.width;
    info.extent.height = image.height;
    info.extent.depth  = 1;
    info.mipLevels     = 1;
    info.arrayLayers   = 1;
    info.samples       = VK_SAMPLE_COUNT_1_BIT;
    info.usage         = static_cast<VkImageUsageFlags>(image.usage);
    VkImage handle     = reinterpret_cast<VkImage>(static_cast<uintptr_t>(image.image));
    return ffxGetResourceVK(handle, ffxGetImageResourceDescriptionVK(handle, info), nullptr, state);
}

extern "C" {

int32_t fossps_fsr_create(const FosspsFsrLoader* loader, const FosspsFsrCreate* create, FosspsFsr** out)
{
    if (!loader || !create || !out)
        return FFX_ERROR_INVALID_POINTER;
    *out = nullptr;
    if (g_upscalers != 0 && g_device != create->device)
        return FOSSPS_FSR_ERROR_SECOND_DEVICE;
    g_loader = *loader;

    FosspsFsr* fsr = static_cast<FosspsFsr*>(calloc(1, sizeof(FosspsFsr)));
    if (!fsr)
        return FFX_ERROR_OUT_OF_MEMORY;

    VkDeviceContext deviceContext    = {};
    deviceContext.vkDevice           = create->device;
    deviceContext.vkPhysicalDevice   = create->physicalDevice;
    deviceContext.vkDeviceProcAddr   = lookUp;
    const size_t scratchSize         = ffxGetScratchMemorySizeVK(create->physicalDevice, 1);
    fsr->scratch                     = calloc(1, scratchSize);
    if (!fsr->scratch)
    {
        free(fsr);
        return FFX_ERROR_OUT_OF_MEMORY;
    }

    FfxErrorCode code = ffxGetInterfaceVK(&fsr->backend, ffxGetDeviceVK(&deviceContext), fsr->scratch, scratchSize, 1);
    if (code == FFX_OK)
    {
        g_createBackendReal               = fsr->backend.fpCreateBackendContext;
        fsr->backend.fpCreateBackendContext = createBackend;

        FfxFsr3UpscalerContextDescription description = {};
        description.flags = FFX_FSR3UPSCALER_ENABLE_HIGH_DYNAMIC_RANGE | FFX_FSR3UPSCALER_ENABLE_DEPTH_INVERTED |
                            FFX_FSR3UPSCALER_ENABLE_DEPTH_INFINITE | FFX_FSR3UPSCALER_ENABLE_AUTO_EXPOSURE;
        description.maxRenderSize.width   = create->maxRenderWidth;
        description.maxRenderSize.height  = create->maxRenderHeight;
        description.maxUpscaleSize.width  = create->maxOutputWidth;
        description.maxUpscaleSize.height = create->maxOutputHeight;
        description.backendInterface      = fsr->backend;
        g_creating = fsr;
        code       = ffxFsr3UpscalerContextCreate(&fsr->context, &description);
        g_creating = nullptr;
    }
    if (code == FFX_OK)
    {
        FfxFsr3UpscalerSharedResourceDescriptions shared = {};
        code = ffxFsr3UpscalerGetSharedResourceDescriptions(&fsr->context, &shared);
        if (code == FFX_OK)
        {
            code                 = fsr->backend.fpCreateResource(&fsr->backend, &shared.dilatedDepth, 0, &fsr->dilatedDepth);
            fsr->hasDilatedDepth = (code == FFX_OK);
        }
        if (code == FFX_OK)
        {
            code = fsr->backend.fpCreateResource(&fsr->backend, &shared.dilatedMotionVectors, 0, &fsr->dilatedMotionVectors);
            fsr->hasDilatedMotionVectors = (code == FFX_OK);
        }
        if (code == FFX_OK)
        {
            code = fsr->backend.fpCreateResource(&fsr->backend, &shared.reconstructedPrevNearestDepth, 0, &fsr->reconstructedDepth);
            fsr->hasReconstructedDepth = (code == FFX_OK);
        }
    }
    if (code != FFX_OK)
    {
        release(fsr);
        return code;
    }

    g_device = create->device;
    ++g_upscalers;
    *out = fsr;
    return FFX_OK;
}

void fossps_fsr_destroy(FosspsFsr* fsr)
{
    if (!fsr)
        return;
    release(fsr);
    if (g_upscalers != 0)
        --g_upscalers;
}

int32_t fossps_fsr_dispatch(FosspsFsr* fsr, const FosspsFsrDispatch* frame)
{
    if (!fsr || !frame)
        return FFX_ERROR_INVALID_POINTER;

    FfxFsr3UpscalerDispatchDescription dispatch = {};
    dispatch.commandList   = ffxGetCommandListVK(frame->commands);
    dispatch.color         = wrapImage(frame->color, FFX_RESOURCE_STATE_COMPUTE_READ);
    dispatch.depth         = wrapImage(frame->depth, FFX_RESOURCE_STATE_COMPUTE_READ);
    dispatch.motionVectors = wrapImage(frame->motion, FFX_RESOURCE_STATE_COMPUTE_READ);
    dispatch.output        = wrapImage(frame->output, FFX_RESOURCE_STATE_UNORDERED_ACCESS);
    dispatch.dilatedDepth  = fsr->backend.fpGetResource(&fsr->backend, fsr->dilatedDepth);
    dispatch.dilatedMotionVectors          = fsr->backend.fpGetResource(&fsr->backend, fsr->dilatedMotionVectors);
    dispatch.reconstructedPrevNearestDepth = fsr->backend.fpGetResource(&fsr->backend, fsr->reconstructedDepth);
    dispatch.jitterOffset.x        = frame->jitterX;
    dispatch.jitterOffset.y        = frame->jitterY;
    dispatch.motionVectorScale.x   = frame->motionScaleX;
    dispatch.motionVectorScale.y   = frame->motionScaleY;
    dispatch.renderSize.width      = frame->color.width;
    dispatch.renderSize.height     = frame->color.height;
    dispatch.upscaleSize.width     = frame->output.width;
    dispatch.upscaleSize.height    = frame->output.height;
    dispatch.enableSharpening      = frame->sharpness > 0.0f;
    dispatch.sharpness             = frame->sharpness;
    dispatch.frameTimeDelta        = frame->frameMilliseconds;
    dispatch.preExposure           = 1.0f;
    dispatch.reset                 = frame->reset != 0;
    // Depth runs from 1 at the near plane to 0 infinitely far away.
    dispatch.cameraNear              = frame->cameraNear;
    dispatch.cameraFar               = FLT_MAX;
    dispatch.cameraFovAngleVertical  = frame->cameraFovY;
    dispatch.viewSpaceToMetersFactor = 1.0f;
    return ffxFsr3UpscalerContextDispatch(&fsr->context, &dispatch);
}

int32_t fossps_fsr_jitter_offset(float* x, float* y, int32_t index, int32_t phaseCount)
{
    return ffxFsr3UpscalerGetJitterOffset(x, y, index, phaseCount);
}

} // extern "C"
