//! AMD FidelityFX Super Resolution 3.1 upscaling, built from source.
//!
//! The game is drawn at a lower resolution and [`Upscaler`] turns each frame
//! into a full-size picture, using the frames before it to recover detail.
//!
//! AMD's own build only runs on Windows. This crate compiles the same C++ and
//! shaders (FidelityFX SDK 1.1.4, MIT, in `vendor/`) with plain `cargo build`
//! on Linux, Windows and macOS. See `docs/fsr.md` for how.
//!
//! What the upscaler expects from the renderer, fixed when it is built:
//!
//! - the picture is in high dynamic range, before tone mapping;
//! - depth is 1 at the camera and 0 infinitely far away;
//! - motion vectors are the size of the drawn picture, without the jitter.
//!
//! The Vulkan device needs the features in [`REQUIRED_FEATURES`].

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

mod error;
mod ffi;
#[cfg(test)]
mod tests;

use std::ffi::c_void;
use std::ptr::{self, NonNull};
use std::sync::Mutex;

use ash::vk;

pub use error::FsrError;

/// The Vulkan device features the upscaler's shaders use.
///
/// Switch these on when creating the device, on top of whatever else the
/// renderer needs. Every desktop graphics card with Vulkan 1.3 has them.
pub const REQUIRED_FEATURES: vk::PhysicalDeviceFeatures = vk::PhysicalDeviceFeatures {
    shader_storage_image_extended_formats: vk::TRUE,
    shader_storage_image_write_without_format: vk::TRUE,
    ..NO_FEATURES
};

/// Every feature switched off, to build [`REQUIRED_FEATURES`] from.
// SAFETY: the struct is 55 `u32`s, for which all zeroes means "off".
const NO_FEATURES: vk::PhysicalDeviceFeatures = unsafe { std::mem::zeroed() };

/// How much smaller than the window the game is drawn.
///
/// The names are AMD's, so players find the setting they know from other games.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Quality {
    /// Drawn at full size; the upscaler only smooths edges.
    NativeAa,
    /// 1.5 times smaller each way. The sharpest upscaled picture.
    Quality,
    /// 1.7 times smaller each way.
    Balanced,
    /// 2 times smaller each way: a quarter of the pixels.
    Performance,
    /// 3 times smaller each way: a ninth of the pixels. Visibly softer.
    UltraPerformance,
}

impl Quality {
    /// How many times smaller the drawn picture is than the window, each way.
    #[must_use]
    pub fn ratio(self) -> f32 {
        match self {
            Self::NativeAa => 1.0,
            Self::Quality => 1.5,
            Self::Balanced => 1.7,
            Self::Performance => 2.0,
            Self::UltraPerformance => 3.0,
        }
    }

    /// The size to draw the game at for a window of `output` pixels.
    ///
    /// Never smaller than one pixel either way.
    pub fn render_size(self, output: vk::Extent2D) -> vk::Extent2D {
        // Truncating matches AMD's `ffxFsr3UpscalerGetRenderResolutionFromQualityMode`.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let scale = |pixels: u32| ((f64::from(pixels) / f64::from(self.ratio())) as u32).max(1);
        vk::Extent2D {
            width: scale(output.width),
            height: scale(output.height),
        }
    }
}

/// How far to shift the camera this frame, in pixels of the drawn picture.
///
/// Shifting the camera by a different part of a pixel each frame is what lets
/// the upscaler see detail smaller than a pixel. `frame` counts frames up
/// from any number; the shifts repeat after a while. Both values are between
/// -0.5 and 0.5. Pass the same pair to [`Frame::jitter`].
///
/// To apply it, move the whole drawn picture `x` pixels to the right and
/// `y` pixels down: where the picture runs from -1 to 1 with y up, that is
/// `2.0 * x / render_width` across and `-2.0 * y / render_height` up, added
/// after the projection.
#[must_use]
pub fn jitter(frame: u32, render_width: u32, output_width: u32) -> (f32, f32) {
    let phases = jitter_phases(render_width, output_width);
    let index = i32::try_from(frame % phases.unsigned_abs()).unwrap_or(0);
    let (mut x, mut y) = (0.0, 0.0);
    // SAFETY: both pointers are to live floats, and `phases` is at least 1,
    // which is all the function asks for.
    unsafe { ffi::fossps_fsr_jitter_offset(&raw mut x, &raw mut y, index, phases) };
    (x, y)
}

/// How many different shifts [`jitter`] goes through before repeating.
///
/// AMD's rule, `ffxFsr3UpscalerGetJitterPhaseCount`: eight, times how many
/// output pixels each drawn pixel becomes. Kept between 1 and 1024 so that
/// sizes no game uses cannot overflow.
fn jitter_phases(render_width: u32, output_width: u32) -> i32 {
    let ratio = f64::from(output_width.max(1)) / f64::from(render_width.max(1));
    // Truncating is AMD's rounding, and the value is between 1 and 1024.
    #[expect(clippy::cast_possible_truncation)]
    let phases = (8.0 * ratio * ratio).clamp(1.0, 1024.0) as i32;
    phases
}

