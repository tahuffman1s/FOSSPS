// The C interface between `src/ffi.rs` and the FidelityFX SDK.
//
// Every struct here has a `#[repr(C)]` twin in `src/ffi.rs`; change both together.

#ifndef FOSSPS_FSR_SHIM_H
#define FOSSPS_FSR_SHIM_H

#include <stdint.h>
#include <vulkan/vulkan.h>

#ifdef __cplusplus
extern "C" {
#endif

// Returned by fossps_fsr_create when an upscaler already exists on another device.
#define FOSSPS_FSR_ERROR_SECOND_DEVICE ((int32_t)0x7F550001)

// Vulkan functions looked up by the game, which the SDK would otherwise need
// the Vulkan library for at link time.
typedef struct FosspsFsrLoader
{
    PFN_vkGetDeviceProcAddr                  getDeviceProcAddr;
    PFN_vkEnumerateDeviceExtensionProperties enumerateDeviceExtensionProperties;
    PFN_vkGetPhysicalDeviceMemoryProperties  getPhysicalDeviceMemoryProperties;
    PFN_vkGetPhysicalDeviceProperties        getPhysicalDeviceProperties;
    PFN_vkGetPhysicalDeviceProperties2       getPhysicalDeviceProperties2;
    PFN_vkGetPhysicalDeviceFeatures          getPhysicalDeviceFeatures;
    PFN_vkGetPhysicalDeviceFeatures2         getPhysicalDeviceFeatures2;
} FosspsFsrLoader;

typedef struct FosspsFsrCreate
{
    VkDevice         device;
    VkPhysicalDevice physicalDevice;
    uint32_t         maxRenderWidth;
    uint32_t         maxRenderHeight;
    uint32_t         maxOutputWidth;
    uint32_t         maxOutputHeight;
} FosspsFsrCreate;

// One image handed to the upscaler. `image` is a VkImage; 0 means "none".
typedef struct FosspsFsrImage
{
    uint64_t image;
    uint32_t format; // VkFormat
    uint32_t usage;  // VkImageUsageFlags the image was created with
    uint32_t width;
    uint32_t height;
} FosspsFsrImage;

typedef struct FosspsFsrDispatch
{
    VkCommandBuffer commands;
    FosspsFsrImage  color;
    FosspsFsrImage  depth;
    FosspsFsrImage  motion;
    FosspsFsrImage  output;
    float           jitterX;
    float           jitterY;
    float           motionScaleX;
    float           motionScaleY;
    float           sharpness; // 0 switches sharpening off
    float           frameMilliseconds;
    float           cameraNear;
    float           cameraFovY; // radians
    uint32_t        reset;
} FosspsFsrDispatch;

typedef struct FosspsFsr FosspsFsr;

int32_t fossps_fsr_create(const FosspsFsrLoader* loader, const FosspsFsrCreate* create, FosspsFsr** out);
void    fossps_fsr_destroy(FosspsFsr* fsr);
int32_t fossps_fsr_dispatch(FosspsFsr* fsr, const FosspsFsrDispatch* frame);
int32_t fossps_fsr_jitter_offset(float* x, float* y, int32_t index, int32_t phaseCount);

#ifdef __cplusplus
}
#endif

#endif // FOSSPS_FSR_SHIM_H
