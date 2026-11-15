//! The C interface in `shim/shim.h`, declared for Rust.
//!
//! Every struct here mirrors one in that header; change both together.

use std::ffi::c_void;

use ash::vk;

/// Returned by [`fossps_fsr_create`] when an upscaler exists on another device.
pub(crate) const ERROR_SECOND_DEVICE: i32 = 0x7F55_0001;

/// `FosspsFsrLoader`: Vulkan functions, as plain addresses.
#[repr(C)]
pub(crate) struct Loader {
    pub get_device_proc_addr: *const c_void,
    pub enumerate_device_extension_properties: *const c_void,
    pub get_physical_device_memory_properties: *const c_void,
    pub get_physical_device_properties: *const c_void,
    pub get_physical_device_properties2: *const c_void,
    pub get_physical_device_features: *const c_void,
    pub get_physical_device_features2: *const c_void,
}

/// `FosspsFsrCreate`.
#[repr(C)]
pub(crate) struct Create {
    pub device: vk::Device,
    pub physical_device: vk::PhysicalDevice,
    pub max_render_width: u32,
    pub max_render_height: u32,
    pub max_output_width: u32,
    pub max_output_height: u32,
}

/// `FosspsFsrImage`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Image {
    pub image: u64,
    pub format: u32,
    pub usage: u32,
    pub width: u32,
    pub height: u32,
}

/// `FosspsFsrDispatch`.
#[repr(C)]
pub(crate) struct Dispatch {
    pub commands: vk::CommandBuffer,
    pub color: Image,
    pub depth: Image,
    pub motion: Image,
    pub output: Image,
    pub jitter_x: f32,
    pub jitter_y: f32,
    pub motion_scale_x: f32,
    pub motion_scale_y: f32,
    pub sharpness: f32,
    pub frame_milliseconds: f32,
    pub camera_near: f32,
    pub camera_fov_y: f32,
    pub reset: u32,
}

/// `FosspsFsr`: one upscaler, only ever handled through a pointer.
#[repr(C)]
pub(crate) struct Fsr {
    _private: [u8; 0],
}

unsafe extern "C" {
    pub(crate) fn fossps_fsr_create(
        loader: *const Loader,
        create: *const Create,
        out: *mut *mut Fsr,
    ) -> i32;
    pub(crate) fn fossps_fsr_destroy(fsr: *mut Fsr);
    pub(crate) fn fossps_fsr_dispatch(fsr: *mut Fsr, frame: *const Dispatch) -> i32;
    pub(crate) fn fossps_fsr_jitter_offset(
        x: *mut f32,
        y: *mut f32,
        index: i32,
        phase_count: i32,
    ) -> i32;
}
