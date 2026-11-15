// Included in front of every FidelityFX source file by build.rs, so the
// vendored files compile unchanged with GCC, Clang and MSVC and need no
// Vulkan library at link time.

#ifndef FOSSPS_FSR_COMPAT_H
#define FOSSPS_FSR_COMPAT_H

// The SDK calls these few Vulkan functions by name, which would need the
// Vulkan library when linking. FOSSPS finds Vulkan when the game starts, so
// the names are pointed at functions in shim.cpp that forward to it.
#define vkGetDeviceProcAddr fossps_fsr_vkGetDeviceProcAddr
#define vkEnumerateDeviceExtensionProperties fossps_fsr_vkEnumerateDeviceExtensionProperties
#define vkGetPhysicalDeviceMemoryProperties fossps_fsr_vkGetPhysicalDeviceMemoryProperties
#define vkGetPhysicalDeviceProperties fossps_fsr_vkGetPhysicalDeviceProperties
#define vkGetPhysicalDeviceProperties2 fossps_fsr_vkGetPhysicalDeviceProperties2
#define vkGetPhysicalDeviceFeatures fossps_fsr_vkGetPhysicalDeviceFeatures
#define vkGetPhysicalDeviceFeatures2 fossps_fsr_vkGetPhysicalDeviceFeatures2
#define vkCreateBuffer fossps_fsr_vkCreateBuffer

#if defined(__cplusplus) && !defined(_MSC_VER)

// Headers the SDK relies on MSVC to pull in by itself.
#include <cmath>
#include <codecvt>
#include <cstddef>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cwchar>
#include <locale>
#include <string>

// MSVC-only helpers the SDK uses, written with the standard library.

#define _countof(array) (sizeof(array) / sizeof((array)[0]))
#define sprintf_s snprintf
#define FFX_UNUSED(x)               ((void)(x))

inline void wcscpy_s(wchar_t* destination, size_t size, const wchar_t* source)
{
    if (size == 0)
        return;
    size_t index = 0;
    for (; index + 1 < size && source[index] != L'\0'; ++index)
        destination[index] = source[index];
    destination[index] = L'\0';
}

template <size_t Size>
inline void wcscpy_s(wchar_t (&destination)[Size], const wchar_t* source)
{
    wcscpy_s(destination, Size, source);
}

inline void strcpy_s(char* destination, size_t size, const char* source)
{
    if (size == 0)
        return;
    size_t index = 0;
    for (; index + 1 < size && source[index] != '\0'; ++index)
        destination[index] = source[index];
    destination[index] = '\0';
}

// Only used for debug names, which are plain ASCII.
inline int wcstombs_s(size_t* converted, char* destination, size_t size, const wchar_t* source, size_t count)
{
    size_t index = 0;
    if (size != 0)
    {
        for (; index + 1 < size && index < count && source[index] != L'\0'; ++index)
            destination[index] = (source[index] < 128) ? static_cast<char>(source[index]) : '?';
        destination[index] = '\0';
    }
    if (converted)
        *converted = index + 1;
    return 0;
}

#endif // defined(__cplusplus) && !defined(_MSC_VER)

#endif // FOSSPS_FSR_COMPAT_H