/// One Vulkan image handed to the upscaler, with how it was created.
#[derive(Debug, Clone, Copy)]
pub struct Image {
    /// The image.
    pub image: vk::Image,
    /// The format it was created with.
    pub format: vk::Format,
    /// The usage flags it was created with.
    pub usage: vk::ImageUsageFlags,
    /// Its size in pixels.
    pub extent: vk::Extent2D,
}

impl Image {
    fn raw(&self) -> ffi::Image {
        use vk::Handle;
        ffi::Image {
            image: self.image.as_raw(),
            // Vulkan's own numbers, which the C side reads back as such.
            format: self.format.as_raw().cast_unsigned(),
            usage: self.usage.as_raw(),
            width: self.extent.width,
            height: self.extent.height,
        }
    }
}

/// Everything the upscaler needs to turn one drawn frame into a full-size one.
#[derive(Debug, Clone, Copy)]
pub struct Frame {
    /// The drawn picture, in high dynamic range, before tone mapping.
    ///
    /// In `SHADER_READ_ONLY_OPTIMAL` layout, created with `SAMPLED` usage.
    pub color: Image,
    /// The depth buffer for [`color`](Self::color), the same size: 1 at the
    /// camera, 0 infinitely far away.
    ///
    /// In `SHADER_READ_ONLY_OPTIMAL` layout, created with `SAMPLED` usage.
    pub depth: Image,
    /// How far each pixel moved since the last frame, the same size as
    /// [`color`](Self::color): where it was minus where it is, not counting
    /// the jitter. See [`motion_scale`](Self::motion_scale) for the units.
    ///
    /// In `SHADER_READ_ONLY_OPTIMAL` layout, created with `SAMPLED` usage.
    pub motion: Image,
    /// Where the full-size picture is written.
    ///
    /// In `GENERAL` layout, created with `STORAGE` usage. It is left in
    /// `GENERAL` layout.
    pub output: Image,
    /// The camera shift used to draw this frame, from [`jitter`].
    pub jitter: (f32, f32),
    /// What to multiply [`motion`](Self::motion) by to get pixels of the
    /// drawn picture. For motion stored as a fraction of the screen, this is
    /// the drawn picture's width and height.
    pub motion_scale: (f32, f32),
    /// Extra sharpening from 0.0 (none) to 1.0 (most).
    pub sharpness: f32,
    /// How long the last frame took, in seconds.
    pub frame_seconds: f32,
    /// The distance to the camera's near plane, in metres.
    pub camera_near: f32,
    /// The camera's vertical field of view, in radians.
    pub camera_fov_y: f32,
    /// Set for the first frame after a cut, a respawn or a teleport, so the
    /// frames before it are forgotten instead of smeared into this one.
    pub reset: bool,
}

/// Held while an upscaler is created or destroyed: the SDK keeps the Vulkan
/// device in a global, which two threads must not change at once.
static SDK: Mutex<()> = Mutex::new(());

/// Upscales frames for one window.
///
/// Make a new one when the window or the [`Quality`] changes. Only one Vulkan
/// device can have upscalers at a time, which is all a game needs.
#[derive(Debug)]
pub struct Upscaler {
    raw: NonNull<ffi::Fsr>,
    max_render: vk::Extent2D,
    output: vk::Extent2D,
}

impl Upscaler {
    /// Makes an upscaler that turns pictures of up to `max_render` pixels
    /// into pictures of `output` pixels.
    ///
    /// This compiles the upscaler's shaders for the graphics card and
    /// allocates its images, so do it at startup or on a resize, not per frame.
    ///
    /// # Errors
    ///
    /// Fails if a size is zero or `max_render` is larger than `output`, if
    /// another device already has an upscaler, or if the graphics driver
    /// refuses, for example when it is out of memory.
    ///
    /// # Safety
    ///
    /// `device` must have been created from `physical_device` of `instance`
    /// with Vulkan 1.2 or newer and [`REQUIRED_FEATURES`] switched on, and
    /// must outlive the upscaler.
    ///
    /// The upscaler must not be dropped while the graphics card is still
    /// running commands recorded with [`upscale`](Self::upscale): they use
    /// its images and pipelines. Wait for the device to go idle first.
    pub unsafe fn new(
        instance: &ash::Instance,
        physical_device: vk::PhysicalDevice,
        device: &ash::Device,
        max_render: vk::Extent2D,
        output: vk::Extent2D,
    ) -> Result<Self, FsrError> {
        if max_render.width == 0 || max_render.height == 0 {
            return Err(FsrError::EmptySize);
        }
        if max_render.width > output.width || max_render.height > output.height {
            return Err(FsrError::RenderLargerThanOutput { max_render, output });
        }

        let v1_0 = instance.fp_v1_0();
        let v1_1 = instance.fp_v1_1();
        let loader = ffi::Loader {
            get_device_proc_addr: v1_0.get_device_proc_addr as *const c_void,
            enumerate_device_extension_properties: v1_0.enumerate_device_extension_properties
                as *const c_void,
            get_physical_device_memory_properties: v1_0.get_physical_device_memory_properties
                as *const c_void,
            get_physical_device_properties: v1_0.get_physical_device_properties as *const c_void,
            get_physical_device_properties2: v1_1.get_physical_device_properties2 as *const c_void,
            get_physical_device_features: v1_0.get_physical_device_features as *const c_void,
            get_physical_device_features2: v1_1.get_physical_device_features2 as *const c_void,
        };
        let create = ffi::Create {
            device: device.handle(),
            physical_device,
            max_render_width: max_render.width,
            max_render_height: max_render.height,
            max_output_width: output.width,
            max_output_height: output.height,
        };
        let mut raw = ptr::null_mut();
        // A poisoned lock only means another thread panicked while holding it;
        // the SDK's global is still consistent, as nothing in there unwinds.
        let _one_at_a_time = SDK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: the structs match `shim/shim.h`, the function addresses come
        // from the instance the caller vouches for, and `raw` is a live pointer.
        let code =
            unsafe { ffi::fossps_fsr_create(&raw const loader, &raw const create, &raw mut raw) };
        match NonNull::new(raw) {
            Some(raw) if code == 0 => Ok(Self {
                raw,
                max_render,
                output,
            }),
            _ => Err(FsrError::from_code("creating the upscaler", code)),
        }
    }

    /// The size of the pictures this upscaler writes.
    pub fn output(&self) -> vk::Extent2D {
        self.output
    }

    /// Records the commands that upscale one frame into `commands`.
    ///
    /// Nothing is drawn until `commands` is submitted. The compute pipeline
    /// and descriptor sets bound in `commands` are replaced.
    ///
    /// # Errors
    ///
    /// Fails if an image is missing, was created without the usage the
    /// upscaler needs or is not the size the upscaler was made for, or if the
    /// graphics driver reports a failure.
    ///
    /// # Safety
    ///
    /// `commands` must be recording, outside a render pass, on the device the
    /// upscaler was made with. The images in `frame` must be live, created as
    /// their fields say and in the layouts [`Frame`] documents, and must stay
    /// alive until the commands have finished running.
    pub unsafe fn upscale(
        &mut self,
        commands: vk::CommandBuffer,
        frame: &Frame,
    ) -> Result<(), FsrError> {
        let sampled = vk::ImageUsageFlags::SAMPLED;
        for (what, image, usage, needs) in [
            ("drawn picture", &frame.color, sampled, "SAMPLED"),
            ("depth buffer", &frame.depth, sampled, "SAMPLED"),
            ("motion vectors", &frame.motion, sampled, "SAMPLED"),
            (
                "output picture",
                &frame.output,
                vk::ImageUsageFlags::STORAGE,
                "STORAGE",
            ),
        ] {
            if image.image == vk::Image::null() {
                return Err(FsrError::MissingImage { what });
            }
            if !image.usage.contains(usage) {
                return Err(FsrError::WrongUsage { what, needs });
            }
        }
        let render = frame.color.extent;
        if render.width == 0
            || render.height == 0
            || render.width > self.max_render.width
            || render.height > self.max_render.height
        {
            return Err(FsrError::WrongSize {
                what: "drawn picture",
                size: render,
                expected: self.max_render,
            });
        }
        for (what, image) in [
            ("depth buffer", &frame.depth),
            ("motion vectors", &frame.motion),
        ] {
            if image.extent != render {
                return Err(FsrError::WrongSize {
                    what,
                    size: image.extent,
                    expected: render,
                });
            }
        }
        if frame.output.extent != self.output {
            return Err(FsrError::WrongSize {
                what: "output picture",
                size: frame.output.extent,
                expected: self.output,
            });
        }

        let dispatch = ffi::Dispatch {
            commands,
            color: frame.color.raw(),
            depth: frame.depth.raw(),
            motion: frame.motion.raw(),
            output: frame.output.raw(),
            jitter_x: frame.jitter.0,
            jitter_y: frame.jitter.1,
            motion_scale_x: frame.motion_scale.0,
            motion_scale_y: frame.motion_scale.1,
            sharpness: frame.sharpness.clamp(0.0, 1.0),
            frame_milliseconds: frame.frame_seconds * 1000.0,
            camera_near: frame.camera_near,
            camera_fov_y: frame.camera_fov_y,
            reset: u32::from(frame.reset),
        };
        // SAFETY: `self.raw` is live until drop, the struct matches
        // `shim/shim.h`, and the caller vouches for the handles in it.
        let code = unsafe { ffi::fossps_fsr_dispatch(self.raw.as_ptr(), &raw const dispatch) };
        if code == 0 {
            Ok(())
        } else {
            Err(FsrError::from_code("upscaling a frame", code))
        }
    }
}

impl Drop for Upscaler {
    /// Frees the upscaler's images and pipelines.
    ///
    /// The graphics card must have finished every frame upscaled with it;
    /// the renderer waits for the device to go idle first.
    fn drop(&mut self) {
        let _one_at_a_time = SDK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // SAFETY: `self.raw` came from `fossps_fsr_create` and is freed only here.
        unsafe { ffi::fossps_fsr_destroy(self.raw.as_ptr()) };
    }
}
